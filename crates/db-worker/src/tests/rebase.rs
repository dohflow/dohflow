//! The rebase engine core (personal-cfo-bxsbz.4): the scratch vault, rolling back to the base,
//! applying the server's envelopes with their write-set hashes verified, and the fork. Fake data
//! only.

use sync_core::envelope::{check_envelope, EnvelopePlaintext, IdempotencyKey, ImmutableMeta};
use sync_core::header::{
    ClearHeader, Fingerprint, IdempotencyKeyTag, NonceDomain, ObjectKind, OBJECT_ENVELOPE,
};
use sync_core::status::{HaltEvidence, SyncEvent, SyncStatus};

use super::*;
use crate::base_image::BaseImageStatus;
use crate::rebase::{ForkEvidence, PulledEnvelope, RebaseError, Scratch};

const FINGERPRINT: [u8; 32] = [7; 32];
const ENGINE: &str = "dohflow-engine-3";

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
    /// The container the service stores: the header and a stand-in ciphertext (the AEAD is the
    /// client engine's, not the rebase's, so any bytes will do).
    fn container(&self) -> Vec<u8> {
        sync_core::header::encode_container(&self.header, &self.plaintext.encode().unwrap())
            .unwrap()
    }

    /// The entry a reconciliation is offered for the log position `seq`.
    fn entry<'a>(
        &'a self,
        container: &'a [u8],
        seq: u64,
        result_ref: Option<Vec<u8>>,
    ) -> crate::rebase::PulledLogEntry<'a> {
        crate::rebase::PulledLogEntry {
            envelope: self.pulled(seq),
            container,
            result_ref,
        }
    }

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
            artifact_binding_commitment: None,
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
            delta_manifest: sync_core::envelope::BoundManifest::empty(),
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
    fn a_sibling_ordered_before_a_refused_command_is_replayed_once_on_a_fresh_trial() {
        // 04b's #291 F1: the refused command's group queues and the replay restarts from a fresh
        // copy of the new base. A sibling that was applied to the first trial BEFORE the refusal
        // must be applied again to the fresh one, once: a replay that reused the first trial would
        // find its memo (an abort) or leave it applied twice.
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
        live.lock()
            .conn
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        live.advance_base_image(6).unwrap();
        // The independent sibling comes FIRST in local order; the command the new base refuses
        // (the move that would make a cycle) comes after it.
        let before = issue(
            &live,
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 1_000),
        );
        let refused = issue(
            &live,
            meta(),
            WriteCommand::MoveCategory {
                id: child,
                new_parent_id: Some(parent),
            },
        );
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
            .rebase_local(&[before.clone(), refused.clone()])
            .unwrap();

        assert_eq!(executed.replayed.len(), 1);
        assert_eq!(executed.replayed[0].command_id, before.meta.command_id);
        let queued: Vec<_> = executed.plan.queued().collect();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].0.command_id, refused.meta.command_id);
        assert_eq!(
            count(scratch.worker(), "SELECT COUNT(*) FROM accounts"),
            2,
            "the base's account and the sibling, once"
        );
        let logged: i64 = scratch
            .worker()
            .lock()
            .conn
            .query_row(
                "SELECT COUNT(*) FROM operation_log WHERE command_id = ?1",
                [before.meta.command_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(logged, 1);
        assert!(op_log_row(scratch.worker(), refused.meta.command_id).is_none());
    }

    #[test]
    fn a_scratch_beside_the_live_vault_shares_its_sync_directory_and_never_reconciles_it() {
        // 04b's #291 F2: a scratch placed in the live vault's own directory has the live vault's
        // `sync/` as its Sync directory. Opening it must not reconcile that directory: a copy of
        // the base image records no `sync_base` row of its own, and a reconcile of that would
        // delete the live vault's `sync/base.db`.
        let (dir, live, _account, _scratch) = live_with_base();
        let sync = crate::base_image::sync_dir(&live.path);
        let beside = live.path.with_file_name("scratch-beside.db");
        assert_eq!(crate::base_image::sync_dir(&beside), sync);
        let base_before =
            crate::base_image::sha256_file(&crate::base_image::base_image_path(&live.path))
                .unwrap();
        let names = |dir: &std::path::Path| -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| !name.ends_with(".runner.lock"))
                .collect();
            names.sort();
            names
        };
        let sync_names = names(&sync);

        let scratch = live.rollback_to(5, &beside).unwrap();

        assert_eq!(names(&sync), sync_names, "nothing was added or removed");
        assert_eq!(
            crate::base_image::sha256_file(&crate::base_image::base_image_path(&live.path))
                .unwrap(),
            base_before
        );
        assert_eq!(
            live.base_image_status().unwrap(),
            BaseImageStatus::Ready { base_seq: 5 }
        );
        drop(scratch);
        drop(dir);
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
    use crate::rebase::{OutboxEntryState, RebaseError};

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
    fn an_entry_the_server_already_accepted_reads_back_bound_and_is_not_replayed() {
        let (_dir, live, account, scratch) = live_with_base();
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

        assert_eq!(read.len(), 2);
        assert!(matches!(
            read[0].entry,
            OutboxEntryState::Bound { seq: 6, .. }
        ));
        assert_eq!(read[1].entry, OutboxEntryState::Pending);
        // The executor refuses a Bound entry: it is reconciled, never replayed.
        let mut scratch = live.rollback_to(5, &scratch).unwrap();
        let error = scratch.rebase_local(&read).unwrap_err();
        assert!(
            matches!(error, RebaseError::NotReconciled { command_id } if command_id == sent.command_id),
            "{error:?}"
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

// ---- already-accepted reconciliation and the re-seal rule (personal-cfo-bxsbz.5) ------------

mod accepted {
    use rusqlite::params;
    use sync_core::accepted::{decide_push, AcceptedRecord, InMemoryIndex, PushIdentity};
    use sync_core::envelope::{canonical_envelope_digest, ReceiverError};
    use sync_core::header::{parse_container, MetadataCommitment};
    use sync_core::status::EngineVersion;

    use super::*;
    use crate::rebase::{
        reconcile, reconcile_and_replay, AcceptedEvidence, Disagreement, ForkSource,
        OutboxEntryState, Probe, ProtocolForkEvidence, RebaseError, SettledBy,
    };
    use crate::sync_state::decode_row_keys;

    struct OutboxRow {
        state: String,
        op_seq: i64,
        bound_seq: Option<i64>,
        result_ref: Option<Vec<u8>>,
        sealed: Option<Vec<u8>>,
        keys: Vec<u8>,
    }

    fn outbox_row(worker: &DbWorker, id: Uuid) -> Option<OutboxRow> {
        worker
            .lock()
            .conn
            .query_row(
                "SELECT state, op_seq, bound_seq, result_ref, sealed_container, write_set_keys
                   FROM sync_outbox WHERE command_id = ?1",
                [id],
                |r| {
                    Ok(OutboxRow {
                        state: r.get(0)?,
                        op_seq: r.get(1)?,
                        bound_seq: r.get(2)?,
                        result_ref: r.get(3)?,
                        sealed: r.get(4)?,
                        keys: r.get(5)?,
                    })
                },
            )
            .ok()
    }

    fn log_row(worker: &DbWorker, id: Uuid) -> Option<(i64, Option<i64>)> {
        worker
            .lock()
            .conn
            .query_row(
                "SELECT op_seq, rebased_from FROM operation_log WHERE command_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
    }

    fn count(worker: &DbWorker, sql: &str) -> i64 {
        worker.lock().conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn log_len(worker: &DbWorker) -> i64 {
        count(worker, "SELECT count(*) FROM operation_log")
    }

    fn approve(live: &DbWorker, id: Uuid, container: &[u8]) {
        live.lock()
            .conn
            .execute(
                "UPDATE sync_outbox SET state = 'Approved', sealed_container = ?2
                  WHERE command_id = ?1",
                params![id, container],
            )
            .unwrap();
    }

    fn digest_of(container: &[u8]) -> [u8; 32] {
        let parsed = parse_container(container).unwrap();
        canonical_envelope_digest(parsed.header_bytes, parsed.ciphertext).unwrap()
    }

    fn accept(index: &mut InMemoryIndex, container: &[u8], seq: u64, result: Option<Vec<u8>>) {
        let parsed = parse_container(container).unwrap();
        let identity = PushIdentity::from_container(&parsed).unwrap();
        index.insert(identity.accepted(1, seq, result)).unwrap();
    }

    /// What re-pushing `container` byte for byte tells the engine.
    fn probe(index: &InMemoryIndex, container: &[u8]) -> Probe {
        let parsed = parse_container(container).unwrap();
        let identity = PushIdentity::from_container(&parsed).unwrap();
        Probe::from_decision(&decide_push(index, &identity).unwrap())
    }

    /// A live vault whose base image already holds `cmd`, as a snapshot bootstrap past its
    /// acceptance would, with the command still unacknowledged (`Approved`) in the outbox.
    fn live_with_accepted_command_in_base() -> (TempDir, DbWorker, AccountId, PathBuf, Sealed, Uuid)
    {
        let (dir, live) = worker();
        let account = AccountId::new();
        live.dispatch(meta(), liquid_account_cmd(account, Currency::Usd, 50_000))
            .unwrap();
        let c = meta();
        let cmd = rename(account, "Mine");
        live.dispatch(c.clone(), cmd.clone()).unwrap();
        live.initialize_base_image(5).unwrap();
        let origin = live
            .rollback_to(5, &dir.path().join("rebase").join("sealer.db"))
            .unwrap();
        let sealed = seal(origin.worker(), &c, &cmd);
        drop(origin);
        let bytes = sealed.container();
        live.lock()
            .conn
            .execute(
                "INSERT INTO sync_outbox
                   (command_id, op_seq, state, write_set_keys, sealed_container, created_at,
                    issued_at)
                 SELECT command_id, op_seq, 'Approved', x'', ?2, '2026-11-06T00:00:00Z', ?3
                   FROM operation_log WHERE command_id = ?1",
                params![c.command_id, bytes, c.issued_at.as_unix_micros()],
            )
            .unwrap();
        let scratch = dir.path().join("rebase").join("scratch.db");
        (dir, live, account, scratch, sealed, c.command_id)
    }

    fn as_fork(error: RebaseError) -> ProtocolForkEvidence {
        match error {
            RebaseError::ProtocolFork(evidence) => *evidence,
            other => panic!("expected a ProtocolFork, got {other:?}"),
        }
    }

    // -- the tail ---------------------------------------------------------------------------------

    #[test]
    fn an_accepted_command_in_the_tail_is_bound_to_its_position_and_never_replayed_or_pending() {
        let (dir, live, account, scratch_path) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let cmd = rename(account, "Mine");
        let sealed = seal(origin.worker(), &c, &cmd);
        live.dispatch(c.clone(), cmd).unwrap();
        let bytes = sealed.container();
        approve(&live, c.command_id, &bytes);
        // The local row keys are stale on the new base: the bound row takes the scratch vault's.
        live.lock()
            .conn
            .execute("UPDATE sync_outbox SET write_set_keys = x''", [])
            .unwrap();
        let local = live.read_local_commands().unwrap();
        let old_op_seq = local[0].old_op_seq;
        let live_log = log_len(&live);

        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        let pulled = [sealed.entry(&bytes, 6, Some(vec![7, 7]))];
        let (reconciliation, executed) =
            reconcile_and_replay(&mut scratch, &pulled, &local, &AcceptedEvidence::new()).unwrap();

        assert_eq!(reconciliation.settled.len(), 1);
        assert_eq!(reconciliation.settled[0].by, SettledBy::Tail);
        assert!(
            executed.replayed.is_empty(),
            "an accepted command is not replayed"
        );
        let row = outbox_row(scratch.worker(), c.command_id).unwrap();
        assert_eq!(row.state, "Bound");
        assert_eq!(row.bound_seq, Some(6));
        assert_eq!(row.result_ref, Some(vec![7, 7]));
        assert_eq!(
            row.sealed.as_deref(),
            Some(bytes.as_slice()),
            "the container is kept"
        );
        assert!(
            !decode_row_keys(&row.keys).unwrap().is_empty(),
            "the rows the scratch apply wrote"
        );
        let (new_op_seq, rebased_from) = log_row(scratch.worker(), c.command_id).unwrap();
        assert_eq!(
            row.op_seq, new_op_seq,
            "the outbox row follows the rebuilt log row"
        );
        assert_eq!(
            rebased_from,
            Some(old_op_seq),
            "the log row remembers what it replaced"
        );
        // AC3: an accepted command is never written back as Pending.
        assert_eq!(
            count(
                scratch.worker(),
                "SELECT count(*) FROM sync_outbox WHERE state = 'Pending'"
            ),
            0
        );
        assert_eq!(
            account_name(scratch.worker(), account),
            "Mine",
            "applied once"
        );
        assert_eq!(
            count(scratch.worker(), "SELECT count(*) FROM outbox_queue"),
            0
        );
        assert_eq!(log_len(&live), live_log, "the live vault is untouched");

        // 04b F1: the rebuilt vault reads back, so the NEXT rebase can start. The bound entry keeps
        // the command's original issue time and its complete metadata.
        let rebuilt = scratch.worker().read_local_commands().unwrap();
        assert_eq!(rebuilt.len(), 1);
        assert_eq!(rebuilt[0].meta.command_id, c.command_id);
        assert_eq!(rebuilt[0].meta.issued_at, c.issued_at);
        assert!(matches!(
            rebuilt[0].entry,
            OutboxEntryState::Bound { seq: 6, .. }
        ));
    }

    #[test]
    fn a_command_built_on_an_accepted_one_replays_and_is_not_mistaken_for_a_conflict() {
        let (dir, live, account, scratch_path) = live_with_base();
        let origin = other_device(&live, &dir);
        let first = meta();
        let mut second = meta();
        second.correlation_id = first.correlation_id;
        second.causation_id = Some(first.command_id);
        let sealed = seal(origin.worker(), &first, &rename(account, "One"));
        live.dispatch(first.clone(), rename(account, "One"))
            .unwrap();
        live.dispatch(second.clone(), rename(account, "Two"))
            .unwrap();
        let bytes = sealed.container();
        approve(&live, first.command_id, &bytes);
        let local = live.read_local_commands().unwrap();

        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        let pulled = [sealed.entry(&bytes, 6, None)];
        let (_, executed) =
            reconcile_and_replay(&mut scratch, &pulled, &local, &AcceptedEvidence::new()).unwrap();

        // The accepted command's own rows are not the server's: the second command is untouched.
        assert_eq!(executed.replayed.len(), 1);
        assert_eq!(executed.replayed[0].command_id, second.command_id);
        assert_eq!(
            count(scratch.worker(), "SELECT count(*) FROM outbox_queue"),
            0
        );
        assert_eq!(account_name(scratch.worker(), account), "Two");
        let one = outbox_row(scratch.worker(), first.command_id).unwrap();
        let two = outbox_row(scratch.worker(), second.command_id).unwrap();
        assert_eq!(
            (one.state.as_str(), two.state.as_str()),
            ("Bound", "Pending")
        );
        assert_eq!(one.result_ref, None, "no result is a NULL column");
        assert!(one.op_seq < two.op_seq);
    }

    #[test]
    fn a_bound_entry_is_remapped_when_the_tail_agrees_and_refused_when_it_does_not() {
        let (dir, live, account, scratch_path) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let cmd = rename(account, "Mine");
        let sealed = seal(origin.worker(), &c, &cmd);
        live.dispatch(c.clone(), cmd).unwrap();
        let bytes = sealed.container();
        live.lock()
            .conn
            .execute(
                "UPDATE sync_outbox SET state = 'Bound', sealed_container = ?2, bound_seq = 6,
                        result_ref = x'0a'
                  WHERE command_id = ?1",
                params![c.command_id, bytes],
            )
            .unwrap();
        let local = live.read_local_commands().unwrap();
        assert!(matches!(
            local[0].entry,
            OutboxEntryState::Bound { seq: 6, .. }
        ));

        // The tail agrees.
        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        let pulled = [sealed.entry(&bytes, 6, Some(vec![0x0a]))];
        reconcile_and_replay(&mut scratch, &pulled, &local, &AcceptedEvidence::new()).unwrap();
        let row = outbox_row(scratch.worker(), c.command_id).unwrap();
        assert_eq!((row.state.as_str(), row.bound_seq), ("Bound", Some(6)));

        // The tail puts the same envelope somewhere else: a fork-consistency failure, not a bind.
        let moved = [sealed.entry(&bytes, 9, Some(vec![0x0a]))];
        let error = reconcile(&moved, &local, &AcceptedEvidence::new()).unwrap_err();
        assert!(
            matches!(
                error,
                RebaseError::EvidenceDisagrees {
                    found: Disagreement::Position { .. },
                    ..
                }
            ),
            "{error:?}"
        );
    }

    // -- forks ------------------------------------------------------------------------------------

    #[test]
    fn the_same_command_id_with_another_envelope_is_a_protocol_fork_with_both_kept_and_nothing_applied(
    ) {
        let (dir, live, account, scratch_path) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let theirs = seal(origin.worker(), &c, &rename(account, "Theirs"));
        live.dispatch(c.clone(), rename(account, "Mine")).unwrap();
        // Sealed from the local record: the same command id, a different envelope.
        let mine = {
            let other = live
                .rollback_to(5, &dir.path().join("rebase").join("sealer.db"))
                .unwrap();
            seal(other.worker(), &c, &rename(account, "Mine"))
        };
        let local_bytes = mine.container();
        let their_bytes = theirs.container();
        approve(&live, c.command_id, &local_bytes);
        let local = live.read_local_commands().unwrap();

        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        let log_before = log_len(scratch.worker());
        let pulled = [theirs.entry(&their_bytes, 6, None)];
        let error = reconcile_and_replay(&mut scratch, &pulled, &local, &AcceptedEvidence::new())
            .unwrap_err();

        let fork = as_fork(error);
        assert_eq!(fork.source, ForkSource::Tail);
        assert_eq!(fork.command_id, c.command_id);
        let (local_kept, other_kept) = (fork.local.clone().unwrap(), fork.other.clone().unwrap());
        assert_eq!(local_kept.container, local_bytes, "our ciphertext is kept");
        assert_eq!(other_kept.container, their_bytes, "theirs is kept");
        assert_eq!(local_kept.digest, digest_of(&local_bytes));
        assert_eq!(other_kept.digest, digest_of(&their_bytes));
        assert_ne!(local_kept.digest, other_kept.digest);
        // Nothing was applied, bound or queued.
        assert_eq!(scratch.head_seq(), 5);
        assert_eq!(log_len(scratch.worker()), log_before);
        assert!(outbox_row(scratch.worker(), c.command_id).is_none());
        // AC5: the status machine halts with the command and both digests.
        let mut status = SyncStatus::Healthy;
        let event = fork.observe(&mut status);
        let evidence = HaltEvidence::ProtocolFork {
            command_id: *c.command_id.as_bytes(),
            local_digest: Some(local_kept.digest),
            other_digest: Some(other_kept.digest),
        };
        assert_eq!(event, Some(SyncEvent::Halted(evidence)));
        assert_eq!(status, SyncStatus::Halted(evidence));
        assert_eq!(fork.halt_evidence(), evidence);
    }

    #[test]
    fn a_pending_entry_whose_command_id_is_already_in_the_log_is_a_fork_with_no_local_envelope() {
        let (dir, live, account, scratch_path) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let theirs = seal(origin.worker(), &c, &rename(account, "Theirs"));
        live.dispatch(c.clone(), rename(account, "Mine")).unwrap();
        let local = live.read_local_commands().unwrap();
        assert_eq!(local[0].entry, OutboxEntryState::Pending);

        let bytes = theirs.container();
        let pulled = [theirs.entry(&bytes, 6, None)];
        let fork = as_fork(reconcile(&pulled, &local, &AcceptedEvidence::new()).unwrap_err());

        assert!(fork.local.is_none(), "it was never sealed");
        assert_eq!(fork.other.as_ref().unwrap().seq, Some(6));
        assert_eq!(
            fork.halt_evidence(),
            HaltEvidence::ProtocolFork {
                command_id: *c.command_id.as_bytes(),
                local_digest: None,
                other_digest: Some(digest_of(&bytes)),
            }
        );
        drop(scratch_path);
    }

    #[test]
    fn a_pending_entry_whose_command_id_is_in_the_log_under_another_key_is_a_fork() {
        let (dir, live, account, _scratch) = live_with_base();
        let origin = other_device(&live, &dir);
        let mine = meta();
        // The log holds the same command id, issued under another idempotency key.
        let mut theirs_meta = mine.clone();
        theirs_meta.idempotency_key = Uuid::new_v4().to_string();
        let theirs = seal(origin.worker(), &theirs_meta, &rename(account, "Theirs"));
        live.dispatch(mine.clone(), rename(account, "Mine"))
            .unwrap();
        let local = live.read_local_commands().unwrap();
        assert_eq!(local[0].entry, OutboxEntryState::Pending);

        let bytes = theirs.container();
        let pulled = [theirs.entry(&bytes, 6, None)];
        let fork = as_fork(reconcile(&pulled, &local, &AcceptedEvidence::new()).unwrap_err());
        assert!(fork.local.is_none(), "it was never sealed");
        assert_eq!(fork.other.as_ref().unwrap().seq, Some(6));
    }

    #[test]
    fn an_approved_entry_whose_sealed_container_is_another_commands_is_unreadable() {
        let (dir, live, account, _scratch) = live_with_base();
        let origin = other_device(&live, &dir);
        let (a, b) = (meta(), meta());
        live.dispatch(a.clone(), rename(account, "A")).unwrap();
        let other = seal(origin.worker(), &b, &rename(account, "B"));
        // A's entry holds B's container: its object id is not A's command id.
        approve(&live, a.command_id, &other.container());
        let local = live.read_local_commands().unwrap();
        let error = reconcile(&[], &local, &AcceptedEvidence::new()).unwrap_err();
        assert!(
            matches!(error, RebaseError::UnreadableSealed { command_id } if command_id == a.command_id),
            "{error:?}"
        );
    }

    #[test]
    fn a_new_command_id_reusing_an_idempotency_key_in_the_log_is_a_fork() {
        let (dir, live, account, _scratch) = live_with_base();
        let origin = other_device(&live, &dir);
        let a = meta();
        let theirs = seal(origin.worker(), &a, &rename(account, "Theirs"));
        let mut b = meta();
        b.idempotency_key = a.idempotency_key.clone();
        live.dispatch(b.clone(), rename(account, "Mine")).unwrap();
        let local = live.read_local_commands().unwrap();

        let bytes = theirs.container();
        let pulled = [theirs.entry(&bytes, 6, None)];
        let fork = as_fork(reconcile(&pulled, &local, &AcceptedEvidence::new()).unwrap_err());

        assert_eq!(fork.command_id, b.command_id);
        assert!(fork.local.is_none());
    }

    #[test]
    fn identical_bytes_whose_plaintext_disagrees_with_the_local_record_are_a_fork() {
        let (dir, live, account, _scratch) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let sealed = seal(origin.worker(), &c, &rename(account, "Sealed"));
        // The local record is another payload under the same command id.
        live.dispatch(c.clone(), rename(account, "Recorded"))
            .unwrap();
        let bytes = sealed.container();
        approve(&live, c.command_id, &bytes);
        let local = live.read_local_commands().unwrap();

        let pulled = [sealed.entry(&bytes, 6, None)];
        let fork = as_fork(reconcile(&pulled, &local, &AcceptedEvidence::new()).unwrap_err());

        assert_eq!(fork.local.unwrap().digest, fork.other.unwrap().digest);
    }

    #[test]
    fn a_fork_is_described_by_what_happened_and_never_logs_ciphertext() {
        let (dir, live, account, _scratch) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let theirs = seal(origin.worker(), &c, &rename(account, "Theirs"));
        live.dispatch(c.clone(), rename(account, "Mine")).unwrap();
        let local = live.read_local_commands().unwrap();
        let bytes = theirs.container();
        let pulled = [theirs.entry(&bytes, 6, None)];

        // A command this device never sealed: not "two digests differ".
        let error = reconcile(&pulled, &local, &AcceptedEvidence::new()).unwrap_err();
        let described = error.to_string();
        assert!(described.contains("never sealed"), "{described}");
        assert!(described.contains(&c.command_id.to_string()), "{described}");
        assert!(!described.contains("two different"), "{described}");

        // Debug names the digest and the length, never the container bytes.
        let debug = format!("{error:?}");
        assert!(!debug.contains("68, 70, 83, 89"), "the DFSY magic: {debug}");
        assert!(debug.contains("container_len"), "{debug}");
        assert!(debug.len() < 1200, "{} bytes of Debug", debug.len());

        // A sealed command held as another envelope says so.
        let sealed_local = {
            let other = live
                .rollback_to(5, &dir.path().join("rebase").join("sealer.db"))
                .unwrap();
            seal(other.worker(), &c, &rename(account, "Mine"))
        };
        let local_bytes = sealed_local.container();
        approve(&live, c.command_id, &local_bytes);
        let local = live.read_local_commands().unwrap();
        let error = reconcile(&pulled, &local, &AcceptedEvidence::new()).unwrap_err();
        assert!(error.to_string().contains("two different envelopes"));
    }

    /// The digest matches but one field of the local record is not the sealed command's.
    #[test]
    fn every_field_of_the_complete_meta_the_payload_and_the_schema_version_must_match() {
        type Change = fn(&mut CommandMeta);
        let changes: [(&str, Change); 6] = [
            ("actor_id", |m| m.actor_id = "someone else".to_owned()),
            ("correlation_id", |m| m.correlation_id = Uuid::now_v7()),
            ("causation_id", |m| m.causation_id = Some(Uuid::now_v7())),
            ("issued_at", |m| {
                m.issued_at = IssuedAt::from_unix_micros(m.issued_at.as_unix_micros() + 1);
            }),
            ("idempotency_key", |m| {
                m.idempotency_key = Uuid::new_v4().to_string();
            }),
            ("actor_type", |m| m.actor_type = ActorType::System),
        ];
        for (field, change) in changes {
            let (dir, live, account, _scratch) = live_with_base();
            let origin = other_device(&live, &dir);
            let sealed_meta = meta();
            let sealed = seal(origin.worker(), &sealed_meta, &rename(account, "Same"));
            let mut recorded = sealed_meta.clone();
            change(&mut recorded);
            live.dispatch(recorded, rename(account, "Same")).unwrap();
            let bytes = sealed.container();
            approve(&live, sealed_meta.command_id, &bytes);
            let local = live.read_local_commands().unwrap();
            let pulled = [sealed.entry(&bytes, 6, None)];

            let error = reconcile(&pulled, &local, &AcceptedEvidence::new()).unwrap_err();

            assert!(
                matches!(error, RebaseError::ProtocolFork(_)),
                "{field}: {error:?}"
            );
        }
        // The schema version is part of the header, not the meta.
        let (dir, live, account, _scratch) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let mut sealed = seal(origin.worker(), &c, &rename(account, "Same"));
        sealed.header.payload_schema_version += 1;
        live.dispatch(c.clone(), rename(account, "Same")).unwrap();
        let bytes = sealed.container();
        approve(&live, c.command_id, &bytes);
        let local = live.read_local_commands().unwrap();
        let pulled = [sealed.entry(&bytes, 6, None)];
        let error = reconcile(&pulled, &local, &AcceptedEvidence::new()).unwrap_err();
        assert!(matches!(error, RebaseError::ProtocolFork(_)), "{error:?}");
    }

    #[test]
    fn a_bound_entry_with_no_result_agrees_with_a_tail_that_has_none() {
        let (dir, live, account, _scratch_path) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let sealed = seal(origin.worker(), &c, &rename(account, "Mine"));
        live.dispatch(c.clone(), rename(account, "Mine")).unwrap();
        let bytes = sealed.container();
        live.lock()
            .conn
            .execute(
                "UPDATE sync_outbox SET state = 'Bound', sealed_container = ?2, bound_seq = 6,
                        result_ref = NULL
                  WHERE command_id = ?1",
                params![c.command_id, bytes],
            )
            .unwrap();
        let local = live.read_local_commands().unwrap();
        assert!(matches!(
            local[0].entry,
            OutboxEntryState::Bound {
                result_ref: None,
                ..
            }
        ));

        let pulled = [sealed.entry(&bytes, 6, None)];
        let reconciliation = reconcile(&pulled, &local, &AcceptedEvidence::new()).unwrap();

        assert_eq!(reconciliation.settled[0].by, SettledBy::Tail);
    }

    #[test]
    fn the_same_envelope_twice_in_the_log_is_a_fork() {
        let (dir, live, account, _scratch) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let sealed = seal(origin.worker(), &c, &rename(account, "Mine"));
        live.dispatch(c.clone(), rename(account, "Mine")).unwrap();
        let bytes = sealed.container();
        approve(&live, c.command_id, &bytes);
        let local = live.read_local_commands().unwrap();

        let pulled = [sealed.entry(&bytes, 6, None), sealed.entry(&bytes, 7, None)];
        let fork = as_fork(reconcile(&pulled, &local, &AcceptedEvidence::new()).unwrap_err());

        assert_eq!(fork.other.unwrap().seq, Some(7));
    }

    #[test]
    fn a_bound_entry_the_base_already_holds_stays_bound_without_the_tail() {
        let (_dir, live, _account, scratch_path, sealed, id) = live_with_accepted_command_in_base();
        live.lock()
            .conn
            .execute(
                "UPDATE sync_outbox SET state = 'Bound', bound_seq = 3, result_ref = x'05'
                  WHERE command_id = ?1",
                [id],
            )
            .unwrap();
        let local = live.read_local_commands().unwrap();
        let _ = sealed;

        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        let (reconciliation, executed) =
            reconcile_and_replay(&mut scratch, &[], &local, &AcceptedEvidence::new()).unwrap();

        assert_eq!(reconciliation.settled[0].by, SettledBy::Bound);
        assert!(executed.replayed.is_empty());
        let row = outbox_row(scratch.worker(), id).unwrap();
        assert_eq!((row.state.as_str(), row.bound_seq), ("Bound", Some(3)));
        assert_eq!(row.result_ref, Some(vec![5]));
    }

    // -- the probe --------------------------------------------------------------------------------

    #[test]
    fn a_sealed_entry_the_tail_does_not_settle_needs_a_probe_result() {
        let (dir, live, account, _scratch) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let sealed = seal(origin.worker(), &c, &rename(account, "Mine"));
        live.dispatch(c.clone(), rename(account, "Mine")).unwrap();
        approve(&live, c.command_id, &sealed.container());
        let local = live.read_local_commands().unwrap();

        let error = reconcile(&[], &local, &AcceptedEvidence::new()).unwrap_err();

        assert!(
            matches!(error, RebaseError::ProbeRequired { command_id } if command_id == c.command_id),
            "{error:?}"
        );
    }

    #[test]
    fn an_unknown_probe_lets_the_entry_re_seal_pending_with_its_container_dropped() {
        let (dir, live, account, scratch_path) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let sealed = seal(origin.worker(), &c, &rename(account, "Mine"));
        live.dispatch(c.clone(), rename(account, "Mine")).unwrap();
        let bytes = sealed.container();
        approve(&live, c.command_id, &bytes);
        let local = live.read_local_commands().unwrap();
        let index = InMemoryIndex::new();
        let mut evidence = AcceptedEvidence::new();
        evidence.record(c.command_id, probe(&index, &bytes));
        assert_eq!(evidence.probe(&c.command_id), Some(&Probe::Unknown));

        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        let (reconciliation, executed) =
            reconcile_and_replay(&mut scratch, &[], &local, &evidence).unwrap();

        assert!(reconciliation.settled.is_empty());
        assert_eq!(executed.replayed.len(), 1);
        let row = outbox_row(scratch.worker(), c.command_id).unwrap();
        assert_eq!(row.state, "Pending");
        assert!(row.sealed.is_none(), "the stale container is dropped");
        assert_eq!(
            decode_row_keys(&row.keys),
            Some(local[0].command.write_set_keys.clone()),
            "the new write-set row keys"
        );
        assert_eq!(account_name(scratch.worker(), account), "Mine");
    }

    #[test]
    fn a_sealed_entry_cannot_reach_the_replay_without_its_evidence() {
        let (dir, live, account, scratch_path) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let sealed = seal(origin.worker(), &c, &rename(account, "Mine"));
        live.dispatch(c.clone(), rename(account, "Mine")).unwrap();
        approve(&live, c.command_id, &sealed.container());
        let local = live.read_local_commands().unwrap();

        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        let error = scratch.rebase_local(&local).unwrap_err();

        assert!(
            matches!(error, RebaseError::NotReconciled { command_id } if command_id == c.command_id),
            "{error:?}"
        );
    }

    #[test]
    fn a_probe_fork_halts_with_our_envelope_kept_and_the_other_unnamed() {
        let (dir, live, account, _scratch) = live_with_base();
        let origin = other_device(&live, &dir);
        let accepted = meta();
        let first = seal(origin.worker(), &accepted, &rename(account, "First"));
        // Another command id under the same idempotency key.
        let mut reuse = meta();
        reuse.idempotency_key = accepted.idempotency_key.clone();
        // A second origin: one vault cannot apply two commands under one key.
        let origin_two = live
            .rollback_to(5, &dir.path().join("rebase").join("device-c.db"))
            .unwrap();
        let second = seal(origin_two.worker(), &reuse, &rename(account, "Second"));
        let first_bytes = first.container();
        let second_bytes = second.container();
        let mut index = InMemoryIndex::new();
        accept(&mut index, &first_bytes, 6, Some(vec![1, 2]));
        live.dispatch(reuse.clone(), rename(account, "Second"))
            .unwrap();
        approve(&live, reuse.command_id, &second_bytes);
        let local = live.read_local_commands().unwrap();
        let live_log = log_len(&live);

        // Re-pushing the exact retained container: the index holds the key under another command.
        let result = probe(&index, &second_bytes);
        assert_eq!(result, Probe::ProtocolFork);
        let mut evidence = AcceptedEvidence::new();
        evidence.record(reuse.command_id, result);
        let fork = as_fork(reconcile(&[], &local, &evidence).unwrap_err());

        assert_eq!(fork.source, ForkSource::Index);
        assert_eq!(fork.local.as_ref().unwrap().container, second_bytes);
        assert!(fork.other.is_none());
        assert_eq!(log_len(&live), live_log, "no second operation-log row");
        // The first command's own exact retry still returns its original result.
        assert_eq!(
            probe(&index, &first_bytes),
            Probe::Replay {
                seq: 6,
                result_ref: Some(vec![1, 2])
            }
        );
    }

    #[test]
    fn the_probe_and_the_tail_must_agree() {
        let (dir, live, account, _scratch) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let sealed = seal(origin.worker(), &c, &rename(account, "Mine"));
        live.dispatch(c.clone(), rename(account, "Mine")).unwrap();
        let bytes = sealed.container();
        approve(&live, c.command_id, &bytes);
        let local = live.read_local_commands().unwrap();
        let pulled = [sealed.entry(&bytes, 6, Some(vec![1]))];

        for (probe, want) in [
            (Probe::Unknown, Disagreement::ProbeUnknownButPulled),
            (Probe::ProtocolFork, Disagreement::ProbeForkButPulled),
        ] {
            let mut evidence = AcceptedEvidence::new();
            evidence.record(c.command_id, probe);
            let error = reconcile(&pulled, &local, &evidence).unwrap_err();
            assert!(
                matches!(&error, RebaseError::EvidenceDisagrees { found, .. } if *found == want),
                "{error:?}"
            );
        }
        let mut evidence = AcceptedEvidence::new();
        evidence.record(
            c.command_id,
            Probe::Replay {
                seq: 7,
                result_ref: Some(vec![1]),
            },
        );
        let error = reconcile(&pulled, &local, &evidence).unwrap_err();
        assert!(matches!(
            error,
            RebaseError::EvidenceDisagrees {
                found: Disagreement::Position { .. },
                ..
            }
        ));
        // A probe that agrees changes nothing.
        let mut evidence = AcceptedEvidence::new();
        evidence.record(
            c.command_id,
            Probe::Replay {
                seq: 6,
                result_ref: Some(vec![1]),
            },
        );
        let reconciliation = reconcile(&pulled, &local, &evidence).unwrap();
        assert_eq!(reconciliation.settled[0].by, SettledBy::Tail);
    }

    // -- the index: truncation, snapshot bootstrap, restart --------------------------------------

    /// The accepted command is not in the pulled tail; its effects are already in the base. The
    /// index's `Replay` binds it with the original position and result.
    fn bind_from_the_index(
        build: impl FnOnce(&[u8]) -> InMemoryIndex,
        seq: u64,
        result: Option<Vec<u8>>,
    ) {
        let (dir, live, account, scratch_path, sealed, id) = live_with_accepted_command_in_base();
        let bytes = sealed.container();
        let index = build(&bytes);
        let local = live.read_local_commands().unwrap();
        let old_op_seq = local[0].old_op_seq;
        let mut evidence = AcceptedEvidence::new();
        evidence.record(id, probe(&index, &bytes));
        // The tail that is left holds only a later, unrelated envelope.
        let origin = other_device(&live, &dir);
        let later = seal(origin.worker(), &meta(), &rename(account, "Later"));
        drop(origin);
        let later_bytes = later.container();
        let pulled = [later.entry(&later_bytes, 6, None)];

        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        let (reconciliation, executed) =
            reconcile_and_replay(&mut scratch, &pulled, &local, &evidence).unwrap();

        assert_eq!(reconciliation.settled[0].by, SettledBy::Index);
        assert!(executed.replayed.is_empty());
        let row = outbox_row(scratch.worker(), id).unwrap();
        assert_eq!(row.state, "Bound", "an accepted command is never Pending");
        assert_eq!(row.bound_seq, Some(i64::try_from(seq).unwrap()));
        assert_eq!(row.result_ref, result);
        assert_eq!(
            row.op_seq, old_op_seq,
            "the base already holds the command's row"
        );
        assert_eq!(
            count(
                scratch.worker(),
                "SELECT count(*) FROM sync_outbox WHERE state = 'Pending'"
            ),
            0
        );
        // One log row for the command: nothing was applied twice.
        assert_eq!(
            count(
                scratch.worker(),
                &format!(
                    "SELECT count(*) FROM operation_log WHERE command_id = x'{}'",
                    id.simple()
                )
            ),
            1
        );
        assert_eq!(account_name(scratch.worker(), account), "Later");
    }

    fn index_of(bytes: &[u8], seq: u64, result: Option<Vec<u8>>) -> InMemoryIndex {
        let mut index = InMemoryIndex::new();
        accept(&mut index, bytes, seq, result);
        index
    }

    #[test]
    fn tail_truncation_the_index_still_binds_the_accepted_command() {
        // Accepted at seq 4; the retained tail starts after the base.
        bind_from_the_index(|bytes| index_of(bytes, 4, Some(vec![9])), 4, Some(vec![9]));
    }

    #[test]
    fn a_latest_snapshot_bootstrap_binds_through_the_index_without_a_result() {
        bind_from_the_index(|bytes| index_of(bytes, 3, None), 3, None);
    }

    #[test]
    fn a_service_restart_that_rebuilds_the_index_from_its_records_answers_the_same() {
        bind_from_the_index(
            |bytes| {
                let parsed = parse_container(bytes).unwrap();
                let record =
                    PushIdentity::from_container(&parsed)
                        .unwrap()
                        .accepted(1, 4, Some(vec![5, 5]));
                // The records are what survives: encode them, "restart", decode into a new index.
                let durable = record.encode().unwrap();
                let mut restarted = InMemoryIndex::new();
                restarted
                    .insert(AcceptedRecord::decode(&durable).unwrap())
                    .unwrap();
                restarted
            },
            4,
            Some(vec![5, 5]),
        );
    }

    #[test]
    fn an_index_replay_for_a_command_the_scratch_vault_does_not_hold_binds_nothing() {
        let (dir, live, account, scratch_path) = live_with_base();
        let origin = other_device(&live, &dir);
        let c = meta();
        let sealed = seal(origin.worker(), &c, &rename(account, "Mine"));
        live.dispatch(c.clone(), rename(account, "Mine")).unwrap();
        let bytes = sealed.container();
        approve(&live, c.command_id, &bytes);
        let local = live.read_local_commands().unwrap();
        let mut evidence = AcceptedEvidence::new();
        evidence.record(
            c.command_id,
            Probe::Replay {
                seq: 8,
                result_ref: None,
            },
        );

        // Accepted at seq 8, but the pulled tail stops short of it and the base lacks it.
        let mut scratch = live.rollback_to(5, &scratch_path).unwrap();
        let error = reconcile_and_replay(&mut scratch, &[], &local, &evidence).unwrap_err();

        assert!(
            matches!(error, RebaseError::AcceptedButAbsent { seq: 8, .. }),
            "{error:?}"
        );
        assert!(outbox_row(scratch.worker(), c.command_id).is_none());
    }

    // -- the receiver checks come first ------------------------------------------------------------

    #[test]
    fn a_tampered_tag_or_commitment_fails_closed_before_any_apply_and_keeps_the_evidence() {
        let (dir, live, account, scratch_path) = live_with_base();
        let origin = other_device(&live, &dir);
        let sealed = seal(origin.worker(), &meta(), &rename(account, "Theirs"));
        let bytes = sealed.container();
        let scratch = live.rollback_to(5, &scratch_path).unwrap();
        let log_before = log_len(scratch.worker());

        let mut bad_tag = sealed.header.clone();
        let mut bad_commitment = sealed.header.clone();
        if let ObjectKind::Envelope {
            idempotency_key_tag,
            ..
        } = &mut bad_tag.object
        {
            *idempotency_key_tag = IdempotencyKeyTag::from_derived([0xee; 32]);
        }
        if let ObjectKind::Envelope {
            immutable_metadata_commitment,
            ..
        } = &mut bad_commitment.object
        {
            *immutable_metadata_commitment = MetadataCommitment::from_derived([0xdd; 32]);
        }
        for (header, want) in [
            (bad_tag, ReceiverError::TagMismatch),
            (bad_commitment, ReceiverError::MetadataCommitmentMismatch),
        ] {
            // No PulledEnvelope exists for it: the proof that the checks passed cannot be built.
            let error = check_envelope(&header, &sealed.plaintext, tag_for).unwrap_err();
            assert_eq!(error, want);
            // The replica stops at the position and keeps its digest; the ciphertext is the
            // caller's container, untouched.
            let mut status = SyncStatus::Healthy;
            let own = EngineVersion::new(ENGINE).unwrap();
            let event = status.observe_envelope(6, digest_of(&bytes), &header, &own, Err(error));
            assert!(matches!(event, Some(SyncEvent::EnvelopeInvalid(i)) if i.seq == 6));
            assert!(
                matches!(status, SyncStatus::EnvelopeInvalid(i) if i.envelope_digest == digest_of(&bytes))
            );
        }
        // Nothing was applied, reconciled or bound.
        assert_eq!(scratch.head_seq(), 5);
        assert_eq!(log_len(scratch.worker()), log_before);
        assert!(scratch.touched().is_empty());
        assert!(parse_container(&bytes).is_ok(), "the ciphertext is kept");
    }
}

// ---- the class-3 overlay: what survives a rebase that is device-local ----------------------

mod overlay {
    use super::*;
    use crate::connectors::CONNECTOR_SECRET_REMOVED_EVENT;
    use crate::rebase::{BlockCause, Executed, Preservation};
    use crate::{BackupHistoryEntry, BackupHistoryKind};
    use rusqlite::params;
    use rusqlite::types::Value;

    const SECRET_A: &str = "fake-token-a-0001";
    const SECRET_B: &str = "fake-token-b-0002";

    struct World {
        dir: TempDir,
        live: DbWorker,
        account: AccountId,
        scratch: PathBuf,
        a: Uuid,
        b: Uuid,
        batch: Uuid,
        record: Uuid,
        staged_kept: Uuid,
        staged_gone: Uuid,
    }

    fn exec(worker: &DbWorker, sql: &str, p: impl rusqlite::Params) {
        worker.lock().conn.execute(sql, p).unwrap();
    }

    fn count(worker: &DbWorker, sql: &str) -> i64 {
        worker.lock().conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn setting(worker: &DbWorker, key: &str) -> Option<String> {
        worker
            .lock()
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .ok()
    }

    fn secret(worker: &DbWorker, id: Uuid) -> Option<String> {
        worker
            .lock()
            .conn
            .query_row(
                "SELECT credential FROM connector_connection_secrets WHERE connection_id = ?1",
                [id],
                |r| r.get(0),
            )
            .ok()
    }

    fn stage(worker: &DbWorker, record: Uuid, account: AccountId, id: Uuid, n: i64) {
        exec(
            worker,
            "INSERT INTO staged_transactions
                (id, source_record_id, proposed_account_id, posted_at, amount_minor, currency,
                 normalized_merchant, description, txn_fingerprint, created_at, row_ordinal)
             VALUES (?1, ?2, ?3, '2026-06-01', ?4, 'USD', 'payee', 'payee', ?5,
                     '2026-11-04T00:00:00+00:00', ?6)",
            params![
                id,
                record,
                account.as_uuid(),
                -100 * n,
                format!("fingerprint {n}"),
                n
            ],
        );
    }

    fn source(worker: &DbWorker) -> (Uuid, Uuid) {
        let (batch, record) = (Uuid::now_v7(), Uuid::now_v7());
        exec(
            worker,
            "INSERT INTO source_batches (id, source_type, source_name, status, created_at, updated_at)
             VALUES (?1, 'other', 'fake import', 'staged', '2026-11-04T00:00:00+00:00',
                     '2026-11-04T00:00:00+00:00')",
            params![batch],
        );
        exec(
            worker,
            "INSERT INTO source_records
                (id, source_batch_id, external_id, source_hash, normalized_json,
                 parse_confidence_bps, created_at)
             VALUES (?1, ?2, 'ext', 'hash', '{}', 10000, '2026-11-04T00:00:00+00:00')",
            params![record, batch],
        );
        (batch, record)
    }

    fn job(worker: &DbWorker, kind: &str) -> Uuid {
        let id = Uuid::now_v7();
        exec(
            worker,
            "INSERT INTO durable_jobs (id, kind, cadence, max_attempts, created_at, updated_at)
             VALUES (?1, ?2, 'daily', 3, '2026-11-04T00:00:00+00:00', '2026-11-04T00:00:00+00:00')",
            params![id, kind],
        );
        id
    }

    fn receipt(worker: &DbWorker, created_at: &str) {
        worker
            .record_backup_history(&BackupHistoryEntry {
                backup_id: Uuid::now_v7(),
                vault_id: worker.vault_metadata().unwrap().vault_id,
                created_at: created_at.to_owned(),
                kind: BackupHistoryKind::Manual,
                destination: "/fake/backups/one.pcfobk".to_owned(),
                format_version: 2,
                size_bytes: 512,
                verified: true,
                error: None,
            })
            .unwrap();
    }

    fn queue_record(worker: &DbWorker, command: Uuid, evidence: &[u8]) {
        queue_record_in(worker, command, Uuid::now_v7(), evidence);
    }

    fn queue_record_in(worker: &DbWorker, command: Uuid, correlation: Uuid, evidence: &[u8]) {
        exec(
            worker,
            "INSERT INTO outbox_queue
                (command_id, correlation_id, reason, origin_node_id, queued_at, envelope,
                 audit_evidence)
             VALUES (?1, ?2, 'queue_choose', ?3, '2026-11-04T00:00:00+00:00', x'01', ?4)",
            params![command, correlation, worker.node_id, evidence],
        );
    }

    /// A live vault whose base (server seq 5) carries the device-local state of the earlier
    /// session, then moves on: every kind of class-3 row is changed, removed or added after it.
    fn world() -> World {
        world_with(|_| {})
    }

    /// [`world`], with `before_base` run on the live vault just before the base image is taken, so
    /// a test can put device-local state in the base's copy.
    fn world_with(before_base: impl FnOnce(&DbWorker)) -> World {
        let (dir, live) = worker();
        let account = AccountId::new();
        live.dispatch(meta(), liquid_account_cmd(account, Currency::Usd, 50_000))
            .unwrap();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        live.create_connector_connection(a, "simplefin", SECRET_A, Some("A"))
            .unwrap();
        live.create_connector_connection(b, "simplefin", SECRET_B, Some("B"))
            .unwrap();
        live.set_connector_attention(a, r#"{"needs":"sign-in"}"#)
            .unwrap();
        live.set_connector_attention(b, r#"{"needs":"sign-in"}"#)
            .unwrap();
        live.upsert_connector_link(a, "acct-1", Some("Checking"), Some("USD"))
            .unwrap();
        live.set_connector_link_account(a, "acct-1", Some(account.as_uuid()))
            .unwrap();
        live.advance_connector_watermarks(a, "2026-10-01", &["acct-1".to_owned()])
            .unwrap();
        let (batch, record) = source(&live);
        let (staged_kept, staged_gone) = (Uuid::now_v7(), Uuid::now_v7());
        stage(&live, record, account, staged_kept, 1);
        stage(&live, record, account, staged_gone, 2);
        live.dispatch(
            meta(),
            WriteCommand::SetUserSetting {
                key: crate::REPORTING_CURRENCY_KEY.to_owned(),
                value: "USD".to_owned(),
            },
        )
        .unwrap();
        live.set_device_setting("device.theme", "dark").unwrap();
        job(&live, "before-base");
        live.record_audit_event(&meta(), "ack_before_base").unwrap();
        receipt(&live, "2026-10-01T12:00:00Z");
        before_base(&live);
        live.initialize_base_image(5).unwrap();
        let scratch = dir.path().join("rebase").join("scratch.db");
        World {
            dir,
            live,
            account,
            scratch,
            a,
            b,
            batch,
            record,
            staged_kept,
            staged_gone,
        }
    }

    /// The device-local state moves on after the base was taken.
    fn drift(w: &World) {
        let live = &w.live;
        live.clear_connector_attention(w.b).unwrap();
        exec(
            live,
            "DELETE FROM connector_connection_secrets WHERE connection_id = ?1",
            params![w.b],
        );
        live.advance_connector_watermarks(w.a, "2026-10-15", &["acct-1".to_owned()])
            .unwrap();
        exec(
            live,
            "DELETE FROM staged_transactions WHERE id = ?1",
            params![w.staged_gone],
        );
        live.set_device_setting("device.theme", "light").unwrap();
        live.set_device_setting("device.after_base", "1").unwrap();
        exec(
            live,
            "UPDATE durable_jobs SET state = 'failed', last_error = 'x'",
            [],
        );
        job(live, "after-base");
        live.record_audit_event(&meta(), "ack_after_base").unwrap();
        live.record_audit_event(&meta(), CONNECTOR_SECRET_REMOVED_EVENT)
            .unwrap();
        receipt(live, "2026-11-01T12:00:00Z");
        queue_record(live, Uuid::now_v7(), b"{\"why\":\"earlier rebase\"}");
    }

    /// Roll back, apply the remote command as the server's envelope 6, rebase the local one.
    fn rebase<'a>(
        w: &'a World,
        local: WriteCommand,
        remote: &WriteCommand,
    ) -> (Scratch<'a>, Preservation, Executed, Uuid) {
        let issued = executor::issue(&w.live, meta(), local);
        let device_b = other_device(&w.live, &w.dir);
        let sealed = seal(device_b.worker(), &meta(), remote);
        let image = w.live.capture_preservation().unwrap();
        let mut scratch = w.live.rollback_to(5, &w.scratch).unwrap();
        scratch.apply_pulled(&[sealed.pulled(6)]).unwrap();
        let executed = scratch.rebase_local(image.local()).unwrap();
        (scratch, image, executed, issued.meta.command_id)
    }

    fn other_account() -> WriteCommand {
        liquid_account_cmd(AccountId::new(), Currency::Usd, 7_000)
    }

    fn blocked(error: RebaseError) -> crate::rebase::RebaseBlockedLocalState {
        match error {
            RebaseError::BlockedLocalState(blocked) => *blocked,
            other => panic!("expected a blocked rebase, got {other:?}"),
        }
    }

    #[test]
    fn a_successful_rebase_carries_every_stable_class_three_value_and_drops_only_the_base_inherited(
    ) {
        let w = world();
        drift(&w);
        let (mut scratch, image, executed, local_id) =
            rebase(&w, other_account(), &rename(w.account, "From device B"));
        assert_eq!(executed.replayed.len(), 1);
        let before = w.live.snapshot_bytes().unwrap();

        let report = scratch.overlay_class3(&image).unwrap();
        let s = scratch.worker();

        // A credential, a watermark: the live values, byte for byte.
        assert_eq!(secret(s, w.a).as_deref(), Some(SECRET_A));
        assert_eq!(
            count(
                s,
                "SELECT COUNT(*) FROM connector_link_cursors WHERE last_synced_on = '2026-10-15'"
            ),
            1,
            "the live watermark, not the base's"
        );
        // What the live vault removed after the base is gone; the base's copy is not resurrected.
        assert_eq!(secret(s, w.b), None);
        assert_eq!(
            count(s, "SELECT COUNT(*) FROM connector_attention"),
            1,
            "A's notice is kept, B's cleared notice is not resurrected"
        );
        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM staged_transactions WHERE id = x'{}'",
                    hex(w.staged_gone)
                )
            ),
            0
        );
        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM staged_transactions WHERE id = x'{}'",
                    hex(w.staged_kept)
                )
            ),
            1
        );
        // device.* settings, a durable job, backup history, the queue.
        assert_eq!(setting(s, "device.theme").as_deref(), Some("light"));
        assert_eq!(setting(s, "device.after_base").as_deref(), Some("1"));
        assert_eq!(count(s, "SELECT COUNT(*) FROM durable_jobs"), 2);
        assert_eq!(
            count(
                s,
                "SELECT COUNT(*) FROM durable_jobs WHERE state = 'failed'"
            ),
            1
        );
        assert_eq!(count(s, "SELECT COUNT(*) FROM backup_history"), 2);
        assert_eq!(count(s, "SELECT COUNT(*) FROM outbox_queue"), 1);
        // Audit: every stable row, before and after the base.
        for event in [
            "ack_before_base",
            "ack_after_base",
            CONNECTOR_SECRET_REMOVED_EVENT,
        ] {
            assert_eq!(
                count(
                    s,
                    &format!("SELECT COUNT(*) FROM audit_events WHERE event_type = '{event}'")
                ),
                1,
                "{event}"
            );
        }
        // The rebuilt class-1 state and the replay's own memo and outbox row are untouched.
        assert_eq!(account_name(s, w.account), "From device B");
        assert_eq!(
            count(
                s,
                "SELECT COUNT(*) FROM sync_outbox WHERE state = 'Pending'"
            ),
            1
        );
        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM command_idempotency_keys WHERE command_id = x'{}'",
                    hex(local_id)
                )
            ),
            1
        );
        // The live vault was only read.
        assert_eq!(w.live.snapshot_bytes().unwrap(), before);
        assert_eq!(
            report
                .table("connector_connection_secrets")
                .unwrap()
                .inserted,
            1
        );
        assert_eq!(report.table("staged_transactions").unwrap().removed, 2);
        assert_eq!(report.table("staged_transactions").unwrap().inserted, 1);
        assert!(w.batch != w.record);
    }

    fn hex(id: Uuid) -> String {
        id.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn a_queued_rebase_preserves_the_same_state_and_withholds_the_queued_commands_memo() {
        let w = world();
        drift(&w);
        // The local rename is touched by the server's rename: it queues.
        let (mut scratch, image, executed, local_id) = rebase(
            &w,
            rename(w.account, "Local rename"),
            &rename(w.account, "Remote rename"),
        );
        assert!(executed.replayed.is_empty());
        scratch.overlay_class3(&image).unwrap();
        let s = scratch.worker();

        assert_eq!(secret(s, w.a).as_deref(), Some(SECRET_A));
        assert_eq!(setting(s, "device.theme").as_deref(), Some("light"));
        assert_eq!(count(s, "SELECT COUNT(*) FROM backup_history"), 2);
        // The earlier record survives byte for byte and the rebase's own is added to it.
        assert_eq!(count(s, "SELECT COUNT(*) FROM outbox_queue"), 2);
        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM outbox_queue WHERE command_id = x'{}'",
                    hex(local_id)
                )
            ),
            1
        );
        // The live memo of the queued command is withheld: a retry is told it is queued.
        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM command_idempotency_keys WHERE command_id = x'{}'",
                    hex(local_id)
                )
            ),
            0
        );
        assert_eq!(account_name(s, w.account), "Remote rename");
    }

    /// A second rebase on another scratch path, for tests that need two.
    fn ready_again(w: &World) -> (Scratch<'_>, Preservation) {
        let image = w.live.capture_preservation().unwrap();
        let mut scratch = w
            .live
            .rollback_to(5, &w.dir.path().join("rebase").join("second.db"))
            .unwrap();
        scratch.rebase_local(image.local()).unwrap();
        (scratch, image)
    }

    /// A rebase of `drift`ed state, left ready for the overlay.
    fn ready(w: &World) -> (Scratch<'_>, Preservation) {
        drift(w);
        let (scratch, image, _executed, _local) =
            rebase(w, other_account(), &rename(w.account, "From device B"));
        (scratch, image)
    }

    #[test]
    fn rows_the_pulled_envelopes_wrote_survive_and_a_replay_owned_audit_row_is_not_copied() {
        let w = world();
        drift(&w);
        // Local: a timezone change (its audit row is the replay's to write). Remote: a rename.
        let (mut scratch, image, _executed, local_id) = rebase(
            &w,
            WriteCommand::SetHouseholdTimezone {
                timezone: "America/Chicago".to_owned(),
            },
            &rename(w.account, "From device B"),
        );
        // A key the base never carried, written on the scratch vault by something other than the
        // live vault: it is not dropped.
        exec(
            scratch.worker(),
            "INSERT INTO settings (key, value, updated_at) VALUES ('device.scratch_only', 'x', 't')",
            [],
        );
        let before = count(
            scratch.worker(),
            "SELECT COUNT(*) FROM audit_events WHERE event_type = 'household_timezone_changed'",
        );
        assert_eq!(before, 1, "the replay wrote its own");
        let report = scratch.overlay_class3(&image).unwrap();
        let s = scratch.worker();

        assert_eq!(setting(s, "device.scratch_only").as_deref(), Some("x"));
        assert_eq!(
            count(
                s,
                "SELECT COUNT(*) FROM audit_events WHERE event_type = 'household_timezone_changed'"
            ),
            1,
            "the live copy of the replayed command's row was withheld, not doubled"
        );
        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM audit_events WHERE command_id = x'{}'",
                    hex(local_id)
                )
            ),
            1
        );
        assert_eq!(report.table("audit_events").unwrap().withheld, 1);
        // Stable audit rows: the token-removed record is carried, never dropped or re-planned.
        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM audit_events WHERE event_type = '{CONNECTOR_SECRET_REMOVED_EVENT}'"
                )
            ),
            1
        );
    }

    #[test]
    fn an_identity_the_rebuilt_vault_lacks_blocks_the_rebase_and_overlays_nothing() {
        let w = world();
        let (scratch, image) = ready(&w);
        let live_before = w.live.snapshot_bytes().unwrap();
        // A pulled envelope removed connection A's identity outright.
        scratch
            .worker()
            .lock()
            .conn
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
             DELETE FROM connector_link_cursors; DELETE FROM connector_attention;
             DELETE FROM connector_account_links; DELETE FROM connector_connections;
             PRAGMA foreign_keys = ON;",
            )
            .unwrap();
        let mut scratch = scratch;
        let error = scratch.overlay_class3(&image).unwrap_err();

        let text = error.to_string();
        assert!(
            !text.contains(SECRET_A) && !text.contains(SECRET_B),
            "{text}"
        );
        let blocked = blocked(error);
        assert_eq!(blocked.table, "connector_connection_secrets");
        let BlockCause::MissingReference {
            column,
            target,
            rows,
        } = &blocked.cause
        else {
            panic!("{:?}", blocked.cause)
        };
        assert_eq!(
            (*column, *target, rows.count),
            ("connection_id", "connector_connections", 1)
        );
        assert!(!format!("{blocked:?}").contains(SECRET_A));
        // The live vault is untouched, the scratch vault is kept as the evidence, and nothing of
        // the overlay is in it.
        assert_eq!(w.live.snapshot_bytes().unwrap(), live_before);
        assert!(blocked.scratch.exists());
        assert_eq!(
            count(
                scratch.worker(),
                "SELECT COUNT(*) FROM connector_connection_secrets"
            ),
            0
        );
        assert_eq!(
            count(scratch.worker(), "SELECT COUNT(*) FROM backup_history"),
            1
        );
        assert_eq!(
            setting(scratch.worker(), "device.theme").as_deref(),
            Some("dark")
        );
    }

    #[test]
    fn a_stale_notice_is_dropped_and_never_blocks() {
        let w = world();
        // B's secret is gone live (drift), so only its notice names an identity.
        w.live.set_connector_attention(w.b, "{}").unwrap();
        let (mut scratch, image) = ready(&w);
        // `ready` drifted B's notice away; capture again with the notice back.
        w.live.set_connector_attention(w.b, "{}").unwrap();
        let image = {
            drop(image);
            w.live.capture_preservation().unwrap()
        };
        // The rebuilt vault lacks B's identity.
        scratch
            .worker()
            .lock()
            .conn
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 DELETE FROM connector_attention WHERE connection_id = (SELECT id FROM connector_connections WHERE display_hint = 'B');
                 DELETE FROM connector_connections WHERE display_hint = 'B';
                 PRAGMA foreign_keys = ON;",
            )
            .unwrap();
        let report = scratch.overlay_class3(&image).unwrap();
        assert_eq!(report.table("connector_attention").unwrap().stale, 1);
        assert_eq!(
            count(scratch.worker(), "SELECT COUNT(*) FROM connector_attention"),
            1,
            "A's notice stays"
        );
    }

    /// An import left half-done on the live vault after the base: its batch, its record and a staged
    /// row, none of which any envelope names.
    fn in_flight_import(w: &World) -> (Uuid, Uuid, Uuid) {
        let (batch, record) = source(&w.live);
        let staged = Uuid::now_v7();
        stage(&w.live, record, w.account, staged, 9);
        (batch, record, staged)
    }

    fn row_count(worker: &DbWorker, table: &str, id: Uuid) -> i64 {
        count(
            worker,
            &format!("SELECT COUNT(*) FROM {table} WHERE id = x'{}'", hex(id)),
        )
    }

    #[test]
    fn an_in_flight_import_is_carried_with_its_source_rows_across_a_rebase_that_pulls_others() {
        let w = world();
        let (batch, record, staged) = in_flight_import(&w);
        let (mut scratch, image) = ready(&w);
        let report = scratch.overlay_class3(&image).unwrap();
        let s = scratch.worker();
        assert_eq!(row_count(s, "source_batches", batch), 1);
        assert_eq!(row_count(s, "source_records", record), 1);
        assert_eq!(row_count(s, "staged_transactions", staged), 1);
        assert_eq!(report.table("source_batches").unwrap().inserted, 1);
        assert_eq!(report.table("source_records").unwrap().inserted, 1);
        // The ids and the bytes are the live vault's, exactly.
        let read = |worker: &DbWorker| -> (String, String) {
            worker
                .lock()
                .conn
                .query_row(
                    "SELECT r.normalized_json, b.source_name FROM source_records r
                       JOIN source_batches b ON b.id = r.source_batch_id WHERE r.id = ?1",
                    [record],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap()
        };
        assert_eq!(read(s), read(&w.live));
    }

    #[test]
    fn an_in_flight_import_commits_or_is_discarded_after_the_rebase() {
        let w = world();
        let (batch, record, staged) = in_flight_import(&w);
        let (mut scratch, image) = ready(&w);
        scratch.overlay_class3(&image).unwrap();
        let s = scratch.worker();
        // Skipping works on the carried rows.
        s.skip_staged(StagedTransactionId::from_uuid(staged))
            .unwrap();
        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM staged_transactions WHERE id = x'{}' AND commit_status = 'skipped'",
                    hex(staged)
                )
            ),
            1
        );
        // Discarding the import deletes the staging and the source rows together.
        crate::ingestion::discard_batch(&s.lock().conn, batch).unwrap();
        assert_eq!(row_count(s, "source_batches", batch), 0);
        assert_eq!(row_count(s, "source_records", record), 0);
        assert_eq!(row_count(s, "staged_transactions", staged), 0);
    }

    #[test]
    fn a_same_id_source_row_from_a_pulled_envelope_blocks_with_a_typed_cause() {
        let w = world();
        let (batch, _record, _staged) = in_flight_import(&w);
        let device_b = other_device(&w.live, &w.dir);
        let sealed = seal(
            device_b.worker(),
            &meta(),
            &WriteCommand::CreateSourceBatch {
                id: SourceBatchId::from_uuid(batch),
                source_type: "other".to_owned(),
                source_name: Some("the same file".to_owned()),
                file_fingerprint: None,
                parser_version: None,
                connector_connection_id: None,
            },
        );
        let image = w.live.capture_preservation().unwrap();
        let mut scratch = w.live.rollback_to(5, &w.scratch).unwrap();
        scratch.apply_pulled(&[sealed.pulled(6)]).unwrap();
        scratch.rebase_local(image.local()).unwrap();
        let blocked = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert_eq!(blocked.table, "source_batches");
        assert!(matches!(
            blocked.cause,
            BlockCause::SourceIdCollision { kind: "source_batches", rows } if rows.count == 1
        ));
        assert_eq!(
            count(
                scratch.worker(),
                "SELECT COUNT(*) FROM staged_transactions WHERE row_ordinal = 9"
            ),
            0,
            "nothing was overlaid"
        );
    }

    #[test]
    fn a_source_row_a_local_command_names_is_never_carried_this_way() {
        let w = world();
        // A local import written through the bus: the batch is named by an outbox command.
        let batch = Uuid::now_v7();
        let issued = executor::issue(
            &w.live,
            meta(),
            WriteCommand::CreateSourceBatch {
                id: SourceBatchId::from_uuid(batch),
                source_type: "other".to_owned(),
                source_name: Some("through the bus".to_owned()),
                file_fingerprint: None,
                parser_version: None,
                connector_connection_id: None,
            },
        );
        assert_eq!(issued.meta.command_id, issued.command.command_id);
        let record = Uuid::now_v7();
        exec(
            &w.live,
            "INSERT INTO source_records
                (id, source_batch_id, external_id, source_hash, normalized_json,
                 parse_confidence_bps, created_at)
             VALUES (?1, ?2, 'ext', 'hash-bus', '{}', 10000, 't')",
            params![record, batch],
        );
        stage(&w.live, record, w.account, Uuid::now_v7(), 8);
        let image = w.live.capture_preservation().unwrap();
        let mut scratch = w.live.rollback_to(5, &w.scratch).unwrap();
        scratch.rebase_local(image.local()).unwrap();
        // The replay wrote the batch itself; the overlay does not copy the live one over it.
        let report = scratch.overlay_class3(&image).unwrap_err();
        // The record (not named by any command) cannot be carried without a batch of its own: the
        // staged row's reference is what blocks, never a copy of a row an envelope names.
        let blocked = blocked(report);
        assert!(matches!(
            blocked.cause,
            BlockCause::MissingReference {
                target: "source_records",
                ..
            }
        ));
        assert_eq!(row_count(scratch.worker(), "source_batches", batch), 1);
    }

    #[test]
    fn a_key_the_rebase_wrote_with_other_content_is_a_collision_not_a_silent_choice() {
        let w = world();
        let (queued, correlation) = (Uuid::now_v7(), Uuid::now_v7());
        queue_record_in(&w.live, queued, correlation, b"live evidence");
        let (mut scratch, image) = ready(&w);
        queue_record_in(scratch.worker(), queued, correlation, b"another evidence");
        let blocked = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert_eq!(blocked.table, "outbox_queue");
        assert!(
            matches!(blocked.cause, BlockCause::KeyCollision { .. }),
            "{:?}",
            blocked.cause
        );
        // The same record, byte for byte, is not a collision.
        let w = world();
        queue_record_in(&w.live, queued, correlation, b"live evidence");
        let (mut scratch, image) = ready(&w);
        queue_record_in(scratch.worker(), queued, correlation, b"live evidence");
        let report = scratch.overlay_class3(&image).unwrap();
        assert_eq!(report.table("outbox_queue").unwrap().unchanged, 1);
    }

    #[test]
    fn a_memo_that_belongs_to_no_plan_and_is_not_in_the_scratch_vault_blocks() {
        let w = world();
        drift(&w);
        exec(
            &w.live,
            "INSERT INTO command_idempotency_keys
                (idempotency_key, command_id, command_type, result_ref_json, op_seq, expires_at)
             VALUES ('stray-key', ?1, 'x', '{}', 1, '2999-01-01T00:00:00+00:00')",
            params![Uuid::now_v7()],
        );
        let (mut scratch, image, _executed, _id) =
            rebase(&w, other_account(), &rename(w.account, "From device B"));
        let blocked = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert_eq!(blocked.table, "command_idempotency_keys");
        assert!(matches!(blocked.cause, BlockCause::UnplannedMemo { rows } if rows.count == 1));
    }

    #[test]
    fn a_malformed_stable_audit_row_blocks_the_rebase() {
        let w = world();
        drift(&w);
        exec(
            &w.live,
            "INSERT INTO audit_events (id, event_type, command_id, correlation_id, created_at)
             VALUES (?1, 'ack_bad', x'0102', ?2, 't')",
            params![Uuid::now_v7(), Uuid::now_v7()],
        );
        let (mut scratch, image, _executed, _id) =
            rebase(&w, other_account(), &rename(w.account, "From device B"));
        let blocked = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert!(matches!(blocked.cause, BlockCause::MalformedAudit { .. }));
    }

    #[test]
    fn schema_migration_cursor_and_outbox_disagreements_block_the_rebase() {
        // Another schema.
        let w = world();
        let (mut scratch, image) = ready(&w);
        exec(
            scratch.worker(),
            "ALTER TABLE backup_history ADD COLUMN extra TEXT",
            [],
        );
        let blocked1 = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert_eq!(blocked1.cause, BlockCause::SchemaDiffers);
        // Another migration history.
        let w = world();
        let (mut scratch, image) = ready(&w);
        exec(scratch.worker(), "DELETE FROM schema_migrations WHERE version = (SELECT MAX(version) FROM schema_migrations)", []);
        let blocked2 = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert_eq!(blocked2.cause, BlockCause::MigrationHistoryDiffers);
        // A cursor ahead of the rebuilt log.
        let w = world();
        let (mut scratch, image) = ready(&w);
        exec(scratch.worker(), "INSERT OR REPLACE INTO projection_cursors (read_model_name, last_applied_op_seq) VALUES ('probe', 999999)", []);
        let blocked3 = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert!(matches!(blocked3.cause, BlockCause::CursorAhead { rows } if rows.count == 1));
        // An outbox entry with no log row.
        let w = world();
        let (mut scratch, image) = ready(&w);
        exec(
            scratch.worker(),
            "UPDATE sync_outbox SET op_seq = 999999",
            [],
        );
        let blocked4 = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert!(matches!(
            blocked4.cause,
            BlockCause::OutboxWithoutLog { .. }
        ));
    }

    /// The Sync void ledger and applied cursor (personal-cfo-1cltt.5): a replica bootstrapped from a
    /// snapshot rooted at 4 that skips positions 2 to 3, with the base taken at seq 5.
    fn seed_ledger(live: &DbWorker) {
        exec(
            live,
            "INSERT INTO sync_voids (ordinal, void_id, first, last, origin)
             VALUES (1, zeroblob(32), 2, 3, 'seeded')",
            [],
        );
        exec(
            live,
            "INSERT INTO sync_position (singleton, snapshot_root, next_to_apply) VALUES (1, 4, 6)",
            [],
        );
    }

    fn honor(live: &DbWorker, ordinal: i64, first: i64, last: i64) {
        exec(
            live,
            "INSERT INTO sync_voids (ordinal, void_id, first, last, origin)
             VALUES (?1, randomblob(32), ?2, ?3, 'honored')",
            params![ordinal, first, last],
        );
    }

    fn ledger(worker: &DbWorker) -> Vec<(i64, i64, i64, String)> {
        let guard = worker.lock();
        let mut stmt = guard
            .conn
            .prepare("SELECT ordinal, first, last, origin FROM sync_voids ORDER BY ordinal")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn cursor(worker: &DbWorker) -> Option<(Option<i64>, i64)> {
        worker
            .lock()
            .conn
            .query_row(
                "SELECT snapshot_root, next_to_apply FROM sync_position WHERE singleton = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
    }

    #[test]
    fn a_void_honored_after_the_base_survives_the_swap_appended_in_ordinal_order() {
        let w = world_with(seed_ledger);
        drift(&w);
        honor(&w.live, 2, 7, 8);
        honor(&w.live, 3, 10, 12);
        let (mut scratch, image, _executed, _local) =
            rebase(&w, other_account(), &rename(w.account, "From device B"));
        assert_eq!(
            ledger(scratch.worker()).len(),
            1,
            "the base's copy holds only what was recorded at capture"
        );

        let report = scratch.overlay_class3(&image).unwrap();

        let want = vec![
            (1, 2, 3, "seeded".to_owned()),
            (2, 7, 8, "honored".to_owned()),
            (3, 10, 12, "honored".to_owned()),
        ];
        assert_eq!(ledger(scratch.worker()), want, "the live ledger, in order");
        assert_eq!(ledger(&w.live), want, "the live vault is only read");
        let voids = report.table("sync_voids").unwrap();
        assert_eq!((voids.inserted, voids.unchanged, voids.removed), (2, 1, 0));
    }

    #[test]
    fn the_applied_cursor_moves_forward_with_the_pull_and_is_never_lowered_to_the_live_value() {
        let w = world_with(seed_ledger);
        drift(&w);
        // The live cursor is still the base's (6); the rebase applied envelope 6 in the scratch
        // vault, so the cursor it swaps in is 7.
        let (mut scratch, image, _executed, _local) =
            rebase(&w, other_account(), &rename(w.account, "From device B"));
        assert_eq!(cursor(scratch.worker()), Some((Some(4), 7)));
        let report = scratch.overlay_class3(&image).unwrap();
        assert_eq!(cursor(scratch.worker()), Some((Some(4), 7)));
        assert_eq!(cursor(&w.live), Some((Some(4), 6)));
        let position = report.table("sync_position").unwrap();
        assert_eq!((position.inserted, position.unchanged), (0, 1));
    }

    #[test]
    fn a_live_cursor_ahead_of_the_pull_is_kept_and_the_root_is_not_touched() {
        let w = world_with(seed_ledger);
        drift(&w);
        // A void honored past the pull moved the live cursor beyond the scratch vault's.
        honor(&w.live, 2, 8, 9);
        exec(&w.live, "UPDATE sync_position SET next_to_apply = 10", []);
        let (mut scratch, image, _executed, _local) =
            rebase(&w, other_account(), &rename(w.account, "From device B"));
        assert_eq!(cursor(scratch.worker()), Some((Some(4), 7)));
        let report = scratch.overlay_class3(&image).unwrap();
        assert_eq!(cursor(scratch.worker()), Some((Some(4), 10)));
        let position = report.table("sync_position").unwrap();
        assert_eq!((position.inserted, position.unchanged), (1, 0));
    }

    #[test]
    fn a_ledger_and_cursor_armed_after_the_base_are_inserted_whole() {
        let w = world();
        drift(&w);
        seed_ledger(&w.live);
        honor(&w.live, 2, 7, 7);
        let (mut scratch, image, _executed, _local) =
            rebase(&w, other_account(), &rename(w.account, "From device B"));
        assert_eq!(cursor(scratch.worker()), None, "the base had no cursor");
        scratch.overlay_class3(&image).unwrap();
        assert_eq!(ledger(scratch.worker()), ledger(&w.live));
        assert_eq!(cursor(scratch.worker()), Some((Some(4), 6)));
    }

    #[test]
    fn a_void_honored_after_the_capture_moves_the_digest_so_the_swap_refuses_it() {
        let w = world_with(seed_ledger);
        drift(&w);
        let first = w.live.capture_preservation().unwrap();
        honor(&w.live, 2, 7, 8);
        let second = w.live.capture_preservation().unwrap();
        assert_ne!(first.digest(), second.digest());
        let third = w.live.capture_preservation().unwrap();
        exec(&w.live, "UPDATE sync_position SET next_to_apply = 9", []);
        assert_ne!(
            third.digest(),
            w.live.capture_preservation().unwrap().digest(),
            "a cursor move is a change too"
        );
    }

    #[test]
    fn two_ledgers_that_are_not_one_history_block_the_rebase_and_overlay_nothing() {
        let w = world_with(seed_ledger);
        drift(&w);
        let (mut scratch, image, _executed, _local) =
            rebase(&w, other_account(), &rename(w.account, "From device B"));
        // The scratch vault's ordinal 1 is not the live one's (a ledger that was rewritten).
        {
            let guard = scratch.worker().lock();
            guard
                .conn
                .execute_batch(
                    "DROP TRIGGER sync_voids_no_update;
                     UPDATE sync_voids SET first = 1, last = 1;",
                )
                .unwrap();
        }
        let error = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert_eq!(error.table, "sync_voids");
        assert!(matches!(error.cause, BlockCause::KeyCollision { rows } if rows.count == 1));
        assert_eq!(
            setting(scratch.worker(), "device.after_base"),
            None,
            "nothing was overlaid"
        );
    }

    #[test]
    fn a_different_snapshot_root_blocks_the_rebase() {
        let w = world_with(seed_ledger);
        drift(&w);
        let (mut scratch, image, _executed, _local) =
            rebase(&w, other_account(), &rename(w.account, "From device B"));
        {
            let guard = scratch.worker().lock();
            guard
                .conn
                .execute_batch(
                    "DROP TRIGGER sync_position_forward;
                     UPDATE sync_position SET snapshot_root = 3;",
                )
                .unwrap();
        }
        let error = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert_eq!(error.table, "sync_position");
        assert!(matches!(error.cause, BlockCause::KeyCollision { .. }));
    }

    #[test]
    fn the_counter_register_and_the_notices_are_the_live_rows_never_the_bases_older_copy() {
        let w = world_with(|live| {
            exec(
                live,
                "INSERT INTO sync_counter_register
                    (vault_id, key_epoch, fingerprint, object_type, next_counter)
                 VALUES (zeroblob(16), 1, zeroblob(32), 1, 4)",
                [],
            );
            exec(
                live,
                "INSERT INTO sync_notices (kind, batch_id, source_name, discarded_count, created_at)
                 VALUES ('import_discarded', zeroblob(16), 'before.csv', 2, 't')",
                [],
            );
        });
        drift(&w);
        // After the base: the counter advanced twice, a notice was dismissed and another arrived.
        exec(
            &w.live,
            "UPDATE sync_counter_register SET next_counter = 6",
            [],
        );
        exec(
            &w.live,
            "UPDATE sync_notices SET dismissed_at = 'later'",
            [],
        );
        exec(
            &w.live,
            "INSERT INTO sync_notices (kind, batch_id, source_name, discarded_count, created_at)
             VALUES ('import_discarded', zeroblob(16), 'after.csv', 1, 't2')",
            [],
        );
        let (mut scratch, image, _executed, _local) =
            rebase(&w, other_account(), &rename(w.account, "From device B"));
        let counter =
            |worker: &DbWorker| count(worker, "SELECT next_counter FROM sync_counter_register");
        assert_eq!(counter(scratch.worker()), 4, "the base's older copy");
        scratch.overlay_class3(&image).unwrap();
        assert_eq!(counter(scratch.worker()), 6, "a counter is never lowered");
        assert_eq!(
            count(scratch.worker(), "SELECT COUNT(*) FROM sync_notices"),
            2
        );
        assert_eq!(
            count(
                scratch.worker(),
                "SELECT COUNT(*) FROM sync_notices WHERE dismissed_at = 'later'"
            ),
            1
        );
    }

    #[test]
    fn the_overlay_runs_once_and_only_after_the_replay() {
        let w = world();
        drift(&w);
        let image = w.live.capture_preservation().unwrap();
        let mut early = w.live.rollback_to(5, &w.scratch).unwrap();
        assert!(matches!(
            early.overlay_class3(&image),
            Err(RebaseError::OverlayBeforeReplay)
        ));
        drop(early);
        let w = world();
        let (mut scratch, image) = ready(&w);
        scratch.overlay_class3(&image).unwrap();
        assert!(matches!(
            scratch.overlay_class3(&image),
            Err(RebaseError::AlreadyOverlaid)
        ));
    }

    #[test]
    fn the_capture_reads_the_plan_and_the_rows_together_and_never_prints_a_value() {
        let w = world();
        drift(&w);
        let issued = executor::issue(&w.live, meta(), other_account());
        let before = w.live.snapshot_bytes().unwrap();
        let first = w.live.capture_preservation().unwrap();
        let second = w.live.capture_preservation().unwrap();
        assert_eq!(
            w.live.snapshot_bytes().unwrap(),
            before,
            "a capture only reads"
        );
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.local().len(), 1);
        assert_eq!(first.local()[0].meta.command_id, issued.meta.command_id);
        let text = format!("{first:?}");
        assert!(
            !text.contains(SECRET_A) && !text.contains("fake-token"),
            "{text}"
        );
        // Any class-3 write changes the digest, so a controller can tell the live vault moved.
        w.live.set_device_setting("device.theme", "blue").unwrap();
        assert_ne!(
            w.live.capture_preservation().unwrap().digest(),
            first.digest()
        );
    }

    #[test]
    fn a_queued_replay_owned_audit_row_is_not_carried_and_a_user_setting_stays_replicated() {
        let w = world();
        drift(&w);
        // The live vault's replicated preference differs from the one the server's side holds.
        exec(
            &w.live,
            "UPDATE settings SET value = 'EUR' WHERE key = 'user.reporting_currency'",
            [],
        );
        // Both devices change the timezone: the local command is touched and queues; the server's
        // value, and the server's user setting, stand.
        let (mut scratch, image, executed, local_id) = rebase(
            &w,
            WriteCommand::SetHouseholdTimezone {
                timezone: "America/Chicago".to_owned(),
            },
            &WriteCommand::SetHouseholdTimezone {
                timezone: "Europe/Paris".to_owned(),
            },
        );
        assert!(executed.replayed.is_empty());
        // A user.* value the pulled side wrote differently from the live vault.
        exec(
            scratch.worker(),
            "INSERT OR REPLACE INTO settings (key, value, updated_at)
             VALUES ('user.reporting_currency', 'GBP', 't')",
            [],
        );
        // The device's identity row and Sync's own record come from the live vault.
        exec(
            scratch.worker(),
            "UPDATE vault_meta SET node_id = x'00000000000000000000000000000001'",
            [],
        );
        exec(
            scratch.worker(),
            "UPDATE sync_base SET base_seq = 1, state = 'NoBase', base_digest = NULL",
            [],
        );
        scratch.overlay_class3(&image).unwrap();
        let s = scratch.worker();

        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM audit_events WHERE command_id = x'{}'",
                    hex(local_id)
                )
            ),
            0,
            "the live row of a queued command is not carried"
        );
        assert_eq!(
            setting(s, "user.reporting_currency").as_deref(),
            Some("GBP"),
            "user.* is replicated state, not an overlay"
        );
        let node: Uuid = s
            .lock()
            .conn
            .query_row("SELECT node_id FROM vault_meta", [], |r| r.get(0))
            .unwrap();
        assert_eq!(node, w.live.node_id);
        let (live_base, scratch_base): (Vec<Value>, Vec<Value>) = {
            let read = |worker: &DbWorker| {
                worker
                    .lock()
                    .conn
                    .query_row(
                        "SELECT base_seq, state, base_digest, updated_at FROM sync_base",
                        [],
                        |r| {
                            Ok(vec![
                                r.get::<_, Value>(0)?,
                                r.get::<_, Value>(1)?,
                                r.get::<_, Value>(2)?,
                                r.get::<_, Value>(3)?,
                            ])
                        },
                    )
                    .unwrap()
            };
            (read(&w.live), read(s))
        };
        assert_eq!(live_base, scratch_base);
    }

    #[test]
    fn a_base_row_that_rebuilt_state_still_refers_to_cannot_be_dropped_and_blocks() {
        let w = world();
        let (mut scratch, image) = ready(&w);
        // A dedupe decision of the rebuilt vault names the staged row the live vault removed.
        exec(
            scratch.worker(),
            "INSERT INTO dedupe_decisions
                (id, source_batch_id, layer, staged_transaction_id, decision, reason, decided_at)
             VALUES (?1, ?2, 'transaction', ?3, 'skipped', 'fake', 't')",
            params![Uuid::now_v7(), w.batch, w.staged_gone],
        );
        let blocked = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert_eq!(blocked.table, "staged_transactions");
        assert!(matches!(
            blocked.cause,
            BlockCause::StillReferenced { rows } if rows.count == 1
        ));
        assert_eq!(
            count(scratch.worker(), "SELECT COUNT(*) FROM staged_transactions"),
            2,
            "nothing was dropped"
        );
    }

    #[test]
    fn every_class_three_table_in_the_manifest_has_a_rule_in_the_overlay() {
        const MANIFEST: &str = include_str!("../../table_classes.toml");
        let handled: std::collections::BTreeSet<&str> = crate::rebase::class_three_tables_handled()
            .into_iter()
            .collect();
        let mut class_three = std::collections::BTreeSet::new();
        for block in MANIFEST.split("[[table]]").skip(1) {
            let field = |name: &str| {
                block.lines().find_map(|line| {
                    let (key, value) = line.split_once('=')?;
                    (key.trim() == name).then(|| value.trim().trim_matches('"').to_owned())
                })
            };
            if field("class").as_deref() == Some("3")
                && field("kind").as_deref().unwrap_or("sqlite_table") == "sqlite_table"
            {
                class_three.insert(field("table").unwrap());
            }
        }
        let unhandled: Vec<_> = class_three
            .iter()
            .filter(|table| !handled.contains(table.as_str()))
            .collect();
        assert!(
            unhandled.is_empty(),
            "class-3 tables with no overlay rule: {unhandled:?}"
        );
        // The overlay names only manifest tables.
        let stray: Vec<_> = handled
            .iter()
            .filter(|table| !class_three.contains(**table))
            .collect();
        assert!(
            stray.is_empty(),
            "overlay rules for non-class-3 tables: {stray:?}"
        );
    }

    /// Roll back, apply the remote command as envelope 6, rebase the local one, carry the overlay.
    fn overlaid<'a>(
        w: &'a World,
        local: WriteCommand,
        remote: &WriteCommand,
    ) -> (Scratch<'a>, Uuid) {
        let issued = executor::issue(&w.live, meta(), local);
        let device_b = other_device(&w.live, &w.dir);
        let sealed = seal(device_b.worker(), &meta(), remote);
        let remote_id = Uuid::from_bytes(sealed.plaintext.meta.command_id);
        let image = w.live.capture_preservation().unwrap();
        let mut scratch = w.live.rollback_to(5, &w.scratch).unwrap();
        scratch.apply_pulled(&[sealed.pulled(6)]).unwrap();
        scratch.rebase_local(image.local()).unwrap();
        scratch.overlay_class3(&image).unwrap();
        assert_ne!(issued.meta.command_id, remote_id);
        (scratch, remote_id)
    }

    fn cursors(worker: &DbWorker, connection: Uuid) -> i64 {
        worker
            .lock()
            .conn
            .query_row(
                "SELECT COUNT(*) FROM connector_link_cursors WHERE connection_id = ?1",
                [connection],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn unmap(connection: Uuid) -> WriteCommand {
        WriteCommand::SetConnectorLinkAccount {
            connection_id: connection,
            external_id: "acct-1".to_owned(),
            account_id: None,
        }
    }

    #[test]
    fn a_pulled_mapping_change_clears_the_watermark_the_overlay_restored() {
        let w = world();
        drift(&w);
        let (mut scratch, _remote) = overlaid(&w, other_account(), &unmap(w.a));
        assert_eq!(
            cursors(scratch.worker(), w.a),
            1,
            "the overlay brought the live watermark back"
        );
        let report = scratch.apply_pulled_class3_effects().unwrap();
        assert_eq!(report.cursors_cleared, 1);
        assert_eq!(cursors(scratch.worker(), w.a), 0);
        // The credential and everything unrelated to the mapping stay.
        assert_eq!(secret(scratch.worker(), w.a).as_deref(), Some(SECRET_A));
    }

    #[test]
    fn a_pulled_delete_scrubs_the_restored_token_and_the_audit_row_keyed_by_the_command_survives() {
        let w = world();
        drift(&w);
        let (mut scratch, remote_id) = overlaid(
            &w,
            other_account(),
            &WriteCommand::DeleteConnectorConnection { id: w.a },
        );
        assert_eq!(
            secret(scratch.worker(), w.a).as_deref(),
            Some(SECRET_A),
            "the overlay restored the token the stripped base did not have"
        );
        let report = scratch.apply_pulled_class3_effects().unwrap();
        let s = scratch.worker();
        assert_eq!(report.connections_scrubbed, 1);
        assert_eq!(report.swept, 0, "the effect already did it");
        assert_eq!(secret(s, w.a), None);
        assert_eq!(cursors(s, w.a), 0);
        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM connector_attention WHERE connection_id = x'{}'",
                    hex(w.a)
                )
            ),
            0
        );
        // The audit row is keyed by the pulled command; the live vault's own token-removed row
        // (a stable audit row) was carried by the overlay and is still there.
        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM audit_events WHERE event_type = '{CONNECTOR_SECRET_REMOVED_EVENT}'
                      AND id = x'{}'",
                    hex(remote_id)
                )
            ),
            1
        );
        assert_eq!(
            count(
                s,
                &format!(
                    "SELECT COUNT(*) FROM audit_events WHERE event_type = '{CONNECTOR_SECRET_REMOVED_EVENT}'"
                )
            ),
            2
        );
    }

    #[test]
    fn the_effects_run_once_and_only_after_the_overlay() {
        let w = world();
        drift(&w);
        let image = w.live.capture_preservation().unwrap();
        let mut scratch = w.live.rollback_to(5, &w.scratch).unwrap();
        scratch.rebase_local(image.local()).unwrap();
        assert!(matches!(
            scratch.apply_pulled_class3_effects(),
            Err(RebaseError::EffectsBeforeOverlay)
        ));
        scratch.overlay_class3(&image).unwrap();
        scratch.apply_pulled_class3_effects().unwrap();
        assert!(matches!(
            scratch.apply_pulled_class3_effects(),
            Err(RebaseError::EffectsAlreadyApplied)
        ));
    }

    #[test]
    fn the_sweep_scrubs_a_tombstoned_connection_no_pulled_command_named() {
        let w = world();
        drift(&w);
        let (mut scratch, _remote) =
            overlaid(&w, other_account(), &rename(w.account, "From device B"));
        exec(
            scratch.worker(),
            "UPDATE connector_connections SET deleted_at = '2026-11-05T00:00:00Z' WHERE id = ?1",
            params![w.a],
        );
        let report = scratch.apply_pulled_class3_effects().unwrap();
        assert_eq!(report.swept, 1);
        assert_eq!(secret(scratch.worker(), w.a), None);
        assert_eq!(cursors(scratch.worker(), w.a), 0);
        // Idempotent: the credential is gone, so a second sweep has nothing to do.
        let guard = scratch.worker().lock();
        let tx = guard.conn.unchecked_transaction().unwrap();
        assert_eq!(crate::connectors::scrub_tombstoned_in(&tx).unwrap(), 0);
    }

    #[test]
    fn a_pulled_command_that_is_this_devices_own_accepted_one_does_not_clear_a_newer_watermark() {
        let w = world();
        drift(&w);
        let (mut scratch, remote_id) = {
            let issued = executor::issue(&w.live, meta(), other_account());
            let device_b = other_device(&w.live, &w.dir);
            let sealed = seal(device_b.worker(), &meta(), &unmap(w.a));
            let remote_id = Uuid::from_bytes(sealed.plaintext.meta.command_id);
            let image = w.live.capture_preservation().unwrap();
            let mut scratch = w.live.rollback_to(5, &w.scratch).unwrap();
            scratch.apply_pulled(&[sealed.pulled(6)]).unwrap();
            scratch.rebase_local(image.local()).unwrap();
            scratch.overlay_class3(&image).unwrap();
            assert_ne!(issued.meta.command_id, remote_id);
            (scratch, remote_id)
        };
        scratch.mark_pulled_own(remote_id);
        let report = scratch.apply_pulled_class3_effects().unwrap();
        assert_eq!(report.cursors_cleared, 0);
        assert_eq!(
            cursors(scratch.worker(), w.a),
            1,
            "the live vault already ran it; its newer watermark stands"
        );
    }

    #[test]
    fn the_classification_names_the_commands_that_touch_class_three() {
        use crate::rebase::{effect_of, Class3Effect};
        assert_eq!(
            effect_of(&unmap(Uuid::nil())),
            Class3Effect::ClearLinkCursor
        );
        assert_eq!(
            effect_of(&WriteCommand::DeleteConnectorConnection { id: Uuid::nil() }),
            Class3Effect::ScrubDeletedConnection
        );
        assert_eq!(
            effect_of(&rename(AccountId::new(), "x")),
            Class3Effect::None
        );
    }

    #[test]
    fn every_class_three_write_in_apply_has_an_effect() {
        use crate::rebase::CLASS3_APPLY_SITES;
        const MANIFEST: &str = include_str!("../../table_classes.toml");
        let mut class_three = std::collections::BTreeSet::new();
        for block in MANIFEST.split("[[table]]").skip(1) {
            let field = |name: &str| {
                block.lines().find_map(|line| {
                    let (key, value) = line.split_once('=')?;
                    (key.trim() == name).then(|| value.trim().trim_matches('"').to_owned())
                })
            };
            if field("class").as_deref() == Some("3")
                && field("kind").as_deref().unwrap_or("sqlite_table") == "sqlite_table"
            {
                class_three.insert(field("table").unwrap());
            }
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/apply");
        let mut found = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let code = source.split("#[cfg(test)]").next().unwrap();
            let words: Vec<&str> = code
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter(|w| !w.is_empty())
                .collect();
            for (i, word) in words.iter().enumerate() {
                let table = match *word {
                    "INTO" | "UPDATE" => words.get(i + 1),
                    "FROM" if i > 0 && words[i - 1] == "DELETE" => words.get(i + 1),
                    _ => None,
                };
                if let Some(table) = table {
                    if class_three.contains(*table) {
                        found.insert((
                            path.file_name().unwrap().to_str().unwrap().to_owned(),
                            (*table).to_owned(),
                        ));
                    }
                }
            }
        }
        let listed: std::collections::BTreeSet<(String, String)> = CLASS3_APPLY_SITES
            .iter()
            .map(|(file, table, _)| ((*file).to_owned(), (*table).to_owned()))
            .collect();
        assert_eq!(
            found, listed,
            "an apply arm writes a class-3 table the registry does not cover (or the list is stale)"
        );
    }

    struct Covers(Vec<crate::rebase::ClassOneBObject>);

    impl crate::rebase::ClosureView for Covers {
        fn covers(&self, object: &crate::rebase::ClassOneBObject) -> bool {
            self.0.contains(object)
        }
    }

    fn link_provenance(worker: &DbWorker, entity_type: &str, entity: Uuid, record: Uuid) {
        exec(
            worker,
            "INSERT INTO source_provenance_links
                (id, entity_type, entity_id, source_record_id, relationship, created_at)
             VALUES (?1, ?2, ?3, ?4, 'created_from', 't')",
            params![Uuid::now_v7(), entity_type, entity, record],
        );
    }

    fn rebuilt(w: &World) -> Scratch<'_> {
        let (mut scratch, image) = ready(w);
        scratch.overlay_class3(&image).unwrap();
        scratch.apply_pulled_class3_effects().unwrap();
        scratch
    }

    #[test]
    fn a_rebuilt_vault_whose_class_1b_references_all_resolve_passes_the_closure_check() {
        let w = world();
        let scratch = rebuilt(&w);
        let account = w.account.as_uuid();
        link_provenance(scratch.worker(), "account", account, w.record);
        let report = scratch
            .check_class_1b_closure(&crate::rebase::NoRetainedClosure)
            .unwrap();
        assert_eq!(report.covered_by_view, 0);
    }

    #[test]
    fn a_reference_to_a_source_record_the_vault_lacks_needs_a_rebootstrap_unless_the_closure_covers_it(
    ) {
        let w = world();
        let scratch = rebuilt(&w);
        link_provenance(scratch.worker(), "account", w.account.as_uuid(), w.record);
        scratch
            .worker()
            .lock()
            .conn
            .execute_batch("PRAGMA foreign_keys = OFF;")
            .unwrap();
        exec(
            scratch.worker(),
            "DELETE FROM source_records WHERE id = ?1",
            params![w.record],
        );
        let live_before = w.live.snapshot_bytes().unwrap();

        let error = scratch
            .check_class_1b_closure(&crate::rebase::NoRetainedClosure)
            .unwrap_err();
        let text = error.to_string();
        assert!(!text.contains(&w.record.to_string()), "{text}");
        let RebaseError::ArtifactRebootstrapRequired(evidence) = error else {
            panic!("expected a rebootstrap, got {error:?}");
        };
        assert_eq!(evidence.missing.len(), 1);
        assert_eq!(
            (evidence.missing[0].kind, evidence.missing[0].rows.count),
            ("source_record", 1)
        );
        assert!(evidence.scratch.exists(), "the evidence is kept");
        assert_eq!(w.live.snapshot_bytes().unwrap(), live_before);

        // The retained closure or the confirmed pin covers it: the check passes.
        let covered = Covers(vec![crate::rebase::ClassOneBObject::SourceRecord(w.record)]);
        let report = scratch.check_class_1b_closure(&covered).unwrap();
        assert_eq!(report.covered_by_view, 1);
    }

    #[test]
    fn a_provenance_link_to_an_entity_the_vault_lacks_is_a_missing_reference_too() {
        let w = world();
        let scratch = rebuilt(&w);
        let absent = Uuid::now_v7();
        link_provenance(scratch.worker(), "ledger_transaction", absent, w.record);
        let RebaseError::ArtifactRebootstrapRequired(evidence) = scratch
            .check_class_1b_closure(&crate::rebase::NoRetainedClosure)
            .unwrap_err()
        else {
            panic!("expected a rebootstrap");
        };
        assert_eq!(evidence.missing[0].kind, "ledger_transaction");
        assert!(evidence.describe().contains("re-bootstrap"));
    }

    #[test]
    fn a_source_record_or_parser_run_whose_batch_is_gone_is_a_missing_batch() {
        let w = world();
        let scratch = rebuilt(&w);
        scratch
            .worker()
            .lock()
            .conn
            .execute_batch("PRAGMA foreign_keys = OFF;")
            .unwrap();
        exec(
            scratch.worker(),
            "DELETE FROM source_batches WHERE id = ?1",
            params![w.batch],
        );
        let RebaseError::ArtifactRebootstrapRequired(evidence) = scratch
            .check_class_1b_closure(&crate::rebase::NoRetainedClosure)
            .unwrap_err()
        else {
            panic!("expected a rebootstrap");
        };
        assert_eq!(evidence.missing[0].kind, "source_batch");
        assert_eq!(evidence.missing[0].rows.count, 1);
    }

    #[test]
    fn a_parser_run_whose_batch_is_gone_is_a_missing_batch_on_its_own() {
        let w = world();
        let scratch = rebuilt(&w);
        let batch = Uuid::now_v7();
        exec(
            scratch.worker(),
            "INSERT INTO source_batches (id, source_type, source_name, status, created_at, updated_at)
             VALUES (?1, 'other', 'run only', 'staged', 't', 't')",
            params![batch],
        );
        exec(
            scratch.worker(),
            "INSERT INTO parser_runs
                (id, source_batch_id, parser_name, parser_version, status, started_at)
             VALUES (?1, ?2, 'p', '1', 'ok', 't')",
            params![Uuid::now_v7(), batch],
        );
        scratch
            .worker()
            .lock()
            .conn
            .execute_batch("PRAGMA foreign_keys = OFF;")
            .unwrap();
        exec(
            scratch.worker(),
            "DELETE FROM source_batches WHERE id = ?1",
            params![batch],
        );
        let RebaseError::ArtifactRebootstrapRequired(evidence) = scratch
            .check_class_1b_closure(&crate::rebase::NoRetainedClosure)
            .unwrap_err()
        else {
            panic!("expected a rebootstrap");
        };
        assert_eq!(
            (evidence.missing[0].kind, evidence.missing[0].rows.count),
            ("source_batch", 1)
        );
    }

    #[test]
    fn the_decided_rows_of_an_import_still_in_progress_travel_with_it_and_a_finished_import_does_not(
    ) {
        let w = world();
        let (batch, record, _staged) = in_flight_import(&w);
        // A second record of the same import was already skipped; the import is still in progress.
        let record2 = Uuid::now_v7();
        exec(
            &w.live,
            "INSERT INTO source_records
                (id, source_batch_id, external_id, source_hash, normalized_json,
                 parse_confidence_bps, created_at)
             VALUES (?1, ?2, 'ext2', 'hash2', '{}', 10000, 't')",
            params![record2, batch],
        );
        let skipped = Uuid::now_v7();
        stage(&w.live, record2, w.account, skipped, 10);
        exec(
            &w.live,
            "UPDATE staged_transactions SET commit_status = 'skipped' WHERE id = ?1",
            params![skipped],
        );
        let (mut scratch, image) = ready(&w);
        scratch.overlay_class3(&image).unwrap();
        assert_eq!(row_count(scratch.worker(), "source_records", record), 1);
        assert_eq!(row_count(scratch.worker(), "source_records", record2), 1);

        // An import whose every staged row is decided is not in progress: nothing of it is
        // carried, so its staged rows have nothing to point at.
        let w = world();
        let (_batch, record, staged) = in_flight_import(&w);
        exec(
            &w.live,
            "UPDATE staged_transactions SET commit_status = 'skipped' WHERE id = ?1",
            params![staged],
        );
        let (mut scratch, image) = ready(&w);
        let blocked = blocked(scratch.overlay_class3(&image).unwrap_err());
        assert!(matches!(
            blocked.cause,
            BlockCause::MissingReference {
                target: "source_records",
                ..
            }
        ));
        assert_eq!(row_count(scratch.worker(), "source_records", record), 0);
    }

    #[test]
    fn begin_swap_freezes_the_live_vault_when_it_is_unchanged_and_abort_restores_it() {
        let w = world();
        drift(&w);
        let image = w.live.capture_preservation().unwrap();
        w.live.begin_swap(&image).unwrap();
        // Frozen: nothing runs, and the intent is on disk.
        let refused = w
            .live
            .dispatch(
                meta(),
                liquid_account_cmd(AccountId::new(), Currency::Usd, 1),
            )
            .unwrap_err();
        assert!(
            matches!(refused, DbError::WorkerUnavailable(WorkerState::Swapping)),
            "{refused:?}"
        );
        assert_eq!(
            w.live
                .lock()
                .conn
                .query_row("SELECT state FROM sync_base", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "Rebasing"
        );
        w.live.abort_swap().unwrap();
        assert_eq!(
            w.live
                .lock()
                .conn
                .query_row("SELECT state FROM sync_base", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "Ready"
        );
        w.live
            .dispatch(
                meta(),
                liquid_account_cmd(AccountId::new(), Currency::Usd, 1),
            )
            .unwrap();
    }

    #[test]
    fn begin_swap_refuses_a_live_vault_that_moved_on_and_changes_nothing() {
        let w = world();
        drift(&w);
        let image = w.live.capture_preservation().unwrap();
        w.live.set_device_setting("device.theme", "blue").unwrap();
        assert!(matches!(
            w.live.begin_swap(&image),
            Err(RebaseError::LiveMovedOn)
        ));
        assert_eq!(
            w.live
                .lock()
                .conn
                .query_row("SELECT state FROM sync_base", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "Ready"
        );
        w.live
            .dispatch(
                meta(),
                liquid_account_cmd(AccountId::new(), Currency::Usd, 1),
            )
            .unwrap();
    }

    #[test]
    fn a_finished_rebase_becomes_two_hashed_files_the_new_base_without_credentials() {
        let w = world();
        drift(&w);
        let (mut scratch, image) = ready(&w);
        // Not ready until the overlay and the pulled effects ran.
        scratch.overlay_class3(&image).unwrap();
        scratch.apply_pulled_class3_effects().unwrap();
        // Defense in depth: even a credential that reached the new base copy is stripped.
        let planted = scratch.new_base_path().unwrap().to_path_buf();
        w.live
            .key
            .open_image(&planted, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .unwrap()
            .execute(
                "INSERT INTO connector_connection_secrets (connection_id, credential, updated_at)
                 VALUES (?1, 'planted-credential', 't')",
                params![w.a],
            )
            .unwrap();
        let inputs = scratch.into_swap().unwrap();

        assert_eq!(inputs.old_base_seq, 5);
        assert_eq!(inputs.head_seq, 6);
        // The files are what the hashes say, and they are closed.
        assert_eq!(
            crate::base_image::sha256_file(&inputs.scratch_path).unwrap(),
            inputs.scratch_sha256
        );
        assert_eq!(
            crate::base_image::sha256_file(&inputs.new_base_path).unwrap(),
            inputs.new_base_sha256
        );
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
        // The new base image holds no credential, but the class-1 state at the new head.
        let base = w.live.key.open_image(&inputs.new_base_path, flags).unwrap();
        let secrets: i64 = base
            .query_row(
                "SELECT COUNT(*) FROM connector_connection_secrets",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(secrets, 0);
        let renamed: String = base
            .query_row(
                "SELECT name FROM accounts WHERE name = 'From device B'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(renamed, "From device B");
        // The rebuilt vault keeps the credential, records the new base, and has the replayed outbox.
        let rebuilt = w.live.key.open_image(&inputs.scratch_path, flags).unwrap();
        let kept: i64 = rebuilt
            .query_row(
                "SELECT COUNT(*) FROM connector_connection_secrets",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1);
        let (seq, digest, state): (i64, Vec<u8>, String) = rebuilt
            .query_row(
                "SELECT base_seq, base_digest, state FROM sync_base",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (seq, digest.as_slice(), state.as_str()),
            (6, inputs.new_base_sha256.as_slice(), "Ready")
        );
        let pending: i64 = rebuilt
            .query_row(
                "SELECT COUNT(*) FROM sync_outbox WHERE state = 'Pending'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pending, 1);
    }

    #[test]
    fn a_void_honored_after_the_base_is_in_the_vault_the_swap_installs_with_the_cursor_forward() {
        // The base was taken with a ledger of one seeded void and a cursor; a void was honored
        // after it. The vault `into_swap` finishes (the file the kernel renames into place) holds
        // both ledger rows in ordinal order, the honored one byte for byte, and a cursor that did
        // not go back (bxsbz.7: a rebase never rolls an honored void back to the base's copy).
        let w = world_with(seed_ledger);
        drift(&w);
        honor(&w.live, 2, 7, 8);
        let (mut scratch, image) = ready(&w);
        scratch.overlay_class3(&image).unwrap();
        scratch.apply_pulled_class3_effects().unwrap();
        let inputs = scratch.into_swap().unwrap();

        let rebuilt = w
            .live
            .key
            .open_image(
                &inputs.scratch_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
        let rows: Vec<(i64, i64, i64, String)> = rebuilt
            .prepare("SELECT ordinal, first, last, origin FROM sync_voids ORDER BY ordinal")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(rows, ledger(&w.live));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], (2, 7, 8, "honored".to_owned()));
        let (root, next): (Option<i64>, i64) = rebuilt
            .query_row(
                "SELECT snapshot_root, next_to_apply FROM sync_position",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(root, Some(4));
        assert!(next >= 6, "the cursor never goes back: {next}");
    }

    #[test]
    fn a_rebase_that_has_not_run_to_the_end_cannot_be_swapped() {
        let w = world();
        drift(&w);
        let (scratch, _image) = ready(&w);
        assert!(matches!(
            scratch.into_swap(),
            Err(RebaseError::NotReadyToSwap)
        ));
        let (mut scratch, image) = ready_again(&w);
        scratch.overlay_class3(&image).unwrap();
        assert!(matches!(
            scratch.into_swap(),
            Err(RebaseError::NotReadyToSwap)
        ));
    }
}
