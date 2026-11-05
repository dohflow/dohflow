//! The AEAD counter register and its anti-rollback state machine (vault-format.md C.4 "Counter
//! register and nonce state", C.6 freshness rule 5; personal-cfo-1cltt.3): the write order, the
//! comparison rule, the cap, fresh contexts, the verified-state guard, and the guards that keep
//! the keychain and every other stand-in from being the witness.

mod common;

use common::register::MemoryRegisterStore;
use platform_mock::MockWitness;
use platform_traits::{CounterContext, Progress, RollbackWitness, VerifiedMark, WitnessError};
use sync_core::header::MAX_INVOCATION_COUNTER;
use vault_crypto::sync_counter::{
    conformance, verified_mark_of, CounterRegister, LostReason, NonceError, RegisterStore,
    RegisterStoreError, VerifiedStateError, VerifiedStateGuard,
};

fn context(epoch: u64, fingerprint: u8) -> CounterContext {
    CounterContext {
        vault_id: [0x11; 16],
        key_epoch: epoch,
        fingerprint: [fingerprint; 32],
        object_type: 2,
    }
}

fn setup() -> (MemoryRegisterStore, MockWitness) {
    (MemoryRegisterStore::new(), MockWitness::new())
}

// ---- the conformance suite ---------------------------------------------------------------------

#[test]
fn the_in_memory_store_passes_the_register_store_conformance_suite() {
    let store = MemoryRegisterStore::new();
    conformance::run(&|| store.clone(), &MockWitness::new());
}

// ---- reserve: the order and the values ---------------------------------------------------------

#[test]
fn counters_count_from_zero_and_are_never_reissued() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let c = context(1, 0xa1);
    let mut seen = Vec::new();
    for expected in 0..10_u64 {
        let reservation = register.reserve(&c).unwrap();
        assert_eq!(reservation.counter(), expected);
        assert_eq!(reservation.context(), &c);
        seen.push(reservation.counter());
    }
    assert_eq!(store.next_counter(&c), Ok(Some(10)));
    assert_eq!(witness.counter(&c), Ok(Some(10)));
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 10);
}

#[test]
fn the_advance_is_committed_before_the_counter_is_handed_out() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let c = context(1, 0xa1);
    let reservation = register.reserve(&c).unwrap();
    // By the time the caller holds counter 0, the register already says the next one is 1.
    assert_eq!(reservation.counter(), 0);
    assert_eq!(store.next_counter(&c), Ok(Some(1)));
    assert_eq!(witness.peek_counter(&c), Some(1));
}

#[test]
fn a_failed_register_commit_hands_out_nothing_and_burns_nothing() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let c = context(1, 0xa1);
    assert_eq!(register.reserve(&c).unwrap().counter(), 0);
    store.fail_next_commits(1);
    assert_eq!(
        register.reserve(&c).unwrap_err(),
        NonceError::Register(RegisterStoreError::Unavailable)
    );
    assert_eq!(store.next_counter(&c), Ok(Some(1)), "register unchanged");
    assert_eq!(witness.peek_counter(&c), Some(1), "witness unchanged");
    assert_eq!(register.reserve(&c).unwrap().counter(), 1);
}

#[test]
fn a_failed_witness_advance_leaves_the_counter_burned_and_hands_out_nothing() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let c = context(1, 0xa1);
    assert_eq!(register.reserve(&c).unwrap().counter(), 0);
    witness.fail_next_counter_advances(1);
    assert_eq!(
        register.reserve(&c).unwrap_err(),
        NonceError::Witness(WitnessError::Unavailable)
    );
    // Step 1 ran: counter 1 is spent. The witness still says 1, the register says 2.
    assert_eq!(store.next_counter(&c), Ok(Some(2)));
    assert_eq!(witness.peek_counter(&c), Some(1));
    // The state is consistent (register ahead of witness), and the next counter is 2: 1 is never
    // handed out.
    assert_eq!(register.check(&c), Ok(Some(2)));
    assert_eq!(register.reserve(&c).unwrap().counter(), 2);
    assert_eq!(witness.peek_counter(&c), Some(3));
}

#[test]
fn an_unavailable_witness_refuses_to_reserve_and_never_falls_back() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let c = context(1, 0xa1);
    witness.set_unavailable(true);
    assert_eq!(
        register.reserve(&c).unwrap_err(),
        NonceError::Witness(WitnessError::Unavailable)
    );
    assert_eq!(store.next_counter(&c), Ok(None), "nothing was committed");
    assert_eq!(store.commits(), 0);
}

// ---- crash points ------------------------------------------------------------------------------

#[test]
fn a_crash_after_the_register_commit_and_before_the_witness_advance_is_consistent_on_restart() {
    let (store, witness) = setup();
    let c = context(1, 0xa1);
    // Counter 0 reserved normally.
    drop(CounterRegister::new(&store, &witness).reserve(&c).unwrap());
    // Counter 1: the register commits, the process dies before the witness advance.
    store.commit_advance(&c, Some(1), 2).unwrap();
    // Restart: the register is ahead of the witness, which is consistent, and reconcile raises it.
    let register = CounterRegister::new(&store, &witness);
    assert_eq!(register.check(&c), Ok(Some(2)));
    assert_eq!(register.reconcile(&c), Ok(Some(2)));
    assert_eq!(witness.peek_counter(&c), Some(2));
    // Counter 1 was lost to the crash; it is never reissued.
    assert_eq!(register.reserve(&c).unwrap().counter(), 2);
}

#[test]
fn a_crash_before_the_first_commit_of_a_new_context_is_not_a_loss() {
    let (store, witness) = setup();
    let c = context(1, 0xa1);
    // The context is born in the witness (0), then the process dies before the register commit.
    witness.advance_counter(&c, 0).unwrap();
    let register = CounterRegister::new(&store, &witness);
    assert_eq!(register.check(&c), Ok(None));
    assert_eq!(register.reserve(&c).unwrap().counter(), 0);
    // And a crash after the first register commit but before the first witness advance is the
    // ordinary register-ahead case, not a missing witness.
    let d = context(1, 0xb1);
    witness.advance_counter(&d, 0).unwrap();
    store.commit_advance(&d, None, 1).unwrap();
    assert_eq!(register.check(&d), Ok(Some(1)));
}

#[test]
fn a_witness_failure_after_the_first_register_commit_of_a_new_context_is_not_a_loss() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let c = context(1, 0xa1);
    // The context is born in the witness (call 1), the register commits counter 0's advance, and
    // the witness advance that follows (call 2) fails: the real crash window of a first reserve.
    witness.fail_counter_advances_after(1, 1);
    assert_eq!(
        register.reserve(&c).unwrap_err(),
        NonceError::Witness(WitnessError::Unavailable)
    );
    assert_eq!(store.next_counter(&c), Ok(Some(1)));
    assert_eq!(witness.peek_counter(&c), Some(0));
    // Consistent, not NonceStateLost; counter 0 is burned and the next one is 1.
    assert_eq!(register.check(&c), Ok(Some(1)));
    assert_eq!(register.reserve(&c).unwrap().counter(), 1);
}

#[test]
fn a_crash_after_the_witness_advance_and_before_encrypting_burns_the_counter() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let c = context(1, 0xa1);
    // Reserved and dropped: the process died before it encrypted.
    drop(register.reserve(&c).unwrap());
    assert_eq!(register.reserve(&c).unwrap().counter(), 1);
}

// ---- NonceStateLost: detect, never jump forward ------------------------------------------------

#[test]
fn a_register_rolled_back_behind_the_witness_is_nonce_state_lost_and_is_not_moved_forward() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let c = context(1, 0xa1);
    for _ in 0..5 {
        drop(register.reserve(&c).unwrap());
    }
    // A restore of the vault: the register says 2, the witness (outside the vault) still says 5.
    store.force_next(&c, Some(2));
    let lost = NonceError::NonceStateLost(LostReason::RegisterBehindWitness);
    assert_eq!(register.check(&c), Err(lost));
    assert_eq!(register.reserve(&c).unwrap_err(), lost);
    assert_eq!(register.reconcile(&c), Err(lost));
    // Nothing moved: the register was not jumped forward to the witness, and nothing was spent.
    assert_eq!(store.next_counter(&c), Ok(Some(2)));
    assert_eq!(witness.peek_counter(&c), Some(5));
}

#[test]
fn a_register_with_no_witness_is_nonce_state_lost() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let c = context(1, 0xa1);
    drop(register.reserve(&c).unwrap());
    witness.forget_counter(&c);
    assert_eq!(
        register.reserve(&c).unwrap_err(),
        NonceError::NonceStateLost(LostReason::WitnessMissing)
    );
    assert_eq!(store.next_counter(&c), Ok(Some(1)), "no counter was spent");
}

#[test]
fn a_witness_for_a_context_with_no_register_is_nonce_state_lost() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let c = context(1, 0xa1);
    for _ in 0..3 {
        drop(register.reserve(&c).unwrap());
    }
    // The vault forgot the context entirely (restored from before it existed).
    store.force_next(&c, None);
    assert_eq!(
        register.reserve(&c).unwrap_err(),
        NonceError::NonceStateLost(LostReason::RegisterMissing)
    );
    assert_eq!(store.next_counter(&c), Ok(None));
}

#[test]
fn an_advance_to_zero_or_not_forward_is_refused_and_writes_nothing() {
    let (store, witness) = setup();
    let c = context(1, 0xa1);
    assert_eq!(
        store.commit_advance(&c, None, 0),
        Err(RegisterStoreError::BadAdvance)
    );
    assert_eq!(store.next_counter(&c), Ok(None));
    // A register entry is never 0, so a store that accepted 0 would let (some(0), some(0)) look
    // consistent and hand counter 0 out a second time after the birth.
    witness.advance_counter(&c, 0).unwrap();
    let register = CounterRegister::new(&store, &witness);
    assert_eq!(register.reserve(&c).unwrap().counter(), 0);
    assert_eq!(store.next_counter(&c), Ok(Some(1)));
}

#[test]
fn injected_reuse_is_refused_by_the_compare_and_set() {
    let (store, witness) = setup();
    let c = context(1, 0xa1);
    drop(CounterRegister::new(&store, &witness).reserve(&c).unwrap());
    // A second writer that read the same value tries to take the same counter.
    assert_eq!(
        store.commit_advance(&c, Some(0), 1),
        Err(RegisterStoreError::Conflict)
    );
    assert_eq!(store.next_counter(&c), Ok(Some(1)));
}

// ---- the cap -----------------------------------------------------------------------------------

#[test]
fn the_last_counter_seals_and_the_next_reserve_requires_a_rotation() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let c = context(1, 0xa1);
    // The register is at the last counter, and the witness agrees.
    store.force_next(&c, Some(MAX_INVOCATION_COUNTER));
    witness.force_counter(&c, MAX_INVOCATION_COUNTER);
    let last = register.reserve(&c).unwrap();
    assert_eq!(last.counter(), MAX_INVOCATION_COUNTER);
    assert_eq!(
        register.reserve(&c).unwrap_err(),
        NonceError::RotationRequired
    );
    assert_eq!(
        register.reserve(&c).unwrap_err(),
        NonceError::RotationRequired,
        "and it stays refused"
    );
    assert_eq!(store.next_counter(&c), Ok(Some(MAX_INVOCATION_COUNTER + 1)));
    // A rotation is a new epoch, a new context at 0.
    assert_eq!(
        register.reserve(&context(2, 0xa1)).unwrap().counter(),
        0,
        "a fresh epoch starts over"
    );
}

// ---- contexts ----------------------------------------------------------------------------------

#[test]
fn re_enrollment_and_rotation_start_fresh_contexts_and_never_consult_the_old_one() {
    let (store, witness) = setup();
    let register = CounterRegister::new(&store, &witness);
    let old = context(1, 0xa1);
    for _ in 0..4 {
        drop(register.reserve(&old).unwrap());
    }
    // The old context is rolled back and lost; a new fingerprint (re-enrollment) is unaffected.
    store.force_next(&old, Some(1));
    assert!(register.reserve(&old).is_err());
    let re_enrolled = context(1, 0xa2);
    assert_eq!(register.reserve(&re_enrolled).unwrap().counter(), 0);
    // So is a new epoch of the same fingerprint, and another object type of the same key.
    assert_eq!(register.reserve(&context(2, 0xa1)).unwrap().counter(), 0);
    let envelope = CounterContext {
        object_type: 1,
        ..old
    };
    assert_eq!(register.reserve(&envelope).unwrap().counter(), 0);
    // And the old context is still lost: the new ones did not repair it.
    assert!(register.reserve(&old).is_err());
}

// ---- the verified-state mark (C.6 freshness rule 5) -------------------------------------------

fn mark(
    version: u64,
    head: u8,
    epoch: u64,
    epoch_head: Option<u8>,
    positioned: u64,
) -> VerifiedMark {
    VerifiedMark {
        membership_version: version,
        membership_head_id: [head; 32],
        key_epoch: epoch,
        epoch_head_id: epoch_head.map(|b| [b; 32]),
        positioned_records: positioned,
    }
}

#[test]
fn marks_compare_by_progress() {
    let base = mark(3, 3, 1, Some(9), 4);
    assert_eq!(base.cmp_progress(&base), Progress::Equal);
    assert_eq!(
        mark(4, 4, 1, Some(9), 4).cmp_progress(&base),
        Progress::Ahead
    );
    assert_eq!(
        mark(3, 3, 1, Some(9), 5).cmp_progress(&base),
        Progress::Ahead
    );
    assert_eq!(
        mark(2, 2, 1, Some(9), 4).cmp_progress(&base),
        Progress::Behind
    );
    assert_eq!(
        mark(3, 3, 1, Some(9), 3).cmp_progress(&base),
        Progress::Behind
    );
    // Ahead in one part and behind in another is not progress.
    assert_eq!(
        mark(4, 4, 1, Some(9), 3).cmp_progress(&base),
        Progress::Diverged
    );
    // The same version with another head, or the same epoch with another head, is another history.
    assert_eq!(
        mark(3, 7, 1, Some(9), 4).cmp_progress(&base),
        Progress::Diverged
    );
    assert_eq!(
        mark(3, 3, 1, Some(8), 4).cmp_progress(&base),
        Progress::Diverged
    );
}

#[test]
fn the_guard_accepts_equal_and_ahead_state_and_raises_the_witness() {
    let witness = MockWitness::new();
    let guard = VerifiedStateGuard::new(&witness);
    let vault = [0x11; 16];
    let m1 = mark(1, 1, 1, Some(1), 1);
    let m2 = mark(2, 2, 1, Some(1), 2);
    assert_eq!(guard.check(&vault, None), Ok(()), "a vault with no state");
    guard.record(&vault, &m1).unwrap();
    assert_eq!(guard.check(&vault, Some(&m1)), Ok(()));
    // The vault persisted m2 and died before recording it: ahead is the crash window.
    assert_eq!(guard.check(&vault, Some(&m2)), Ok(()));
    guard.record(&vault, &m2).unwrap();
    assert_eq!(witness.verified_mark(&vault), Ok(Some(m2)));
    // Recording an older mark is refused; the witness never goes backwards.
    assert_eq!(
        guard.record(&vault, &m1),
        Err(VerifiedStateError::StateRolledBack)
    );
    assert_eq!(witness.verified_mark(&vault), Ok(Some(m2)));
}

#[test]
fn older_restored_verified_state_is_state_rolled_back() {
    let witness = MockWitness::new();
    let guard = VerifiedStateGuard::new(&witness);
    let vault = [0x11; 16];
    let (old, new) = (mark(2, 2, 1, Some(1), 2), mark(5, 5, 2, Some(4), 4));
    guard.record(&vault, &new).unwrap();
    for restored in [
        Some(&old),
        Some(&mark(5, 5, 2, Some(4), 3)), // the position map lost an entry
        Some(&mark(5, 6, 2, Some(4), 4)), // another history at the same version
        None,                             // the vault holds no state at all
    ] {
        assert_eq!(
            guard.check(&vault, restored),
            Err(VerifiedStateError::StateRolledBack),
            "{restored:?}"
        );
    }
    // And a vault that holds state the witness has never heard of is not trusted either.
    assert_eq!(
        guard.check(&[0x22; 16], Some(&old)),
        Err(VerifiedStateError::StateRolledBack)
    );
}

#[test]
fn the_verified_state_has_a_birth_mark_so_the_first_persist_has_no_false_rollback_window() {
    let witness = MockWitness::new();
    let guard = VerifiedStateGuard::new(&witness);
    let vault = [0x11; 16];
    let first = mark(1, 1, 1, Some(1), 1);

    // (none, none): the vault has not begun. Nothing was persisted, so there is nothing to lose.
    assert_eq!(guard.check(&vault, None), Ok(()));

    // Step 0, before the first persist: the birth mark.
    guard.begin(&vault).unwrap();
    assert_eq!(witness.verified_mark(&vault), Ok(Some(VerifiedMark::BIRTH)));
    guard.begin(&vault).unwrap(); // idempotent
                                  // (none, birth): begun, nothing persisted yet. Fresh, not rolled back.
    assert_eq!(guard.check(&vault, None), Ok(()));

    // The first state is persisted and the process dies before `record`: (state, birth) is ahead,
    // the crash window, and reconcile records the state's mark.
    assert_eq!(guard.check(&vault, Some(&first)), Ok(()));
    guard.reconcile(&vault, Some(&first)).unwrap();
    assert_eq!(witness.verified_mark(&vault), Ok(Some(first)));
    assert_eq!(guard.check(&vault, Some(&first)), Ok(()));

    // (state, none): a vault whose state the witness never saw is a restore onto another device.
    let other = [0x22; 16];
    assert_eq!(
        guard.check(&other, Some(&first)),
        Err(VerifiedStateError::StateRolledBack)
    );
    assert_eq!(
        guard.reconcile(&other, Some(&first)),
        Err(VerifiedStateError::StateRolledBack)
    );
    assert_eq!(
        witness.verified_mark(&other),
        Ok(None),
        "and nothing was recorded"
    );

    // (none, a real mark): the vault lost the state the witness saw.
    assert_eq!(
        guard.check(&vault, None),
        Err(VerifiedStateError::StateRolledBack)
    );
}

#[test]
fn a_crash_before_the_birth_persisted_nothing_and_a_crash_after_it_persisted_nothing_yet() {
    let witness = MockWitness::new();
    let guard = VerifiedStateGuard::new(&witness);
    let vault = [0x11; 16];
    // Crash window 1: before `begin`. Nothing was persisted (the birth precedes the persist), so
    // the restart sees (none, none).
    assert_eq!(guard.check(&vault, None), Ok(()));
    // Crash window 2: after `begin` and before the persist: (none, birth).
    guard.begin(&vault).unwrap();
    assert_eq!(guard.check(&vault, None), Ok(()));
    // The persist then completes and `record` runs: equal.
    let first = mark(1, 1, 1, Some(1), 1);
    guard.record(&vault, &first).unwrap();
    assert_eq!(guard.check(&vault, Some(&first)), Ok(()));
}

#[test]
fn recording_a_real_mark_on_a_vault_the_witness_never_saw_is_born_first() {
    let witness = MockWitness::new();
    let guard = VerifiedStateGuard::new(&witness);
    let vault = [0x11; 16];
    let first = mark(1, 1, 1, Some(1), 1);
    // `record` without a `begin` never leaves a real mark as the witness's first value.
    guard.record(&vault, &first).unwrap();
    assert_eq!(witness.verified_mark(&vault), Ok(Some(first)));
    assert_eq!(
        witness.mark_log(),
        [(vault, VerifiedMark::BIRTH), (vault, first)],
        "the birth came first"
    );
    // And a birth can never be recorded over a real mark.
    assert_eq!(
        witness.advance_verified_mark(&vault, &VerifiedMark::BIRTH),
        Err(WitnessError::WouldRegress)
    );
}

#[test]
fn the_birth_mark_is_behind_every_real_mark_and_is_not_one() {
    let thin = mark(1, 1, 1, Some(1), 0); // no epoch record counted: still a real mark
    assert!(!thin.is_birth());
    assert!(VerifiedMark::BIRTH.is_birth());
    assert_eq!(thin.cmp_progress(&VerifiedMark::BIRTH), Progress::Ahead);
    assert_eq!(VerifiedMark::BIRTH.cmp_progress(&thin), Progress::Behind);
    assert_eq!(
        VerifiedMark::BIRTH.cmp_progress(&VerifiedMark::BIRTH),
        Progress::Equal
    );
}

#[test]
fn an_unavailable_witness_is_a_typed_error_for_the_guard() {
    let witness = MockWitness::new();
    let guard = VerifiedStateGuard::new(&witness);
    witness.set_unavailable(true);
    let m = mark(1, 1, 1, Some(1), 1);
    assert_eq!(
        guard.check(&[0x11; 16], Some(&m)),
        Err(VerifiedStateError::Witness(WitnessError::Unavailable))
    );
    assert_eq!(
        guard.record(&[0x11; 16], &m),
        Err(VerifiedStateError::Witness(WitnessError::Unavailable))
    );
}

#[test]
fn the_mark_of_a_verified_chain_follows_its_head_and_epoch() {
    let c = common::chain();
    let recs = |n: usize| {
        let m: Vec<&[u8]> = c.memberships[..n].iter().map(Vec::as_slice).collect();
        let e: Vec<&[u8]> = c.epochs[..n].iter().map(Vec::as_slice).collect();
        sync_core::membership::verify_chain(&common::VAULT, &m, &e).unwrap()
    };
    assert!(verified_mark_of(
        &sync_core::membership::VerifiedState::empty(common::VAULT),
        0
    )
    .is_none());
    let early = verified_mark_of(&recs(1), 1).unwrap();
    let late = verified_mark_of(&recs(3), 3).unwrap();
    assert_eq!(early.membership_version, 1);
    assert_eq!(late.membership_version, 3);
    assert_eq!(late.cmp_progress(&early), Progress::Ahead);
    assert_eq!(early.cmp_progress(&late), Progress::Behind);
}

// ---- the witness is never the keychain, and has no production implementation here ------------------

/// `RollbackWitness` is implemented in `platform-mock` and in tests only, until the platform
/// witness (personal-cfo-elciw) lands. A production build with no platform witness has nothing to
/// give `CounterRegister`, so it refuses to encrypt; it never falls back to the keychain or to
/// "no witness".
#[test]
fn no_production_code_implements_the_witness_except_the_mock() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut stack = vec![root.join("crates"), root.join("apps")];
    let mut found = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "target" && name != "node_modules" && !name.starts_with('.') {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") {
                let shown = path.to_string_lossy().replace('\\', "/");
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                for line in text.lines() {
                    let t = line.trim_start();
                    if t.starts_with("impl") && t.contains("RollbackWitness for") {
                        found += 1;
                        assert!(
                            shown.ends_with("crates/platform-mock/src/lib.rs")
                                || shown.contains("/tests/"),
                            "{shown} implements RollbackWitness: only platform-mock (or a test) \
                             may until the platform witness exists"
                        );
                    }
                }
            }
        }
    }
    assert!(found >= 1, "the guard found no implementations at all");
}

/// No type that implements `SecretStore` implements the witness, and the witness trait is not
/// named in a keychain implementation.
#[test]
fn the_keychain_crate_never_names_the_witness() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let keychain = std::fs::read_to_string(root.join("crates/platform-macos/src/keychain.rs"))
        .expect("the keychain source");
    assert!(
        !keychain.contains("RollbackWitness"),
        "a keychain is custody, never the witness"
    );
}
