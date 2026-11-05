//! The anti-rollback witness (`docs/architecture/vault-format.md` C.4 "Counter register and nonce
//! state" and C.6 freshness rule 5; ADR 0017; `personal-cfo-1cltt.3`).
//!
//! A [`RollbackWitness`] is **independent state that a restore of the vault does not restore**. The
//! Sync counter register and the verified chain live inside the vault, so restoring a backup rolls
//! them back, and a rolled-back counter would reuse an AES-GCM nonce. The witness remembers how far
//! that state had advanced, so a register or a verified chain that is *behind* it is detected.
//!
//! **A [`SecretStore`](crate::SecretStore) is never a witness** (`StoreCapabilities::
//! rollback_resistant` is always `false` for a keychain; the capability matrix says the same). This
//! crate defines the trait and nothing here implements it for production: the platform witness is
//! owned by the Sync client engine bead (`personal-cfo-elciw`), and the only implementation in the
//! tree until then is the in-memory one in `platform-mock`. A source-scan test keeps it that way, so
//! no build can fall back to the keychain or to "no witness".
//!
//! One witness, two kinds of state, because C.6 puts both under the same anti-rollback witness:
//!
//! * **Counters** ([`CounterContext`] to a `u64`), which only move forward.
//! * **The verified-state mark** ([`VerifiedMark`]) of a vault: the newest verified membership head,
//!   the accepted epoch chain and the per-position record map, which only move forward.
//!
//! Both are monotone **by contract**: an implementation refuses to move either backwards
//! ([`WitnessError::WouldRegress`]) and never invents a value.

use thiserror::Error;

/// Which counter a value belongs to: one `(vault, key_epoch, device fingerprint, object_type)`
/// context (C.4). A new fingerprint (re-enrollment) or a new epoch (rotation) is a new context that
/// starts at 0; an old context's value is never consulted for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CounterContext {
    /// The vault.
    pub vault_id: [u8; 16],
    /// The key epoch.
    pub key_epoch: u64,
    /// The sealing device's key fingerprint.
    pub fingerprint: [u8; 32],
    /// The object type (`1` envelope, `2` snapshot).
    pub object_type: u8,
}

/// How far a vault's verified Sync state had advanced (C.6 freshness rule 5). Ordered by
/// [`Self::cmp_progress`]: a mark is *behind* another when any component is behind and none is
/// ahead; two marks that differ in opposite directions, or that agree on the version but name
/// different heads (or agree on the epoch-record count but name different newest records), are *diverged* and are never treated as progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VerifiedMark {
    /// The newest verified membership record's version.
    pub membership_version: u64,
    /// The id of that record (the membership head).
    pub membership_head_id: [u8; 32],
    /// The newest accepted key epoch.
    pub key_epoch: u64,
    /// The id of the newest accepted epoch record, if any.
    pub epoch_head_id: Option<[u8; 32]>,
    /// How many positioned epoch records the per-position map holds: one per accepted epoch
    /// record, grants and cutovers alike, so it grows with every record `epoch_head_id` names.
    pub positioned_records: u64,
}

/// How one mark relates to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// The same state.
    Equal,
    /// This mark is further along than the other in at least one component and behind in none,
    /// and the heads it shares a version with agree.
    Ahead,
    /// This mark is behind the other in at least one component and ahead in none.
    Behind,
    /// Neither: they cannot both be the history of one vault.
    Diverged,
}

impl VerifiedMark {
    /// The **birth mark** of a vault: version 0, all-zero head, epoch 0, no epoch head, no records.
    /// A witness holds it once a vault has begun keeping verified Sync state and before the first
    /// state is persisted, so "the witness has never heard of this vault" (a restored backup) is
    /// told apart from "the first state was persisted a moment before the witness was told" (a
    /// crash). Every real mark is ahead of it; no real mark equals it, because a real state has a
    /// membership head of version 1 or more. The counters have the same device in the value `0`.
    pub const BIRTH: Self = Self {
        membership_version: 0,
        membership_head_id: [0; 32],
        key_epoch: 0,
        epoch_head_id: None,
        positioned_records: 0,
    };

    /// Whether this is [`Self::BIRTH`].
    #[must_use]
    pub fn is_birth(&self) -> bool {
        self == &Self::BIRTH
    }

    /// Compare with `other`.
    #[must_use]
    pub fn cmp_progress(&self, other: &Self) -> Progress {
        use core::cmp::Ordering::{Greater, Less};
        // The birth mark is behind every real mark, whatever its other parts say.
        match (self.is_birth(), other.is_birth()) {
            (true, true) => return Progress::Equal,
            (true, false) => return Progress::Behind,
            (false, true) => return Progress::Ahead,
            (false, false) => {}
        }
        // The same version must name the same head, and the same number of accepted epoch records
        // the same newest one, or the two are different histories.
        if (self.membership_version == other.membership_version
            && self.membership_head_id != other.membership_head_id)
            || (self.positioned_records == other.positioned_records
                && self.epoch_head_id != other.epoch_head_id)
        {
            return Progress::Diverged;
        }
        let parts = [
            self.membership_version.cmp(&other.membership_version),
            self.key_epoch.cmp(&other.key_epoch),
            self.positioned_records.cmp(&other.positioned_records),
        ];
        let ahead = parts.contains(&Greater);
        let behind = parts.contains(&Less);
        match (ahead, behind) {
            (false, false) => Progress::Equal,
            (true, false) => Progress::Ahead,
            (false, true) => Progress::Behind,
            (true, true) => Progress::Diverged,
        }
    }
}

/// Why a witness could not answer or record. Fixed text, no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WitnessError {
    /// The witness is not reachable (locked, unsupported, no platform implementation). The caller
    /// does not encrypt and does not fall back to anything weaker.
    #[error("the anti-rollback witness is not available")]
    Unavailable,
    /// The value offered is behind what the witness already holds. A witness never goes backwards.
    #[error("the anti-rollback witness would move backwards")]
    WouldRegress,
}

/// Independent anti-rollback state (see the module comment). Every method is synchronous and
/// durable: when `advance_*` returns `Ok`, the new value survives a crash and a restore of the
/// vault.
pub trait RollbackWitness: Send + Sync {
    /// The witnessed `next_counter` of `context`, or `None` if the witness has never seen it.
    ///
    /// # Errors
    /// [`WitnessError::Unavailable`].
    fn counter(&self, context: &CounterContext) -> Result<Option<u64>, WitnessError>;

    /// Record that the register of `context` has advanced to `next_counter`. Equal to the held
    /// value is a no-op; a lower value is [`WitnessError::WouldRegress`].
    ///
    /// # Errors
    /// [`WitnessError::Unavailable`] or [`WitnessError::WouldRegress`].
    fn advance_counter(
        &self,
        context: &CounterContext,
        next_counter: u64,
    ) -> Result<(), WitnessError>;

    /// The newest verified-state mark of `vault_id`, or `None` if the witness has never seen one.
    ///
    /// # Errors
    /// [`WitnessError::Unavailable`].
    fn verified_mark(&self, vault_id: &[u8; 16]) -> Result<Option<VerifiedMark>, WitnessError>;

    /// Record that `vault_id`'s verified state has advanced to `mark`. An equal mark is a no-op; a
    /// mark that is behind or diverged from the held one is [`WitnessError::WouldRegress`].
    ///
    /// # Errors
    /// [`WitnessError::Unavailable`] or [`WitnessError::WouldRegress`].
    fn advance_verified_mark(
        &self,
        vault_id: &[u8; 16],
        mark: &VerifiedMark,
    ) -> Result<(), WitnessError>;
}
