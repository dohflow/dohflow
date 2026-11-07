//! The SQLite counter register and the persisted genesis (personal-cfo-1cltt.4, part A).
//!
//! The register passes the same conformance suite as the in-memory store of `vault-crypto`, and the
//! freshness cases of `vault-format.md` C.4 and C.6 rule 5 are run against the states a reopened
//! (and a restored) vault actually holds.

use std::path::Path;

use db_worker::sync_genesis::{self, GenesisState, GenesisStoreError};
use db_worker::sync_register::{self, SqliteRegisterStore};
use db_worker::DbWorker;
use platform_mock::MockWitness;
use platform_traits::{CounterContext, RollbackWitness, VerifiedMark};
use vault_crypto::sync_counter::{
    conformance, CounterRegister, LostReason, NonceError, RegisterStore, RegisterStoreError,
    SavedSealed, VerifiedStateError, VerifiedStateGuard,
};
use vault_crypto::sync_keys::DeviceKeys;
use vault_crypto::sync_records::{
    create_genesis, create_genesis_with, fresh_epoch_key, fresh_tag_secret, GenesisRecords,
};

const KEY: &str = "correct horse battery staple";
const VAULT: [u8; 16] = [0x11; 16];
const NOW: &str = "2026-11-06T00:00:00+00:00";

fn open(path: &Path) -> DbWorker {
    DbWorker::open(path, KEY).unwrap()
}

fn context(epoch: u64, fingerprint: u8) -> CounterContext {
    CounterContext {
        vault_id: VAULT,
        key_epoch: epoch,
        fingerprint: [fingerprint; 32],
        object_type: 2,
    }
}

fn saved(counter: u64) -> SavedSealed {
    SavedSealed {
        object_id: [0x33; 16],
        base_seq: 41,
        plaintext_sha256: [0x44; 32],
        container_sha256: [0x55; 32],
        container_len: 4096,
        counter,
    }
}

fn genesis_records() -> GenesisRecords {
    let keys = DeviceKeys::generate().unwrap();
    create_genesis(VAULT, [0xa1; 16], &keys, 1_700_000_000).unwrap()
}

// ---- the counter register ----------------------------------------------------------------------

#[test]
fn the_sqlite_register_passes_the_conformance_suite() {
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.db"));
    let witness = MockWitness::new();
    conformance::run(&|| SqliteRegisterStore::new(&worker), &witness);
}

#[test]
fn an_advance_and_a_saved_snapshot_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let ctx = context(1, 0xa1);
    {
        let worker = open(&path);
        let store = SqliteRegisterStore::new(&worker);
        store.commit_advance(&ctx, None, 1).unwrap();
        store.commit_advance(&ctx, Some(1), 2).unwrap();
        store.set_saved(&ctx, Some(&saved(1))).unwrap();
    }
    let worker = open(&path);
    let store = SqliteRegisterStore::new(&worker);
    assert_eq!(store.next_counter(&ctx), Ok(Some(2)));
    assert_eq!(store.saved(&ctx), Ok(Some(saved(1))));
    assert_eq!(
        store.commit_advance(&ctx, Some(1), 2),
        Err(RegisterStoreError::Conflict),
        "a stale writer after the restart still loses"
    );
}

#[test]
fn an_advance_commits_or_rolls_back_with_the_transaction_that_holds_it() {
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.db"));
    let ctx = context(1, 0xa1);

    // The command engine's shape: the advance beside another write, one transaction. A failure
    // after the advance takes both back.
    let outcome: Result<(), RegisterStoreError> = worker.write_sync_state(|tx| {
        sync_register::commit_advance(tx, &ctx, None, 1)?;
        sync_register::set_saved(tx, &ctx, Some(&saved(1)))?;
        Err(RegisterStoreError::Unavailable)
    });
    assert_eq!(outcome, Err(RegisterStoreError::Unavailable));
    let store = SqliteRegisterStore::new(&worker);
    assert_eq!(
        store.next_counter(&ctx),
        Ok(None),
        "the advance rolled back"
    );
    assert_eq!(store.saved(&ctx), Ok(None), "and the write beside it");

    worker
        .write_sync_state(|tx| -> Result<(), RegisterStoreError> {
            sync_register::commit_advance(tx, &ctx, None, 1)?;
            sync_register::set_saved(tx, &ctx, Some(&saved(1)))
        })
        .unwrap();
    assert_eq!(store.next_counter(&ctx), Ok(Some(1)));
    assert_eq!(store.saved(&ctx), Ok(Some(saved(1))));
}

#[test]
fn a_restored_vault_has_a_register_behind_the_witness() {
    let dir = tempfile::tempdir().unwrap();
    let live = dir.path().join("live.db");
    let restored = dir.path().join("restored.db");
    let ctx = context(1, 0xa1);
    let witness = MockWitness::new();

    let worker = open(&live);
    {
        let store = SqliteRegisterStore::new(&worker);
        let register = CounterRegister::new(&store, &witness);
        drop(register.reserve(&ctx).unwrap());
    }
    // The backup is taken here, with the register at 1 ...
    std::fs::write(&restored, worker.snapshot_bytes().unwrap()).unwrap();
    {
        let store = SqliteRegisterStore::new(&worker);
        let register = CounterRegister::new(&store, &witness);
        drop(register.reserve(&ctx).unwrap());
        drop(register.reserve(&ctx).unwrap());
    }
    drop(worker);

    // ... and restoring it puts the register back at 1 while the witness, which a restore does
    // not touch, is at 3.
    let worker = open(&restored);
    let store = SqliteRegisterStore::new(&worker);
    assert_eq!(store.next_counter(&ctx), Ok(Some(1)));
    assert_eq!(witness.counter(&ctx), Ok(Some(3)));
    assert_eq!(
        CounterRegister::new(&store, &witness).check(&ctx),
        Err(NonceError::NonceStateLost(
            LostReason::RegisterBehindWitness
        ))
    );
}

#[test]
fn the_register_freshness_cases_are_judged_against_the_witness() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");

    // No entry, no witness entry: fresh. No entry and a witness of exactly 0: fresh.
    let (a, b) = (context(1, 0xa1), context(1, 0xb1));
    let worker = open(&path);
    let store = SqliteRegisterStore::new(&worker);
    let witness = MockWitness::new();
    let register = CounterRegister::new(&store, &witness);
    assert_eq!(register.check(&a), Ok(None));
    witness.advance_counter(&a, 0).unwrap();
    assert_eq!(register.check(&a), Ok(None));

    // A witness of 1 or more and no register entry: lost.
    witness.advance_counter(&b, 1).unwrap();
    assert_eq!(
        register.check(&b),
        Err(NonceError::NonceStateLost(LostReason::RegisterMissing))
    );

    // A register entry and no witness entry: lost.
    let c = context(2, 0xc1);
    store.commit_advance(&c, None, 1).unwrap();
    assert_eq!(
        register.check(&c),
        Err(NonceError::NonceStateLost(LostReason::WitnessMissing))
    );

    // A register entry below the witness: lost. At or above it: consistent, and reconcile raises
    // the witness (a crash between the register advance and the witness advance).
    let d = context(3, 0xd1);
    store.commit_advance(&d, None, 2).unwrap();
    witness.advance_counter(&d, 3).unwrap();
    assert_eq!(
        register.check(&d),
        Err(NonceError::NonceStateLost(
            LostReason::RegisterBehindWitness
        ))
    );
    let e = context(4, 0xe1);
    store.commit_advance(&e, None, 2).unwrap();
    witness.advance_counter(&e, 1).unwrap();
    assert_eq!(register.check(&e), Ok(Some(2)));
    assert_eq!(register.reconcile(&e), Ok(Some(2)));
    assert_eq!(witness.counter(&e), Ok(Some(2)));

    // A register entry is never 0, in any store.
    assert_eq!(
        store.commit_advance(&context(5, 0xf1), None, 0),
        Err(RegisterStoreError::BadAdvance)
    );
    assert_eq!(store.next_counter(&context(5, 0xf1)), Ok(None));
}

// ---- the persisted genesis ---------------------------------------------------------------------

fn persist(worker: &DbWorker, records: &GenesisRecords) -> Result<GenesisState, GenesisStoreError> {
    worker.write_sync_state(|tx| {
        sync_genesis::persist(tx, &VAULT, &records.membership, &records.epoch, NOW)
    })
}

fn advance(worker: &DbWorker, to: GenesisState) -> Result<GenesisState, GenesisStoreError> {
    worker.write_sync_state(|tx| sync_genesis::advance(tx, to, NOW))
}

fn load(worker: &DbWorker) -> Option<sync_genesis::PersistedGenesis> {
    worker
        .write_sync_state(|tx| sync_genesis::load(tx))
        .unwrap()
}

#[test]
fn a_vault_holds_one_genesis_and_a_retry_lands_on_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let first = genesis_records();
    {
        let worker = open(&path);
        assert!(load(&worker).is_none());
        assert_eq!(persist(&worker, &first), Ok(GenesisState::Persisted));
    }
    // After a restart the same bytes are idempotent, another genesis is refused, and the records
    // come back exactly as they were written.
    let worker = open(&path);
    assert_eq!(persist(&worker, &first), Ok(GenesisState::Persisted));
    assert_eq!(
        persist(&worker, &genesis_records()),
        Err(GenesisStoreError::Conflict),
        "a vault that holds a genesis never mints a second"
    );
    let held = load(&worker).unwrap();
    assert_eq!(held.vault_id, VAULT);
    assert_eq!(held.membership_record, first.membership);
    assert_eq!(held.epoch_record, first.epoch);
    assert_eq!(held.state, GenesisState::Persisted);
}

#[test]
fn the_genesis_state_only_moves_forward() {
    use GenesisState::{Installed, Persisted, Published, VaultKnown};
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.db"));
    assert_eq!(
        advance(&worker, Installed),
        Err(GenesisStoreError::NotPersisted)
    );
    persist(&worker, &genesis_records()).unwrap();

    // A retried step is a no-op, and a step cannot be skipped to a state it does not reach.
    assert_eq!(advance(&worker, Persisted), Ok(Persisted));
    assert_eq!(
        advance(&worker, Published),
        Err(GenesisStoreError::BadTransition {
            from: Persisted,
            to: Published
        })
    );
    assert_eq!(advance(&worker, Installed), Ok(Installed));
    assert_eq!(advance(&worker, Installed), Ok(Installed));
    assert_eq!(
        advance(&worker, Persisted),
        Err(GenesisStoreError::BadTransition {
            from: Installed,
            to: Persisted
        })
    );
    assert_eq!(advance(&worker, Published), Ok(Published));
    assert_eq!(
        advance(&worker, VaultKnown),
        Err(GenesisStoreError::BadTransition {
            from: Published,
            to: VaultKnown
        }),
        "a published genesis is not refused afterwards"
    );
    assert_eq!(load(&worker).unwrap().state, Published);
}

#[test]
fn a_genesis_the_service_refused_is_terminal() {
    use GenesisState::{Installed, Persisted, Published, VaultKnown};
    for before_refusal in [Persisted, Installed] {
        let dir = tempfile::tempdir().unwrap();
        let worker = open(&dir.path().join("v.db"));
        persist(&worker, &genesis_records()).unwrap();
        if before_refusal == Installed {
            advance(&worker, Installed).unwrap();
        }
        assert_eq!(advance(&worker, VaultKnown), Ok(VaultKnown));
        assert_eq!(advance(&worker, VaultKnown), Ok(VaultKnown));
        for to in [Persisted, Installed, Published] {
            assert_eq!(
                advance(&worker, to),
                Err(GenesisStoreError::BadTransition {
                    from: VaultKnown,
                    to
                })
            );
        }
        assert_eq!(load(&worker).unwrap().state, VaultKnown);
    }
}

#[test]
fn a_genesis_whose_snapshot_the_service_holds_is_terminal_across_a_restart() {
    use GenesisState::{GenesisResumeLost, Installed, Persisted, Published, VaultKnown};
    // Entered from Persisted or Installed, and from nowhere else.
    for before in [Persisted, Installed] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.db");
        let records = genesis_records();
        {
            let worker = open(&path);
            persist(&worker, &records).unwrap();
            if before == Installed {
                advance(&worker, Installed).unwrap();
            }
            assert_eq!(advance(&worker, GenesisResumeLost), Ok(GenesisResumeLost));
            assert_eq!(advance(&worker, GenesisResumeLost), Ok(GenesisResumeLost));
        }
        // After a restart it is still there, the same records persist to it, and nothing leaves it.
        let worker = open(&path);
        assert_eq!(load(&worker).unwrap().state, GenesisResumeLost);
        assert_eq!(persist(&worker, &records), Ok(GenesisResumeLost));
        for to in [Persisted, Installed, Published, VaultKnown] {
            assert_eq!(
                advance(&worker, to),
                Err(GenesisStoreError::BadTransition {
                    from: GenesisResumeLost,
                    to
                })
            );
        }
    }

    // A published genesis never becomes ResumeLost, and a VaultKnown one does not either.
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.db"));
    persist(&worker, &genesis_records()).unwrap();
    advance(&worker, Installed).unwrap();
    advance(&worker, Published).unwrap();
    assert_eq!(
        advance(&worker, GenesisResumeLost),
        Err(GenesisStoreError::BadTransition {
            from: Published,
            to: GenesisResumeLost
        })
    );
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.db"));
    persist(&worker, &genesis_records()).unwrap();
    advance(&worker, VaultKnown).unwrap();
    assert_eq!(
        advance(&worker, GenesisResumeLost),
        Err(GenesisStoreError::BadTransition {
            from: VaultKnown,
            to: GenesisResumeLost
        })
    );
}

#[test]
fn the_genesis_table_refuses_anything_but_a_singleton_of_a_known_state() {
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.db"));
    let records = genesis_records();
    let conn = worker.read_connection().unwrap();
    let insert = |singleton: i64, state: &str| {
        conn.execute(
            "INSERT INTO sync_genesis
                 (singleton, vault_id, membership_record, epoch_record, state, persisted_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 't', 't')",
            rusqlite::params![singleton, VAULT, records.membership, records.epoch, state],
        )
    };
    assert!(insert(2, "Persisted").is_err(), "one row only");
    assert!(insert(1, "Done").is_err(), "a known state only");
    assert!(
        insert(1, "GenesisResumeLost").is_ok(),
        "the terminal state is storable"
    );
}

// ---- the verified state's freshness ------------------------------------------------------------

fn loaded(worker: &DbWorker) -> Option<VerifiedMark> {
    worker
        .write_sync_state(|tx| sync_genesis::loaded_verified_mark(tx))
        .unwrap()
}

fn check(worker: &DbWorker, witness: &MockWitness) -> Result<(), VerifiedStateError> {
    VerifiedStateGuard::new(witness).check(&VAULT, loaded(worker).as_ref())
}

#[test]
fn the_verified_state_is_born_in_the_witness_before_it_is_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.db");
    let witness = MockWitness::new();
    let guard = VerifiedStateGuard::new(&witness);
    let records = genesis_records();

    // Nothing has begun: fresh.
    {
        let worker = open(&path);
        assert_eq!(loaded(&worker), None);
        assert_eq!(check(&worker, &witness), Ok(()));
        // Step 0: the birth mark, then nothing persisted yet. A crash here restarts fresh.
        guard.begin(&VAULT).unwrap();
    }
    {
        let worker = open(&path);
        assert_eq!(loaded(&worker), None);
        assert_eq!(check(&worker, &witness), Ok(()), "begun, nothing persisted");
        // Step 1: the first persist. A crash here, before the mark is recorded, is the window.
        persist(&worker, &records).unwrap();
    }
    let worker = open(&path);
    let mark = loaded(&worker).expect("the persisted genesis yields a mark");
    assert_eq!(mark.membership_version, 1);
    assert_eq!(mark.key_epoch, 1);
    assert_eq!(mark.positioned_records, 1);
    assert_eq!(
        witness.verified_mark(&VAULT),
        Ok(Some(VerifiedMark::BIRTH)),
        "only the birth mark so far"
    );
    assert_eq!(
        check(&worker, &witness),
        Ok(()),
        "ahead of the birth mark is the crash window, not a restore"
    );
    // Start-up reconciliation records the state's mark; the next start is consistent.
    guard.reconcile(&VAULT, Some(&mark)).unwrap();
    assert_eq!(witness.verified_mark(&VAULT), Ok(Some(mark)));
    assert_eq!(check(&worker, &witness), Ok(()));
}

#[test]
fn a_restored_or_stripped_verified_state_is_rolled_back() {
    let dir = tempfile::tempdir().unwrap();
    let records = genesis_records();

    // Persisted state the witness never saw: rolled back.
    let worker = open(&dir.path().join("a.db"));
    persist(&worker, &records).unwrap();
    assert_eq!(
        check(&worker, &MockWitness::new()),
        Err(VerifiedStateError::StateRolledBack)
    );

    // The vault lost state the witness saw (a restore from before the genesis): rolled back.
    let seen = MockWitness::new();
    let mark = loaded(&worker).unwrap();
    VerifiedStateGuard::new(&seen)
        .record(&VAULT, &mark)
        .unwrap();
    let empty = open(&dir.path().join("b.db"));
    assert_eq!(loaded(&empty), None);
    assert_eq!(
        check(&empty, &seen),
        Err(VerifiedStateError::StateRolledBack)
    );

    // A witness that has seen a newer state than the vault holds: rolled back.
    let ahead = MockWitness::new();
    ahead
        .advance_verified_mark(
            &VAULT,
            &VerifiedMark {
                membership_version: 2,
                positioned_records: 2,
                ..mark
            },
        )
        .unwrap();
    assert_eq!(
        check(&worker, &ahead),
        Err(VerifiedStateError::StateRolledBack)
    );

    // Equal to the witness: consistent.
    assert_eq!(check(&worker, &seen), Ok(()));
}

#[test]
fn records_damaged_after_the_write_install_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.db"));
    let records = genesis_records();
    persist(&worker, &records).unwrap();
    assert!(loaded(&worker).is_some(), "the true records verify");

    // Damage after the write (a bad disk, an edit outside this module): the verification at load
    // still refuses it.
    let mut torn = records.epoch.clone();
    let last = torn.len() - 1;
    torn[last] ^= 0x01;
    worker
        .read_connection()
        .unwrap()
        .execute("UPDATE sync_genesis SET epoch_record = ?1", [torn])
        .unwrap();
    assert_eq!(
        worker.write_sync_state(|tx| sync_genesis::loaded_verified_mark(tx)),
        Err::<Option<VerifiedMark>, _>(GenesisStoreError::Corrupt)
    );
}

#[test]
fn a_torn_record_and_another_vaults_records_are_refused_at_persist_and_write_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.db"));
    let records = genesis_records();

    let mut torn = records.epoch.clone();
    let last = torn.len() - 1;
    torn[last] ^= 0x01;
    let persist_bytes = |vault: &[u8; 16], membership: &[u8], epoch: &[u8]| {
        worker.write_sync_state(|tx| sync_genesis::persist(tx, vault, membership, epoch, NOW))
    };
    assert_eq!(
        persist_bytes(&VAULT, &records.membership, &torn),
        Err(GenesisStoreError::Invalid),
        "a torn epoch record"
    );
    assert_eq!(
        persist_bytes(&VAULT, &torn, &records.epoch),
        Err(GenesisStoreError::Invalid),
        "a torn membership record"
    );
    assert_eq!(
        persist_bytes(&[0x22; 16], &records.membership, &records.epoch),
        Err(GenesisStoreError::Invalid),
        "one vault's valid records filed under another vault"
    );
    assert_eq!(
        persist_bytes(&VAULT, &[], &[]),
        Err(GenesisStoreError::Invalid),
        "nothing"
    );
    assert!(load(&worker).is_none(), "none of them wrote a row");

    // And the true records still persist, and advance, afterwards.
    assert_eq!(persist(&worker, &records), Ok(GenesisState::Persisted));
    assert!(loaded(&worker).is_some());
}

#[test]
fn a_retried_persist_reports_how_far_enabling_got_and_different_records_conflict() {
    use GenesisState::{Installed, VaultKnown};
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.db"));
    let keys = DeviceKeys::generate().unwrap();
    let first = create_genesis_with(
        VAULT,
        [0xa1; 16],
        &keys,
        1_700_000_000,
        &fresh_epoch_key(VAULT, 1).unwrap(),
        &fresh_tag_secret().unwrap(),
    )
    .unwrap();
    persist(&worker, &first).unwrap();
    advance(&worker, Installed).unwrap();
    assert_eq!(
        persist(&worker, &first),
        Ok(Installed),
        "an installed genesis stays installed"
    );

    // The same device and the same membership record, a different (valid) epoch record: not this
    // genesis.
    let second = create_genesis_with(
        VAULT,
        [0xa1; 16],
        &keys,
        1_700_000_000,
        &fresh_epoch_key(VAULT, 1).unwrap(),
        &fresh_tag_secret().unwrap(),
    )
    .unwrap();
    assert_eq!(
        second.membership, first.membership,
        "only the epoch record differs"
    );
    assert_ne!(second.epoch, first.epoch);
    assert_eq!(persist(&worker, &second), Err(GenesisStoreError::Conflict));

    // The same records filed under another vault are not this genesis either. (They are refused
    // as invalid when nothing is held; with a genesis held, the comparison comes first.)
    assert_eq!(
        worker.write_sync_state(|tx| {
            sync_genesis::persist(tx, &[0x22; 16], &first.membership, &first.epoch, NOW)
        }),
        Err(GenesisStoreError::Conflict)
    );

    advance(&worker, VaultKnown).unwrap();
    assert_eq!(
        persist(&worker, &first),
        Ok(VaultKnown),
        "a refused genesis stays refused"
    );
}
