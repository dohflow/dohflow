//! The rebase engine core (personal-cfo-bxsbz.4): the scratch vault, rolling back to the base,
//! applying the server's envelopes with their write-set hashes verified, and the fork. Fake data
//! only.

use sync_core::envelope::{
    check_envelope, ArtifactManifest, EnvelopePlaintext, IdempotencyKey, ImmutableMeta,
};
use sync_core::header::{
    ClearHeader, Fingerprint, IdempotencyKeyTag, NonceDomain, ObjectKind, OBJECT_ENVELOPE,
};
use sync_core::status::{HaltEvidence, SyncEvent, SyncStatus};

use super::*;
use crate::base_image::BaseImageStatus;
use crate::rebase::{ForkEvidence, PulledEnvelope, RebaseError, Scratch};

const FINGERPRINT: [u8; 32] = [7; 32];
const ENGINE: &str = "dohflow-engine-2";

/// What stands in for the vault-crypto keyed tag: any fixed function of the key.
fn tag_for(key: &IdempotencyKey) -> IdempotencyKeyTag {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(key.as_bytes());
    bytes[16..].copy_from_slice(key.as_bytes());
    IdempotencyKeyTag::from_derived(bytes)
}

/// A sealed envelope as the server hands it back: the header and the decrypted plaintext.
struct Sealed {
    header: ClearHeader,
    plaintext: EnvelopePlaintext,
}

impl Sealed {
    fn pulled(&self, seq: u64) -> PulledEnvelope<'_> {
        PulledEnvelope {
            seq,
            checked: check_envelope(&self.header, &self.plaintext, tag_for)
                .expect("the fixture envelope passes the receiver checks"),
        }
    }
}

fn actor_byte(actor: ActorType) -> u8 {
    match actor {
        ActorType::User => 1,
        ActorType::System => 2,
        ActorType::Importer => 3,
        ActorType::Agent => 4,
    }
}

/// Run `cmd` on the origin device `origin` and seal what it produced into an envelope.
fn seal(origin: &DbWorker, meta: &CommandMeta, cmd: &WriteCommand) -> Sealed {
    origin.dispatch(meta.clone(), cmd.clone()).unwrap();
    let (payload, version, hash): (Vec<u8>, u32, Vec<u8>) = origin
        .lock()
        .conn
        .query_row(
            "SELECT payload, payload_schema_version, write_set_hash
               FROM operation_log WHERE command_id = ?1",
            [meta.command_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let immutable = ImmutableMeta {
        command_id: *meta.command_id.as_bytes(),
        correlation_id: *meta.correlation_id.as_bytes(),
        causation_id: meta.causation_id.map(|id| *id.as_bytes()),
        actor_type: actor_byte(meta.actor_type),
        actor_id: meta.actor_id.clone(),
        idempotency_key: IdempotencyKey::parse(&meta.idempotency_key).unwrap(),
        issued_at: meta.issued_at.as_unix_micros(),
        device_id: [0xb2; 16],
    };
    let vault_id = *origin.vault_id().unwrap().as_bytes();
    let fingerprint = Fingerprint::from_derived(FINGERPRINT);
    let header = ClearHeader {
        vault_id,
        key_epoch: 1,
        object: ObjectKind::Envelope {
            idempotency_key_tag: tag_for(&immutable.idempotency_key),
            immutable_metadata_commitment: immutable.commitment().unwrap(),
            artifact_manifest_commitment: None,
        },
        object_id: immutable.command_id,
        device_id: immutable.device_id,
        device_key_fingerprint: fingerprint,
        nonce_domain: NonceDomain::derive(&vault_id, 1, &fingerprint, OBJECT_ENVELOPE),
        invocation_counter: 0,
        base_seq: 0,
        payload_schema_version: version,
        engine_version: ENGINE.to_owned(),
        schema_version: 66,
    };
    Sealed {
        header,
        plaintext: EnvelopePlaintext {
            meta: immutable,
            write_set_hash: hash.try_into().unwrap(),
            delta_manifest: ArtifactManifest::new(Vec::new()).unwrap(),
            payload,
        },
    }
}

/// A live vault with an account, a base image at server seq 5 and an empty outbox, plus where its
/// scratch vaults go.
fn live_with_base() -> (TempDir, DbWorker, AccountId, PathBuf) {
    let (dir, worker) = worker();
    let account = AccountId::new();
    worker
        .dispatch(meta(), liquid_account_cmd(account, Currency::Usd, 50_000))
        .unwrap();
    worker.initialize_base_image(5).unwrap();
    let scratch = dir.path().join("rebase").join("scratch.db");
    (dir, worker, account, scratch)
}

/// Another device at the same base, as a scratch copy of it: it issues the server's commands.
fn other_device<'a>(live: &'a DbWorker, dir: &TempDir) -> Scratch<'a> {
    live.rollback_to(5, &dir.path().join("rebase").join("device-b.db"))
        .unwrap()
}

fn rename(id: AccountId, name: &str) -> WriteCommand {
    WriteCommand::UpdateAccount {
        id,
        name: name.to_owned(),
    }
}

fn account_name(worker: &DbWorker, id: AccountId) -> String {
    worker
        .lock()
        .conn
        .query_row(
            "SELECT name FROM accounts WHERE id = ?1",
            [id.as_uuid()],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn rolling_back_reaches_the_base_and_no_further() {
    let (_dir, live, _account, scratch) = live_with_base();
    for target in [0, 4, 6, 99] {
        let error = live.rollback_to(target, &scratch).unwrap_err();
        assert!(
            matches!(
                error,
                RebaseError::TargetNotTheBase {
                    requested,
                    base_seq: 5
                } if requested == target
            ),
            "target {target}: {error:?}"
        );
        assert!(!scratch.exists(), "a refused rollback creates nothing");
    }
    let rolled = live.rollback_to(5, &scratch).unwrap();
    assert_eq!((rolled.base_seq(), rolled.head_seq()), (5, 5));
}

#[test]
fn there_is_nothing_to_roll_back_to_without_a_usable_base() {
    let (dir, worker) = worker();
    let scratch = dir.path().join("scratch.db");
    let error = worker.rollback_to(0, &scratch).unwrap_err();
    assert!(
        matches!(error, RebaseError::NoUsableBase(BaseImageStatus::SyncOff)),
        "{error:?}"
    );
    assert!(!scratch.exists());
}

#[test]
fn the_scratch_vault_is_the_base_not_the_live_vault_and_the_live_vault_is_untouched() {
    let (_dir, live, account, scratch_path) = live_with_base();
    // The live vault moves on after the base was taken.
    live.dispatch(meta(), rename(account, "Renamed after the base"))
        .unwrap();
    let before = live.snapshot_bytes().unwrap();

    let scratch = live.rollback_to(5, &scratch_path).unwrap();

    assert_eq!(account_name(scratch.worker(), account), "Checking");
    assert_eq!(account_name(&live, account), "Renamed after the base");
    assert_eq!(
        live.snapshot_bytes().unwrap(),
        before,
        "the live vault is byte-identical"
    );
}

#[test]
fn a_scratch_path_that_exists_is_refused_and_left_alone() {
    let (_dir, live, _account, scratch_path) = live_with_base();
    std::fs::create_dir_all(scratch_path.parent().unwrap()).unwrap();
    std::fs::write(&scratch_path, b"someone else's file").unwrap();
    assert!(matches!(
        live.rollback_to(5, &scratch_path),
        Err(RebaseError::ScratchExists)
    ));
    assert_eq!(
        std::fs::read(&scratch_path).unwrap(),
        b"someone else's file"
    );
}

#[test]
fn pulled_envelopes_apply_in_seq_order_and_their_rows_are_recorded_as_touched() {
    let (dir, live, account, scratch_path) = live_with_base();
    let device_b = other_device(&live, &dir);
    let first = seal(
        device_b.worker(),
        &meta(),
        &rename(account, "From device B"),
    );
    let second = seal(
        device_b.worker(),
        &meta(),
        &liquid_account_cmd(AccountId::new(), Currency::Usd, 1_000),
    );
    let before = live.snapshot_bytes().unwrap();

    let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
    scratch
        .apply_pulled(&[first.pulled(6), second.pulled(7)])
        .unwrap();

    assert_eq!(scratch.head_seq(), 7);
    assert_eq!(account_name(scratch.worker(), account), "From device B");
    assert!(
        scratch
            .touched()
            .iter()
            .any(|(table, _)| table == "accounts"),
        "{:?}",
        scratch.touched()
    );
    // A server envelope is not an unpushed local command: no outbox row, and it records the base
    // it was applied on (the seq before it).
    let outbox: i64 = scratch
        .worker()
        .lock()
        .conn
        .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
        .unwrap();
    assert_eq!(outbox, 0);
    let applied_on: Vec<Option<i64>> = scratch
        .worker()
        .lock()
        .conn
        .prepare(
            "SELECT applied_base_seq FROM operation_log
              WHERE command_id IN (?1, ?2) ORDER BY op_seq",
        )
        .unwrap()
        .query_map(
            [
                Uuid::from_bytes(first.plaintext.meta.command_id),
                Uuid::from_bytes(second.plaintext.meta.command_id),
            ],
            |r| r.get(0),
        )
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(applied_on, vec![Some(5), Some(6)]);
    assert_eq!(
        live.snapshot_bytes().unwrap(),
        before,
        "the live vault is byte-identical"
    );
}

#[test]
fn envelopes_out_of_order_or_not_next_to_the_base_are_refused() {
    let (dir, live, account, scratch_path) = live_with_base();
    let device_b = other_device(&live, &dir);
    let envelope = seal(device_b.worker(), &meta(), &rename(account, "B"));
    let mut scratch = live.rollback_to(5, &scratch_path).unwrap();

    for seq in [5, 7, 9] {
        let error = scratch.apply_pulled(&[envelope.pulled(seq)]).unwrap_err();
        assert!(
            matches!(error, RebaseError::OutOfOrder { expected: 6, got } if got == seq),
            "seq {seq}: {error:?}"
        );
    }
    assert_eq!(scratch.head_seq(), 5);
    assert_eq!(account_name(scratch.worker(), account), "Checking");
}

#[test]
fn a_write_set_hash_that_differs_is_a_fork_that_leaves_the_live_vault_byte_identical() {
    let (dir, live, account, scratch_path) = live_with_base();
    let device_b = other_device(&live, &dir);
    let mut tampered = seal(
        device_b.worker(),
        &meta(),
        &rename(account, "From device B"),
    );
    let honest = tampered.plaintext.write_set_hash;
    tampered.plaintext.write_set_hash[0] ^= 0xff;
    let before = live.snapshot_bytes().unwrap();

    let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
    let error = scratch.apply_pulled(&[tampered.pulled(6)]).unwrap_err();

    let RebaseError::Fork(evidence) = error else {
        panic!("expected a fork, got {error:?}");
    };
    assert_eq!(evidence.seq, 6);
    assert_eq!(evidence.computed, honest);
    assert_ne!(evidence.expected, honest);
    assert_eq!(
        live.snapshot_bytes().unwrap(),
        before,
        "the live vault is byte-identical"
    );

    // The user-visible state: the replica's status halts on exactly this evidence, once.
    let mut status = SyncStatus::Healthy;
    let event =
        status.observe_write_set_mismatch(evidence.seq, evidence.expected, evidence.computed);
    assert_eq!(
        event,
        Some(SyncEvent::Halted(evidence.halt_evidence())),
        "the halt event is what the interface shows"
    );
    assert!(matches!(
        status,
        SyncStatus::Halted(HaltEvidence::WriteSetMismatch { seq: 6, .. })
    ));
    assert!(!status.can_push());
    let _: ForkEvidence = evidence;
}

#[test]
fn an_envelope_whose_memo_is_already_in_the_scratch_vault_is_not_applied_twice() {
    let (dir, live, account, scratch_path) = live_with_base();
    let device_b = other_device(&live, &dir);
    let envelope = seal(
        device_b.worker(),
        &meta(),
        &rename(account, "From device B"),
    );
    let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
    // The same key already applied here (an already-accepted command is reconciled, not re-applied).
    let key = Uuid::from_bytes(*envelope.plaintext.meta.idempotency_key.as_bytes()).to_string();
    scratch
        .worker()
        .lock()
        .conn
        .execute(
            "INSERT INTO command_idempotency_keys
               (idempotency_key, command_id, command_type, result_ref_json, op_seq, expires_at)
             VALUES (?1, zeroblob(16), 'update_account', '{}', 1, '2099-01-01T00:00:00Z')",
            [key],
        )
        .unwrap();

    let error = scratch.apply_pulled(&[envelope.pulled(6)]).unwrap_err();

    assert!(
        matches!(error, RebaseError::MemoPresent { .. }),
        "{error:?}"
    );
    assert_eq!(account_name(scratch.worker(), account), "Checking");
}

// ---- the plan: the four cells and the correlation closure ---------------------------------

mod plan_matrix {
    use std::collections::BTreeSet;

    use crate::rebase::{
        auto_iff, plan, Disposition, OutboxCommand, QueueCause, QueueReason, RowKey,
    };
    use uuid::Uuid;

    fn id(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn key(table: &str, pk: u8) -> RowKey {
        (table.to_owned(), vec![pk])
    }

    fn command(n: u8, correlation: u8, causation: Option<u8>, rows: &[RowKey]) -> OutboxCommand {
        OutboxCommand {
            command_id: id(n),
            correlation_id: id(correlation),
            causation_id: causation.map(id),
            write_set_keys: rows.to_vec(),
        }
    }

    fn touched(rows: &[RowKey]) -> BTreeSet<RowKey> {
        rows.iter().cloned().collect()
    }

    fn dispositions(
        outbox: &[OutboxCommand],
        touched: &BTreeSet<RowKey>,
        invalid: &[u8],
    ) -> Vec<Disposition> {
        let invalid: BTreeSet<Uuid> = invalid.iter().map(|n| id(*n)).collect();
        plan(outbox, touched, &invalid)
            .commands
            .into_iter()
            .map(|planned| planned.disposition)
            .collect()
    }

    const CHOOSE_TOUCHED: Disposition = Disposition::Queue {
        cause: QueueCause::Touched,
        reason: QueueReason::Choose,
    };
    const EDIT_INVALID: Disposition = Disposition::Queue {
        cause: QueueCause::Invalid,
        reason: QueueReason::Edit,
    };

    #[test]
    fn the_four_cells_of_untouched_by_valid() {
        // untouched and valid: the only automatic outcome.
        assert_eq!(auto_iff(true, true), Disposition::Replay);
        // touched but valid: both values are kept and the person chooses.
        assert_eq!(auto_iff(false, true), CHOOSE_TOUCHED);
        // untouched but no longer valid: the person repairs it.
        assert_eq!(auto_iff(true, false), EDIT_INVALID);
        // touched and invalid: it cannot be offered as a choice, so it is repaired.
        assert_eq!(auto_iff(false, false), EDIT_INVALID);
    }

    #[test]
    fn a_command_is_touched_only_when_the_exact_row_was_written() {
        let rows = [key("accounts", 1)];
        let one = command(1, 1, None, &rows);
        assert!(
            one.untouched(&touched(&[key("accounts", 2)])),
            "same table, another row"
        );
        assert!(
            one.untouched(&touched(&[key("categories", 1)])),
            "same key, another table"
        );
        assert!(!one.untouched(&touched(&[key("accounts", 1)])));
        assert!(command(2, 2, None, &[]).untouched(&touched(&[key("accounts", 1)])));
    }

    #[test]
    fn an_untouched_valid_command_is_replayed_and_the_order_is_the_local_order() {
        let outbox = [
            command(3, 3, None, &[key("accounts", 3)]),
            command(1, 1, None, &[key("accounts", 1)]),
            command(2, 2, None, &[key("accounts", 2)]),
        ];
        let planned = plan(&outbox, &touched(&[key("accounts", 9)]), &BTreeSet::new());
        let order: Vec<Uuid> = planned
            .commands
            .iter()
            .map(|p| p.command.command_id)
            .collect();
        assert_eq!(order, vec![id(3), id(1), id(2)]);
        assert!(planned.queued().next().is_none());
        assert_eq!(planned.replayed().count(), 3);
    }

    #[test]
    fn a_touched_command_queues_its_whole_correlation_group() {
        let outbox = [
            command(1, 1, None, &[key("accounts", 1)]), // touched
            command(2, 1, None, &[key("accounts", 2)]), // same group, disjoint rows
            command(3, 3, None, &[key("accounts", 3)]), // another group
        ];
        let got = dispositions(&outbox, &touched(&[key("accounts", 1)]), &[]);
        assert_eq!(got[0], CHOOSE_TOUCHED);
        assert_eq!(
            got[1],
            Disposition::Queue {
                cause: QueueCause::Closure { root: id(1) },
                reason: QueueReason::Choose
            },
            "a disjoint write set does not save a member of the group"
        );
        assert_eq!(got[2], Disposition::Replay, "another group is unaffected");
    }

    #[test]
    fn a_causal_descendant_queues_even_in_another_correlation_group_and_so_do_its_siblings() {
        let outbox = [
            command(1, 1, None, &[key("accounts", 1)]), // touched: creates the entity
            command(2, 2, Some(1), &[key("accounts", 2)]), // caused by 1, its own group
            command(3, 2, None, &[key("accounts", 3)]), // sibling of the descendant
            command(4, 4, Some(3), &[key("accounts", 4)]), // caused by that sibling: a chain
            command(5, 5, None, &[key("accounts", 5)]), // unrelated
        ];
        let got = dispositions(&outbox, &touched(&[key("accounts", 1)]), &[]);
        let closure = Disposition::Queue {
            cause: QueueCause::Closure { root: id(1) },
            reason: QueueReason::Choose,
        };
        assert_eq!(
            got,
            vec![
                CHOOSE_TOUCHED,
                closure,
                closure,
                closure,
                Disposition::Replay
            ]
        );
    }

    #[test]
    fn a_causation_naming_a_command_that_is_not_in_the_outbox_is_ignored() {
        // Its cause was pushed and accepted earlier: nothing to close over.
        let outbox = [command(1, 1, Some(99), &[key("accounts", 1)])];
        assert_eq!(
            dispositions(&outbox, &touched(&[]), &[]),
            vec![Disposition::Replay]
        );
    }

    #[test]
    fn an_invalid_command_queues_its_group_to_be_repaired() {
        let outbox = [
            command(1, 1, None, &[key("accounts", 1)]),
            command(2, 1, None, &[key("accounts", 2)]),
        ];
        let got = dispositions(&outbox, &touched(&[]), &[2]);
        // The group's earlier member is pulled in by the later invalid one, and is offered the
        // repair too.
        assert_eq!(
            got,
            vec![
                Disposition::Queue {
                    cause: QueueCause::Closure { root: id(2) },
                    reason: QueueReason::Edit
                },
                EDIT_INVALID
            ]
        );
    }

    #[test]
    fn the_first_root_in_local_order_is_recorded_when_two_groups_reach_one_command() {
        let outbox = [
            command(1, 1, None, &[key("accounts", 1)]), // touched
            command(2, 2, None, &[key("accounts", 2)]), // touched
            command(3, 3, Some(2), &[]),                // caused by 2
            command(4, 1, Some(2), &[]),                // sibling of 1 AND caused by 2
        ];
        let got = dispositions(
            &outbox,
            &touched(&[key("accounts", 1), key("accounts", 2)]),
            &[],
        );
        assert_eq!(
            got[3],
            Disposition::Queue {
                cause: QueueCause::Closure { root: id(1) },
                reason: QueueReason::Choose
            }
        );
        assert_eq!(
            got[2],
            Disposition::Queue {
                cause: QueueCause::Closure { root: id(2) },
                reason: QueueReason::Choose
            }
        );
    }

    #[test]
    fn the_reason_columns_are_the_ones_the_queue_table_allows() {
        assert_eq!(QueueReason::Choose.as_column(), "queue_choose");
        assert_eq!(QueueReason::Edit.as_column(), "queue_edit");
    }
}
