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
fn a_scratch_vault_can_be_built_from_another_image_at_the_position_the_caller_names() {
    let (dir, live, account, scratch_path) = live_with_base();
    // An image that is not the live vault's own base: a copy, so the test can tell it apart from
    // `sync/base.db` by its position and by what it is left as.
    let image = dir.path().join("genesis-image.db");
    std::fs::copy(crate::base_image::base_image_path(live.db_path()), &image).unwrap();
    let image_before = std::fs::read(&image).unwrap();
    live.dispatch(meta(), rename(account, "Renamed after the base"))
        .unwrap();
    let live_before = live.snapshot_bytes().unwrap();

    let scratch = live.scratch_from_image(&image, 3, &scratch_path).unwrap();

    // The position is the caller's, not the live base's (5).
    assert_eq!((scratch.base_seq(), scratch.head_seq()), (3, 3));
    assert_eq!(account_name(scratch.worker(), account), "Checking");
    scratch
        .worker()
        .dispatch(meta(), rename(account, "Written to the copy"))
        .unwrap();
    drop(scratch);
    assert_eq!(
        std::fs::read(&image).unwrap(),
        image_before,
        "the image is only ever copied"
    );
    assert_eq!(live.snapshot_bytes().unwrap(), live_before);
    assert!(matches!(
        live.scratch_from_image(&image, 3, &scratch_path),
        Err(RebaseError::ScratchExists)
    ));
    let missing = dir.path().join("rebase").join("none.db");
    assert!(matches!(
        live.scratch_from_image(&dir.path().join("absent.db"), 3, &missing),
        Err(RebaseError::Db(_))
    ));
    assert!(!missing.exists(), "a failed copy leaves nothing behind");
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

// ---- the executor: replay, queue, the memo and the rebuilt rows ---------------------------

mod executor {
    use super::*;
    use crate::rebase::{Disposition, LocalCommand, QueueCause, QueueReason, RebaseError};

    /// Run `cmd` on the live vault (Sync is on, so it is an unpushed outbox entry) and read back
    /// everything a rebase needs to replay or queue it.
    pub(super) fn issue(live: &DbWorker, meta: CommandMeta, cmd: WriteCommand) -> LocalCommand {
        live.dispatch(meta.clone(), cmd).unwrap();
        live.read_local_commands()
            .unwrap()
            .into_iter()
            .find(|c| c.meta.command_id == meta.command_id)
            .expect("the command is in the outbox")
    }

    fn op_log_row(worker: &DbWorker, command_id: Uuid) -> Option<(i64, Option<i64>, Option<i64>)> {
        worker
            .lock()
            .conn
            .query_row(
                "SELECT op_seq, applied_base_seq, rebased_from FROM operation_log
                  WHERE command_id = ?1",
                [command_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok()
    }

    fn memo_op_seq(worker: &DbWorker, key: &str) -> Option<i64> {
        worker
            .lock()
            .conn
            .query_row(
                "SELECT op_seq FROM command_idempotency_keys WHERE idempotency_key = ?1",
                [key],
                |r| r.get(0),
            )
            .ok()
    }

    fn count(worker: &DbWorker, sql: &str) -> i64 {
        worker.lock().conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// The commitment an envelope would bind for `meta`, issued by the vault `device`.
    fn commitment_of(meta: &CommandMeta, device: Uuid) -> sync_core::header::MetadataCommitment {
        ImmutableMeta {
            command_id: *meta.command_id.as_bytes(),
            correlation_id: *meta.correlation_id.as_bytes(),
            causation_id: meta.causation_id.map(|id| *id.as_bytes()),
            actor_type: actor_byte(meta.actor_type),
            actor_id: meta.actor_id.clone(),
            idempotency_key: IdempotencyKey::parse(&meta.idempotency_key).unwrap(),
            issued_at: meta.issued_at.as_unix_micros(),
            device_id: *device.as_bytes(),
        }
        .commitment()
        .unwrap()
    }

    #[test]
    fn a_replayed_command_rebuilds_its_whole_meta_from_the_log_and_commits_to_what_a_seal_before_the_rollback_bound(
    ) {
        let (_dir, live, account, scratch_path) = live_with_base();
        let mut original = meta();
        original.actor_id = "alice".to_owned();
        original.issued_at = IssuedAt::from_unix_micros(1_790_000_000_333_333);
        original.causation_id = Some(Uuid::now_v7());
        // A seal made BEFORE the rollback binds this commitment into the envelope header.
        let sealed_before = commitment_of(&original, live.node_id);
        let local = issue(&live, original.clone(), rename(account, "Local rename"));

        // The metadata a rebase works from is rebuilt from operation_log and sync_outbox.issued_at
        // alone: nothing of `original` is carried across.
        assert_eq!(local.meta.command_id, original.command_id);
        assert_eq!(local.meta.issued_at, original.issued_at);
        assert_eq!(commitment_of(&local.meta, live.node_id), sealed_before);

        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        // The replay applies exactly that metadata: the rebuilt vault's log row, rebuilt the same
        // way, commits to the same value.
        let executed = scratch.rebase_local(std::slice::from_ref(&local)).unwrap();
        assert_eq!(executed.replayed.len(), 1);
        let replayed = scratch.worker().read_local_commands().unwrap();
        assert_eq!(replayed.len(), 1);
        assert_eq!(
            commitment_of(&replayed[0].meta, scratch.worker().node_id),
            sealed_before,
            "the meta commitment after the rollback and replay equals the one sealed before it"
        );
    }

    #[test]
    fn an_untouched_valid_command_is_replayed_under_its_original_ids_on_the_new_base() {
        let (dir, live, account, scratch_path) = live_with_base();
        // Local, unpushed: a second account. Server: a rename of the first.
        let local = issue(
            &live,
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 7_000),
        );
        let device_b = other_device(&live, &dir);
        let remote = seal(
            device_b.worker(),
            &meta(),
            &rename(account, "From device B"),
        );

        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        scratch.apply_pulled(&[remote.pulled(6)]).unwrap();
        let remote_op_seq = op_log_row(
            scratch.worker(),
            Uuid::from_bytes(remote.plaintext.meta.command_id),
        )
        .unwrap()
        .0;
        let executed = scratch.rebase_local(std::slice::from_ref(&local)).unwrap();

        // Re-applied under its original command id and key, once, and recorded as rebased.
        assert_eq!(executed.replayed.len(), 1);
        let row = op_log_row(scratch.worker(), local.meta.command_id).unwrap();
        assert_eq!(row.1, Some(6), "applied on the new base (the pulled head)");
        assert_eq!(
            row.2,
            Some(local.old_op_seq),
            "it replaced its pre-rebase op_seq"
        );
        assert_eq!(executed.replayed[0].new_op_seq, row.0);
        // The old op_seq is now claimed by the remote operation, and the local memo points at the
        // REBUILT op_seq, never at the pre-rebase number.
        assert_eq!(remote_op_seq, local.old_op_seq);
        assert_ne!(row.0, local.old_op_seq);
        assert_eq!(
            memo_op_seq(scratch.worker(), &local.meta.idempotency_key),
            Some(row.0)
        );
        // It is an unpushed entry on the rebuilt vault, keyed by the rebuilt op_seq.
        let (state, op_seq): (String, i64) = scratch
            .worker()
            .lock()
            .conn
            .query_row(
                "SELECT state, op_seq FROM sync_outbox WHERE command_id = ?1",
                [local.meta.command_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((state.as_str(), op_seq), ("Pending", row.0));
        assert_eq!(
            count(scratch.worker(), "SELECT COUNT(*) FROM outbox_queue"),
            0
        );
        // Both effects are in the rebuilt vault.
        assert_eq!(account_name(scratch.worker(), account), "From device B");
        assert_eq!(count(scratch.worker(), "SELECT COUNT(*) FROM accounts"), 2);
    }

    #[test]
    fn a_touched_command_is_queued_with_its_envelope_and_never_applied() {
        let (dir, live, account, scratch_path) = live_with_base();
        let local = issue(&live, meta(), rename(account, "Local rename"));
        let device_b = other_device(&live, &dir);
        let remote = seal(
            device_b.worker(),
            &meta(),
            &rename(account, "Remote rename"),
        );

        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        scratch.apply_pulled(&[remote.pulled(6)]).unwrap();
        let executed = scratch.rebase_local(std::slice::from_ref(&local)).unwrap();

        assert!(executed.replayed.is_empty());
        let queued: Vec<_> = executed.plan.queued().collect();
        assert_eq!(queued.len(), 1);
        assert_eq!(
            (queued[0].1, queued[0].2),
            (QueueCause::Touched, QueueReason::Choose)
        );
        // The server's value stands; the local value is preserved only in the queue.
        assert_eq!(account_name(scratch.worker(), account), "Remote rename");
        assert!(op_log_row(scratch.worker(), local.meta.command_id).is_none());
        assert_eq!(
            memo_op_seq(scratch.worker(), &local.meta.idempotency_key),
            None
        );
        let (reason, correlation, origin, envelope): (String, Uuid, Uuid, Vec<u8>) = scratch
            .worker()
            .lock()
            .conn
            .query_row(
                "SELECT reason, correlation_id, origin_node_id, envelope FROM outbox_queue
                  WHERE command_id = ?1",
                [local.meta.command_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(reason, "queue_choose");
        assert_eq!(correlation, local.meta.correlation_id);
        assert_eq!(origin, scratch.worker().node_id);
        // The preserved envelope is the command's complete original metadata and payload.
        let preserved = EnvelopePlaintext::decode(&envelope).unwrap();
        assert_eq!(
            preserved.meta.issued_at,
            local.meta.issued_at.as_unix_micros()
        );
        assert_eq!(preserved.payload, local.payload);
        assert_eq!(preserved.write_set_hash, local.write_set_hash);
        // An ordinary retry of it is told so; it is neither applied nor a no-op success.
        let retry = scratch
            .worker()
            .dispatch(local.meta.clone(), local.cmd.clone())
            .unwrap_err();
        assert!(
            matches!(retry, DbError::QueuedForDisposition { command_id } if command_id == local.meta.command_id),
            "{retry:?}"
        );
        assert_eq!(account_name(scratch.worker(), account), "Remote rename");
    }

    #[test]
    fn a_two_device_rebase_replays_one_group_and_queues_the_other_with_its_causal_descendant() {
        let (dir, live, account, scratch_path) = live_with_base();
        // Group 1 (auto): a new account. Group 2: a rename of the shared account (touched by the
        // server) and a command it caused, in another correlation group, on disjoint rows.
        let auto = issue(
            &live,
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 3_000),
        );
        let root = issue(&live, meta(), rename(account, "Local rename"));
        let mut caused_meta = meta();
        caused_meta.causation_id = Some(root.meta.command_id);
        let descendant = issue(
            &live,
            caused_meta,
            liquid_account_cmd(AccountId::new(), Currency::Usd, 4_000),
        );
        let device_b = other_device(&live, &dir);
        let remote = seal(
            device_b.worker(),
            &meta(),
            &rename(account, "Remote rename"),
        );

        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        scratch.apply_pulled(&[remote.pulled(6)]).unwrap();
        let outbox = [auto.clone(), root.clone(), descendant.clone()];
        let executed = scratch.rebase_local(&outbox).unwrap();

        assert_eq!(
            executed.plan.commands[0].disposition,
            Disposition::Replay,
            "the untouched group replays"
        );
        assert_eq!(
            executed.plan.commands[1].disposition,
            Disposition::Queue {
                cause: QueueCause::Touched,
                reason: QueueReason::Choose
            }
        );
        assert_eq!(
            executed.plan.commands[2].disposition,
            Disposition::Queue {
                cause: QueueCause::Closure {
                    root: root.meta.command_id
                },
                reason: QueueReason::Choose
            },
            "a causal descendant queues with the command that caused it"
        );
        // Nothing of the queued group was applied, including the descendant's disjoint rows.
        assert!(op_log_row(scratch.worker(), auto.meta.command_id).is_some());
        assert!(op_log_row(scratch.worker(), root.meta.command_id).is_none());
        assert!(op_log_row(scratch.worker(), descendant.meta.command_id).is_none());
        assert_eq!(
            count(scratch.worker(), "SELECT COUNT(*) FROM outbox_queue"),
            2
        );
        assert_eq!(count(scratch.worker(), "SELECT COUNT(*) FROM accounts"), 2);
    }

    #[test]
    fn a_command_the_new_base_refuses_queues_its_whole_group_to_be_repaired() {
        let (dir, live, _account, scratch_path) = live_with_base();
        let (parent, child) = (CategoryId::new(), CategoryId::new());
        for (id, name) in [(parent, "Parent"), (child, "Child")] {
            live.dispatch(
                meta(),
                WriteCommand::CreateCategory {
                    id,
                    parent_id: None,
                    name: name.to_owned(),
                    category_type: "expense".to_owned(),
                    color: None,
                    icon: None,
                },
            )
            .unwrap();
        }
        // The base is taken AFTER both categories exist, with the outbox empty.
        live.lock()
            .conn
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        live.advance_base_image(6).unwrap();
        // Local: move the child under the parent. Server: move the parent under the child.
        let local = issue(
            &live,
            meta(),
            WriteCommand::MoveCategory {
                id: child,
                new_parent_id: Some(parent),
            },
        );
        let sibling = {
            let mut same_group = meta();
            same_group.correlation_id = local.meta.correlation_id;
            issue(
                &live,
                same_group,
                liquid_account_cmd(AccountId::new(), Currency::Usd, 1_000),
            )
        };
        let device_b = live
            .rollback_to(6, &dir.path().join("rebase").join("b.db"))
            .unwrap();
        let remote = seal(
            device_b.worker(),
            &meta(),
            &WriteCommand::MoveCategory {
                id: parent,
                new_parent_id: Some(child),
            },
        );

        let mut scratch = live.rollback_to(6, &scratch_path).unwrap();
        scratch.apply_pulled(&[remote.pulled(7)]).unwrap();
        let executed = scratch
            .rebase_local(&[local.clone(), sibling.clone()])
            .unwrap();

        // The move would make a cycle on the new base: the command is invalid, not touched, and
        // so is its group.
        assert!(executed.replayed.is_empty());
        let queued: Vec<_> = executed.plan.queued().collect();
        assert_eq!(queued.len(), 2);
        assert_eq!(
            (queued[0].1, queued[0].2),
            (QueueCause::Invalid, QueueReason::Edit)
        );
        assert!(matches!(queued[1].1, QueueCause::Closure { .. }));
        assert_eq!(queued[1].2, QueueReason::Edit);
        assert!(op_log_row(scratch.worker(), sibling.meta.command_id).is_none());
        let reasons: Vec<String> = scratch
            .worker()
            .lock()
            .conn
            .prepare("SELECT reason FROM outbox_queue")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(reasons, vec!["queue_edit", "queue_edit"]);
    }

    #[test]
    fn replaying_the_same_outbox_twice_leaves_one_row_per_command() {
        let (dir, live, account, scratch_path) = live_with_base();
        let kept = issue(
            &live,
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 2_000),
        );
        let queued = issue(&live, meta(), rename(account, "Local rename"));
        let device_b = other_device(&live, &dir);
        let remote = seal(
            device_b.worker(),
            &meta(),
            &rename(account, "Remote rename"),
        );
        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        scratch.apply_pulled(&[remote.pulled(6)]).unwrap();
        let outbox = [kept.clone(), queued.clone()];
        scratch.rebase_local(&outbox).unwrap();
        let log_rows = count(scratch.worker(), "SELECT COUNT(*) FROM operation_log");

        let again = scratch.rebase_local(&outbox).unwrap_err();

        assert!(matches!(again, RebaseError::AlreadyRebased), "{again:?}");
        for command in [&kept, &queued] {
            let id = command.meta.command_id;
            let in_log: i64 = scratch
                .worker()
                .lock()
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM operation_log WHERE command_id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap();
            let in_queue: i64 = scratch
                .worker()
                .lock()
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM outbox_queue WHERE command_id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                in_log + in_queue,
                1,
                "{id}: one row, in the log or in the queue"
            );
        }
        assert_eq!(
            count(scratch.worker(), "SELECT COUNT(*) FROM operation_log"),
            log_rows
        );
    }

    #[test]
    fn unrelated_memos_are_untouched_and_the_live_vault_is_never_written() {
        let (dir, live, account, scratch_path) = live_with_base();
        let local = issue(
            &live,
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 2_000),
        );
        let device_b = other_device(&live, &dir);
        let remote = seal(
            device_b.worker(),
            &meta(),
            &rename(account, "Remote rename"),
        );
        let live_before = live.snapshot_bytes().unwrap();
        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        scratch.apply_pulled(&[remote.pulled(6)]).unwrap();
        let memos = |worker: &DbWorker| -> Vec<(String, i64)> {
            worker
                .lock()
                .conn
                .prepare("SELECT idempotency_key, op_seq FROM command_idempotency_keys ORDER BY 1")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let before = memos(scratch.worker());

        scratch.rebase_local(std::slice::from_ref(&local)).unwrap();

        let after = memos(scratch.worker());
        let added: Vec<_> = after.iter().filter(|m| !before.contains(m)).collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].0, local.meta.idempotency_key);
        assert!(
            before.iter().all(|m| after.contains(m)),
            "no existing memo moved"
        );
        assert_eq!(live.snapshot_bytes().unwrap(), live_before);
    }

    #[test]
    fn class_two_is_rebuilt_once_after_the_replay() {
        let (_dir, live, _account, scratch_path) = live_with_base();
        let local = issue(
            &live,
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 2_000),
        );
        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        // Damage a class-2 read model in the post-pull scratch vault: only a full rebuild restores
        // it (an apply updates it incrementally, for the rows it touches).
        scratch
            .worker()
            .lock()
            .conn
            .execute("DELETE FROM transaction_display_rows_read_model", [])
            .unwrap();
        assert_eq!(
            count(
                scratch.worker(),
                "SELECT COUNT(*) FROM transaction_display_rows_read_model"
            ),
            0
        );

        scratch.rebase_local(std::slice::from_ref(&local)).unwrap();

        // The damage is gone and the projection is exactly what a full rebuild gives.
        let rows = count(
            scratch.worker(),
            "SELECT COUNT(*) FROM transaction_display_rows_read_model",
        );
        assert!(rows > 0, "the projection was rebuilt");
        let after_rebase = scratch.worker().transaction_display_checksum().unwrap();
        scratch.worker().rebuild_transaction_display().unwrap();
        assert_eq!(
            scratch.worker().transaction_display_checksum().unwrap(),
            after_rebase
        );
    }
}

// ---- the first pull after a bootstrap (bxsbz.4 AC7) ---------------------------------------

mod first_pull {
    use super::*;
    use crate::rebase::{replay, rollback_to};

    /// A fixture vault standing in for a snapshot bootstrap at server seq `n`: its base image is
    /// initialized through the entry point a bootstrap calls, with nothing unpushed.
    fn bootstrapped(n: i64) -> (TempDir, DbWorker, AccountId) {
        let (dir, worker) = worker();
        let account = AccountId::new();
        worker
            .dispatch(meta(), liquid_account_cmd(account, Currency::Usd, 9_000))
            .unwrap();
        worker.initialize_base_image(n).unwrap();
        (dir, worker, account)
    }

    #[test]
    fn a_bootstrapped_vault_rebases_cleanly_on_its_first_pulled_envelope() {
        const N: i64 = 41;
        let (dir, live, account) = bootstrapped(N);
        let device_b = live
            .rollback_to(N, &dir.path().join("rebase").join("b.db"))
            .unwrap();
        let first = seal(device_b.worker(), &meta(), &rename(account, "First pull"));

        let mut scratch = rollback_to(&live, N, &dir.path().join("rebase").join("s.db")).unwrap();
        let executed = replay(&mut scratch, &[first.pulled(42)], &[]).unwrap();

        assert_eq!(scratch.head_seq(), 42, "the first envelope is seq N+1");
        assert!(executed.replayed.is_empty());
        assert_eq!(account_name(scratch.worker(), account), "First pull");
    }

    #[test]
    fn it_also_rebases_with_an_unpushed_local_command_on_top() {
        const N: i64 = 41;
        let (dir, live, account) = bootstrapped(N);
        // The bootstrapped device did local work after its base was taken: unpushed.
        let local = super::executor::issue(
            &live,
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 500),
        );
        let device_b = live
            .rollback_to(N, &dir.path().join("rebase").join("b.db"))
            .unwrap();
        let first = seal(device_b.worker(), &meta(), &rename(account, "First pull"));

        let mut scratch = rollback_to(&live, N, &dir.path().join("rebase").join("s.db")).unwrap();
        let executed = replay(
            &mut scratch,
            &[first.pulled(42)],
            std::slice::from_ref(&local),
        )
        .unwrap();

        assert_eq!(executed.replayed.len(), 1);
        assert_eq!(account_name(scratch.worker(), account), "First pull");
        let applied_on: Option<i64> = scratch
            .worker()
            .lock()
            .conn
            .query_row(
                "SELECT applied_base_seq FROM operation_log WHERE command_id = ?1",
                [local.meta.command_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(applied_on, Some(42));
    }
}

// ---- envelopes this build cannot apply ----------------------------------------------------

#[test]
fn a_pulled_envelope_that_cannot_be_decoded_stops_the_rebase_before_anything_is_applied() {
    let (dir, live, account, scratch_path) = live_with_base();
    let device_b = other_device(&live, &dir);
    let mut bad_payload = seal(device_b.worker(), &meta(), &rename(account, "B"));
    bad_payload.plaintext.payload = b"{\"not\":\"a command\"}".to_vec();
    let mut scratch = live.rollback_to(5, &scratch_path).unwrap();

    let error = scratch.apply_pulled(&[bad_payload.pulled(6)]).unwrap_err();

    assert!(
        matches!(
            error,
            RebaseError::Undecodable {
                seq: 6,
                cause: crate::rebase::Undecodable::Payload(_)
            }
        ),
        "{error:?}"
    );
    assert_eq!(scratch.head_seq(), 5);
    assert_eq!(account_name(scratch.worker(), account), "Checking");
}

#[test]
fn a_pulled_envelope_the_commands_own_validation_refuses_stops_the_replica_at_its_position() {
    let (dir, live, _account, scratch_path) = live_with_base();
    let device_b = other_device(&live, &dir);
    let category = CategoryId::new();
    // Device B creates a category and then moves it; only the move is offered, so its subject does
    // not exist on this replica's base.
    seal(
        device_b.worker(),
        &meta(),
        &WriteCommand::CreateCategory {
            id: category,
            parent_id: None,
            name: "Only on B".to_owned(),
            category_type: "expense".to_owned(),
            color: None,
            icon: None,
        },
    );
    let missing = seal(
        device_b.worker(),
        &meta(),
        &WriteCommand::MoveCategory {
            id: category,
            new_parent_id: None,
        },
    );
    let mut scratch = live.rollback_to(5, &scratch_path).unwrap();

    let error = scratch.apply_pulled(&[missing.pulled(6)]).unwrap_err();

    assert!(
        matches!(error, RebaseError::PulledEnvelopeRefused { seq: 6, .. }),
        "{error:?}"
    );
    assert_eq!(scratch.head_seq(), 5, "the replica stops at the position");
}

// ---- reading the unpushed commands back ---------------------------------------------------

mod outbox_reader {
    use super::*;
    use crate::rebase::RebaseError;

    #[test]
    fn the_unpushed_commands_come_back_in_local_order_with_their_complete_original_meta() {
        let (_dir, live, account, _scratch) = live_with_base();
        let mut first = meta();
        first.actor_id = "alice".to_owned();
        first.issued_at = IssuedAt::from_unix_micros(1_790_000_000_111_111);
        let mut second = meta();
        second.causation_id = Some(first.command_id);
        second.issued_at = IssuedAt::from_unix_micros(1_790_000_000_222_222);
        live.dispatch(first.clone(), rename(account, "One"))
            .unwrap();
        live.dispatch(second.clone(), rename(account, "Two"))
            .unwrap();

        let read = live.read_local_commands().unwrap();

        assert_eq!(read.len(), 2);
        for (got, want) in read.iter().zip([&first, &second]) {
            assert_eq!(got.meta.command_id, want.command_id);
            assert_eq!(got.meta.correlation_id, want.correlation_id);
            assert_eq!(got.meta.causation_id, want.causation_id);
            assert_eq!(got.meta.actor_id, want.actor_id);
            assert_eq!(got.meta.idempotency_key, want.idempotency_key);
            assert_eq!(
                got.meta.issued_at, want.issued_at,
                "the issue time, not the wall clock"
            );
            assert!(!got.command.write_set_keys.is_empty());
        }
        assert!(read[0].old_op_seq < read[1].old_op_seq);
        assert!(
            matches!(read[0].cmd, WriteCommand::UpdateAccount { ref name, .. } if name == "One")
        );
    }

    #[test]
    fn an_entry_the_server_already_accepted_is_not_replayed() {
        let (_dir, live, account, _scratch) = live_with_base();
        let sent = meta();
        live.dispatch(sent.clone(), rename(account, "Pushed"))
            .unwrap();
        live.dispatch(meta(), rename(account, "Still local"))
            .unwrap();
        live.lock()
            .conn
            .execute(
                "UPDATE sync_outbox
                    SET state = 'Bound', sealed_container = zeroblob(1), bound_seq = 6,
                        result_ref = zeroblob(1)
                  WHERE command_id = ?1",
                [sent.command_id],
            )
            .unwrap();

        let read = live.read_local_commands().unwrap();

        assert_eq!(read.len(), 1);
        assert!(
            matches!(read[0].cmd, WriteCommand::UpdateAccount { ref name, .. } if name == "Still local")
        );
    }

    #[test]
    fn a_row_with_no_recorded_issue_time_is_refused_by_name() {
        let (_dir, live, account, _scratch) = live_with_base();
        let m = meta();
        live.dispatch(m.clone(), rename(account, "Old row"))
            .unwrap();
        live.lock()
            .conn
            .execute("UPDATE sync_outbox SET issued_at = NULL", [])
            .unwrap();

        let error = live.read_local_commands().unwrap_err();

        assert!(
            matches!(error, RebaseError::MissingIssuedAt { command_id } if command_id == m.command_id),
            "{error:?}"
        );
    }

    #[test]
    fn a_vault_with_nothing_unpushed_reads_back_nothing() {
        let (_dir, live, _account, _scratch) = live_with_base();
        assert!(live.read_local_commands().unwrap().is_empty());
    }
}
