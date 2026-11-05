//! The AEAD counter register and its anti-rollback state machine (`docs/architecture/vault-format.md`
//! C.4 "Counter register and nonce state", C.6 freshness rule 5; `personal-cfo-1cltt.3`).
//!
//! Every AEAD key `K_{e,d,t}` is used with a 4-byte nonce domain and an 8-byte counter, so one
//! counter value used twice under one key is AES-GCM nonce reuse, which breaks both
//! confidentiality and authenticity. A device therefore owns a durable `next_counter` for every
//! `(vault, key_epoch, fingerprint, object_type)` context, and **a counter is never handed out
//! before its advance is durable**.
//!
//! # The order, per encryption (W2)
//!
//! 1. **Commit the register advance** ([`RegisterStore::commit_advance`]). For an envelope the
//!    store makes it part of the same transaction as the command's op-log row (C.4 producer rule 1),
//!    which is the store's business, not this module's.
//! 2. **Advance the witness** ([`RollbackWitness::advance_counter`]) to the same value.
//! 3. **Only then** return a [`CounterReservation`], the one value a sealer accepts as a counter.
//!
//! A failure at (1) burns nothing. A failure at (2) leaves the register ahead of the witness and the
//! counter **burned**: it is never handed out, and the next call advances past it. A crash between
//! (1) and (2) is the same state. [`CounterRegister::reserve`] never returns a counter on any error.
//!
//! # The comparison rule (detect, never jump forward)
//!
//! Before it reserves, the register compares its persisted value `r` with the witness's `w`:
//!
//! | Register | Witness | Verdict |
//! | --- | --- | --- |
//! | none | none, or `0` | a fresh context, counter 0 (a new context records `0` in the witness first, so a crash before its first commit is not a loss) |
//! | `r` | `w`, `r >= w` | consistent (`r > w` is the crash window above; the next advance raises the witness) |
//! | `r` | `w`, `r < w` | [`NonceStateLost`]: the vault was rolled back |
//! | `r` | none | `NonceStateLost`: a register with no witness is not trusted |
//! | none | `w` | `NonceStateLost`: the witness has seen a context the vault has forgotten |
//!
//! The register **never moves itself forward to the witness**: the counters between its value and
//! the witness's were used by this device under this key, and the only safe continuation is a fresh
//! key (re-enrollment or a new epoch), because the old key's used counter range is unknown. C.14
//! requires `NonceStateLost` on a register rollback.
//!
//! # The cap
//!
//! A derived key has `2^32` invocations (counters `0` to `2^32 - 1`). After the last one has been
//! reserved, [`CounterRegister::reserve`] returns [`NonceError::RotationRequired`] and nothing
//! more: the rotation itself (reason 4) belongs to the epoch cutover bead.
//!
//! # No production witness here
//!
//! This module is generic over a [`RollbackWitness`]; the only implementation in the tree is
//! `platform-mock`'s, and a source-scan test keeps it so until the platform witness
//! (`personal-cfo-elciw`) exists. A build with no platform witness has nothing to hand
//! [`CounterRegister`], so it cannot reserve a counter: it never falls back to a keychain or to "no
//! witness".

use platform_traits::{CounterContext, Progress, RollbackWitness, VerifiedMark, WitnessError};
use sync_core::header::MAX_INVOCATION_COUNTER;
use sync_core::membership::VerifiedState;
use thiserror::Error;

/// Why a register store could not read or commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum RegisterStoreError {
    /// The store could not be read or written.
    #[error("the counter register is not available")]
    Unavailable,
    /// The persisted value is not the one the advance was computed from.
    #[error("the counter register changed during the advance")]
    Conflict,
    /// The advance does not move forward: to `0`, or to a value no greater than the one it is
    /// computed from. A register entry is never `0` (the first advance goes to `1`), which is what
    /// lets a witness at `0` mean "no counter was ever handed out".
    #[error("the counter register can only advance to a larger value, and never to zero")]
    BadAdvance,
}

/// A sealed snapshot saved for exact-byte retransmission (W4). It sits next to the register entry
/// of the context that sealed it; the bytes themselves are a file (see `sync_snapshot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedSealed {
    /// The snapshot id the container was sealed for.
    pub object_id: [u8; 16],
    /// The `base_seq` the header carries.
    pub base_seq: u64,
    /// SHA-256 of the plaintext that was sealed: a retry with different content is not the same
    /// publish.
    pub plaintext_sha256: [u8; 32],
    /// SHA-256 of the container bytes in the file.
    pub container_sha256: [u8; 32],
    /// The container's length.
    pub container_len: u64,
    /// The counter the container was sealed with.
    pub counter: u64,
}

/// The durable `next_counter` of every context, and the saved snapshot of a context.
///
/// An implementation must make [`Self::commit_advance`] durable before it returns `Ok`, and must
/// refuse it with [`RegisterStoreError::Conflict`] unless the persisted value equals `from`
/// (a compare-and-set, so two writers can never hand out one counter). The SQLite store, which
/// commits the advance in the same transaction as the op-log row, comes with the first consumer
/// that has a transaction (the snapshot publish and the command engine).
pub trait RegisterStore {
    /// The persisted `next_counter` of `context`, or `None` if it has no entry.
    ///
    /// # Errors
    /// [`RegisterStoreError::Unavailable`].
    fn next_counter(&self, context: &CounterContext) -> Result<Option<u64>, RegisterStoreError>;

    /// Persist `next_counter = to` if it currently equals `from` (`None` meaning no entry).
    /// `to` must be at least `1` and greater than `from`: anything else is
    /// [`RegisterStoreError::BadAdvance`], checked before the comparison.
    ///
    /// # Errors
    /// [`RegisterStoreError::Unavailable`], [`RegisterStoreError::Conflict`] or
    /// [`RegisterStoreError::BadAdvance`].
    fn commit_advance(
        &self,
        context: &CounterContext,
        from: Option<u64>,
        to: u64,
    ) -> Result<(), RegisterStoreError>;

    /// The saved sealed snapshot of `context`, if any.
    ///
    /// # Errors
    /// [`RegisterStoreError::Unavailable`].
    fn saved(&self, context: &CounterContext) -> Result<Option<SavedSealed>, RegisterStoreError>;

    /// Replace (or, with `None`, clear) the saved sealed snapshot of `context`.
    ///
    /// # Errors
    /// [`RegisterStoreError::Unavailable`].
    fn set_saved(
        &self,
        context: &CounterContext,
        saved: Option<&SavedSealed>,
    ) -> Result<(), RegisterStoreError>;
}

/// Why the register is not trusted. The device must not encrypt in the context and recovers by
/// re-enrolling with a fresh device key pair and fingerprint, or by bootstrapping a fresh epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LostReason {
    /// The register is behind the witness: the vault was restored to an older state.
    RegisterBehindWitness,
    /// The register has an entry and the witness has none.
    WitnessMissing,
    /// The saved sealed snapshot names a counter the register has not reached: the saved record
    /// is from another timeline than the register (a restore rolled one back without the other).
    SavedAheadOfRegister,
    /// The witness has an entry and the register has none.
    RegisterMissing,
}

/// Why no counter was handed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum NonceError {
    /// The register and the witness disagree (C.11 `NonceStateLost`). Nothing is encrypted.
    #[error("the nonce state is lost: {0:?}")]
    NonceStateLost(LostReason),
    /// The derived key has used all `2^32` counters; a fresh-key epoch rotation comes first.
    #[error("the key's counters are used up: a fresh-key epoch rotation is required")]
    RotationRequired,
    /// The register store failed.
    #[error("{0}")]
    Register(RegisterStoreError),
    /// The witness failed. If this came from the advance (step 2) the counter is burned.
    #[error("{0}")]
    Witness(WitnessError),
}

impl From<RegisterStoreError> for NonceError {
    fn from(error: RegisterStoreError) -> Self {
        Self::Register(error)
    }
}

impl From<WitnessError> for NonceError {
    fn from(error: WitnessError) -> Self {
        Self::Witness(error)
    }
}

/// A counter value that its register has durably advanced past and its witness has recorded. The
/// only way to obtain one is [`CounterRegister::reserve`], and a sealer takes it by value, so
/// "persist the counter before encrypting" holds by type: a header cannot be built with a counter
/// that was not reserved, and a reservation seals at most once.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a reserved counter that is dropped is burned"]
pub struct CounterReservation {
    context: CounterContext,
    counter: u64,
}

impl CounterReservation {
    /// The context this counter belongs to.
    #[must_use]
    pub const fn context(&self) -> &CounterContext {
        &self.context
    }

    /// The counter value.
    #[must_use]
    pub const fn counter(&self) -> u64 {
        self.counter
    }
}

/// The register of one device: a [`RegisterStore`] and the [`RollbackWitness`] that guards it.
pub struct CounterRegister<'a, S: RegisterStore, W: RollbackWitness> {
    store: &'a S,
    witness: &'a W,
}

impl<S: RegisterStore, W: RollbackWitness> core::fmt::Debug for CounterRegister<'_, S, W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CounterRegister").finish_non_exhaustive()
    }
}

impl<'a, S: RegisterStore, W: RollbackWitness> CounterRegister<'a, S, W> {
    /// A register over `store`, guarded by `witness`.
    pub const fn new(store: &'a S, witness: &'a W) -> Self {
        Self { store, witness }
    }

    /// The store, for the saved-snapshot record the caller keeps beside the register entry.
    pub(crate) const fn store(&self) -> &S {
        self.store
    }

    /// The persisted value of `context`, after comparing it with the witness: `Ok(None)` for a
    /// fresh context, `Ok(Some(next))` for a consistent one, [`NonceError::NonceStateLost`]
    /// otherwise. It changes nothing.
    ///
    /// # Errors
    /// [`NonceError`].
    pub fn check(&self, context: &CounterContext) -> Result<Option<u64>, NonceError> {
        let register = self.store.next_counter(context)?;
        let witnessed = self.witness.counter(context)?;
        match (register, witnessed) {
            (None, None | Some(0)) => Ok(None),
            (Some(r), Some(w)) if r >= w => Ok(Some(r)),
            (Some(_), Some(_)) => Err(NonceError::NonceStateLost(
                LostReason::RegisterBehindWitness,
            )),
            (Some(_), None) => Err(NonceError::NonceStateLost(LostReason::WitnessMissing)),
            (None, Some(_)) => Err(NonceError::NonceStateLost(LostReason::RegisterMissing)),
        }
    }

    /// The start-up reconciliation of `context`: [`Self::check`], and when the register is ahead
    /// of the witness (a crash between steps 1 and 2) raise the witness to the register. Returns
    /// the register's value.
    ///
    /// # Errors
    /// [`NonceError`].
    pub fn reconcile(&self, context: &CounterContext) -> Result<Option<u64>, NonceError> {
        let register = self.check(context)?;
        if let Some(next) = register {
            self.witness.advance_counter(context, next)?;
        }
        Ok(register)
    }

    /// Reserve the next counter of `context`: compare, commit the advance, advance the witness,
    /// and only then return the counter.
    ///
    /// # Errors
    /// [`NonceError`]; no counter is returned on any error, and none is reused after one.
    pub fn reserve(&self, context: &CounterContext) -> Result<CounterReservation, NonceError> {
        let held = self.check(context)?;
        let counter = held.unwrap_or(0);
        if counter > MAX_INVOCATION_COUNTER {
            return Err(NonceError::RotationRequired);
        }
        let to = counter + 1;
        if held.is_none() {
            // A new context is born in the witness first (value 0, nothing issued), so a crash
            // between the register commit and the witness advance below is never mistaken for a
            // register the witness has not heard of.
            self.witness.advance_counter(context, 0)?;
        }
        // (1) the register advance, durable before anything else happens.
        self.store.commit_advance(context, held, to)?;
        // (2) the witness. A failure here leaves the counter burned and never handed out.
        self.witness.advance_counter(context, to)?;
        // (3) the counter.
        Ok(CounterReservation {
            context: *context,
            counter,
        })
    }
}

// ---- the verified-state mark (C.6 freshness rule 5) ------------------------------------------------

/// Why the verified Sync state is not trusted. Handled like `NonceStateLost`, by re-enrollment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum VerifiedStateError {
    /// The loaded state is older than the witness has seen, or the witness has no record of a
    /// state the vault holds, or the vault holds none of a state the witness has seen (C.11
    /// `StateRolledBack`). Nothing is accepted.
    #[error("the verified Sync state was rolled back")]
    StateRolledBack,
    /// The witness failed.
    #[error("{0}")]
    Witness(WitnessError),
}

impl From<WitnessError> for VerifiedStateError {
    fn from(error: WitnessError) -> Self {
        Self::Witness(error)
    }
}

/// The mark of a verified chain: its membership head, its newest accepted epoch and the number of
/// positioned epoch records the caller keeps. `None` for a chain with no record.
#[must_use]
pub fn verified_mark_of(state: &VerifiedState, positioned_records: u64) -> Option<VerifiedMark> {
    let membership_head_id = state.head_id()?;
    Some(VerifiedMark {
        membership_version: state.head_version(),
        membership_head_id,
        key_epoch: state.key_epoch(),
        epoch_head_id: state.epoch_head_id(),
        positioned_records,
    })
}

/// The anti-rollback guard of a vault's verified Sync state. It does for the membership head, the
/// accepted epoch chain and the per-position record map what [`CounterRegister`] does for
/// counters: a loaded state older than the witness has seen is [`VerifiedStateError::StateRolledBack`].
pub struct VerifiedStateGuard<'a, W: RollbackWitness> {
    witness: &'a W,
}

impl<'a, W: RollbackWitness> VerifiedStateGuard<'a, W> {
    /// A guard over `witness`.
    pub const fn new(witness: &'a W) -> Self {
        Self { witness }
    }

    /// Step 0: the vault begins keeping verified Sync state. Records the birth mark in the
    /// witness **before the first state is persisted**, so a crash between that persist and
    /// [`Self::record`] is not mistaken for a restored vault the witness never saw. Idempotent: a
    /// witness that already holds any mark for the vault is left as it is.
    ///
    /// The order for the verified state is: [`Self::begin`], persist, [`Self::record`].
    ///
    /// # Errors
    /// [`VerifiedStateError::Witness`] if the witness is unavailable.
    pub fn begin(&self, vault_id: &[u8; 16]) -> Result<(), VerifiedStateError> {
        if self.witness.verified_mark(vault_id)?.is_none() {
            self.witness
                .advance_verified_mark(vault_id, &VerifiedMark::BIRTH)?;
        }
        Ok(())
    }

    /// Check the state the vault loaded (`None` if it holds none) against the witness, before
    /// anything uses it. Changes nothing.
    ///
    /// | Loaded | Witness | Verdict |
    /// | --- | --- | --- |
    /// | none | none | fresh: the vault has not begun |
    /// | none | birth | fresh: begun, nothing persisted yet |
    /// | state | birth | ahead: the crash window after the first persist ([`Self::reconcile`] records it) |
    /// | state | mark | equal or ahead is consistent; behind or diverged is rolled back |
    /// | state | none | rolled back: a state the witness never saw |
    /// | none | a real mark | rolled back: the vault lost state the witness saw |
    ///
    /// # Errors
    /// [`VerifiedStateError`].
    pub fn check(
        &self,
        vault_id: &[u8; 16],
        loaded: Option<&VerifiedMark>,
    ) -> Result<(), VerifiedStateError> {
        let witnessed = self.witness.verified_mark(vault_id)?;
        match (loaded, witnessed) {
            (None, None) => Ok(()),
            (None, Some(witnessed)) if witnessed.is_birth() => Ok(()),
            (Some(loaded), Some(witnessed)) => match loaded.cmp_progress(&witnessed) {
                Progress::Equal | Progress::Ahead => Ok(()),
                Progress::Behind | Progress::Diverged => Err(VerifiedStateError::StateRolledBack),
            },
            (Some(_), None) | (None, Some(_)) => Err(VerifiedStateError::StateRolledBack),
        }
    }

    /// The start-up reconciliation: [`Self::check`], and when the loaded state is ahead of the
    /// witness (the crash window after a persist and before [`Self::record`]) record it.
    ///
    /// # Errors
    /// [`VerifiedStateError`].
    pub fn reconcile(
        &self,
        vault_id: &[u8; 16],
        loaded: Option<&VerifiedMark>,
    ) -> Result<(), VerifiedStateError> {
        self.check(vault_id, loaded)?;
        if let Some(loaded) = loaded {
            if self.witness.verified_mark(vault_id)?.as_ref() != Some(loaded) {
                self.record(vault_id, loaded)?;
            }
        }
        Ok(())
    }

    /// Record that the vault has persisted `mark`. Call it **after** the state is durable, and
    /// before relying on it. Marks only move forward, and a vault the witness has never seen is
    /// born first, so a real mark is never the first value the witness holds.
    ///
    /// # Errors
    /// [`VerifiedStateError::StateRolledBack`] if `mark` is behind or diverged from the witness,
    /// [`VerifiedStateError::Witness`] if it is unavailable.
    pub fn record(
        &self,
        vault_id: &[u8; 16],
        mark: &VerifiedMark,
    ) -> Result<(), VerifiedStateError> {
        self.begin(vault_id)?;
        match self.witness.advance_verified_mark(vault_id, mark) {
            Ok(()) => Ok(()),
            Err(WitnessError::WouldRegress) => Err(VerifiedStateError::StateRolledBack),
            Err(error) => Err(VerifiedStateError::Witness(error)),
        }
    }
}

/// The conformance suite every [`RegisterStore`] runs (`personal-cfo-1cltt.3` AC5): the in-memory
/// store of this crate's tests, and the SQLite store of the snapshot-publish bead. It states what
/// the register's safety rests on, so a store that fails it is unsafe to encrypt with.
///
/// Test support: it panics on a violation, like `platform-traits`'s suite.
pub mod conformance {
    use platform_traits::{CounterContext, RollbackWitness};

    use super::{CounterRegister, RegisterStore, RegisterStoreError, SavedSealed};

    const fn context(epoch: u64, fingerprint: u8) -> CounterContext {
        CounterContext {
            vault_id: [0x11; 16],
            key_epoch: epoch,
            fingerprint: [fingerprint; 32],
            object_type: 2,
        }
    }

    const fn saved(counter: u64) -> SavedSealed {
        SavedSealed {
            object_id: [0x33; 16],
            base_seq: 41,
            plaintext_sha256: [0x44; 32],
            container_sha256: [0x55; 32],
            container_len: 4096,
            counter,
        }
    }

    /// Run the suite. `open` returns a **new handle on the same underlying storage** each time it
    /// is called (a store reopened after a restart); `witness` is an empty, working witness.
    ///
    /// # Panics
    /// On the first property that does not hold.
    pub fn run<S: RegisterStore, W: RollbackWitness>(open: &dyn Fn() -> S, witness: &W) {
        let first = context(1, 0xa1);
        let other = context(1, 0xb1);
        let next_epoch = context(2, 0xa1);
        let store = open();

        // A new context has no entry; contexts never share one.
        assert_eq!(store.next_counter(&first), Ok(None));
        store.commit_advance(&first, None, 1).unwrap();
        assert_eq!(store.next_counter(&first), Ok(Some(1)));
        assert_eq!(store.next_counter(&other), Ok(None), "another fingerprint");
        assert_eq!(store.next_counter(&next_epoch), Ok(None), "another epoch");

        // An advance moves forward and is never to zero: a register entry is never 0, so a
        // witness at 0 can only mean "no counter was ever handed out".
        let zero = context(4, 0xd1);
        assert_eq!(
            store.commit_advance(&zero, None, 0),
            Err(RegisterStoreError::BadAdvance)
        );
        assert_eq!(store.next_counter(&zero), Ok(None), "nothing was written");
        store.commit_advance(&zero, None, 3).unwrap();
        for (from, to) in [(3, 3), (3, 2), (3, 0)] {
            assert_eq!(
                store.commit_advance(&zero, Some(from), to),
                Err(RegisterStoreError::BadAdvance),
                "{from} to {to}"
            );
        }
        assert_eq!(
            store.next_counter(&zero),
            Ok(Some(3)),
            "and nothing changed"
        );

        // An advance is durable once it returns: a reopened store sees it.
        assert_eq!(open().next_counter(&first), Ok(Some(1)));

        // It is a compare-and-set: a stale `from` is refused and changes nothing, so two writers
        // can never both take one counter.
        let (a, b) = (open(), open());
        assert_eq!(a.next_counter(&first), Ok(Some(1)));
        assert_eq!(b.next_counter(&first), Ok(Some(1)));
        a.commit_advance(&first, Some(1), 2).unwrap();
        assert_eq!(
            b.commit_advance(&first, Some(1), 2),
            Err(RegisterStoreError::Conflict)
        );
        assert_eq!(
            b.commit_advance(&first, None, 1),
            Err(RegisterStoreError::Conflict),
            "an entry exists"
        );
        assert_eq!(open().next_counter(&first), Ok(Some(2)));

        // The saved sealed snapshot: one per context, with its hashes, surviving a reopen and an
        // advance of the counter; clearing it leaves the counter alone.
        assert_eq!(store.saved(&first), Ok(None));
        store.set_saved(&first, Some(&saved(1))).unwrap();
        assert_eq!(open().saved(&first), Ok(Some(saved(1))));
        assert_eq!(open().saved(&other), Ok(None));
        open().commit_advance(&first, Some(2), 3).unwrap();
        assert_eq!(
            open().saved(&first),
            Ok(Some(saved(1))),
            "survives an advance"
        );
        store.set_saved(&first, Some(&saved(2))).unwrap();
        assert_eq!(open().saved(&first), Ok(Some(saved(2))), "replaced");
        store.set_saved(&first, None).unwrap();
        assert_eq!(open().saved(&first), Ok(None));
        assert_eq!(open().next_counter(&first), Ok(Some(3)));

        // A crash between the register commit and the witness advance is consistent after a
        // reopen, and no counter is ever reissued: the burned counter is skipped.
        let fresh = context(3, 0xc1);
        witness.advance_counter(&fresh, 0).unwrap();
        store.commit_advance(&fresh, None, 1).unwrap(); // step 1 done; step 2 never ran
        let reopened = open();
        let register = CounterRegister::new(&reopened, witness);
        assert_eq!(register.reconcile(&fresh), Ok(Some(1)));
        assert_eq!(
            witness.counter(&fresh),
            Ok(Some(1)),
            "the witness is raised"
        );
        let mut seen = vec![0_u64];
        for _ in 0..5 {
            let reservation = register.reserve(&fresh).unwrap();
            assert!(
                !seen.contains(&reservation.counter()),
                "a counter was reissued"
            );
            assert_eq!(reservation.counter(), seen.last().unwrap() + 1);
            seen.push(reservation.counter());
        }
        assert_eq!(open().next_counter(&fresh), Ok(Some(6)));
    }
}
