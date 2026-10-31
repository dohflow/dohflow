//! Deterministic apply rejections (`docs/architecture/vault-format.md` C.10 "Apply failures";
//! personal-cfo-g3m.26).
//!
//! A command that fails to apply stops a Sync replica only when the failure is a pure function
//! of the command's payload and metadata and the class-1 base state at the `engine_version`: every
//! replica then reaches the same failure at the same position, so a void can name it. Anything
//! else (an I/O error, a lock, a full disk, a cancelled write, a SQLite error whose cause is
//! unknown) is NOT such a verdict and must never become one.
//!
//! The classification is therefore by SITE, not by error variant. [`crate::DbError::InvalidCommand`]
//! stays non-deterministic by default; only a validator audited as reading payload, metadata and
//! class-1 state raises [`crate::DbError::Rejected`] through [`crate::DbError::reject`], and the
//! sites that may do so are listed in `deterministic_sites.toml` (a ratchet test fails on any
//! other). The audit is proven behaviorally in `tests/apply_rejections.rs`: the same rejection under
//! two device time zones and two class-3 states gives the same code and the same write set.
//!
//! The codes are audit-only (a void honors on position, digest and command id, never on the
//! code), so a code is never consensus; the registry is stable and numbered so a replica of the
//! same engine version records the same code. A write-set hash that differs after a SUCCESSFUL
//! apply is never a rejection: it is a fork of the apply semantics and halts (C.10).

use sync_core::status::ApplyFailureCode;

/// Why a command was deterministically rejected. Stable numeric codes, listed in C.10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RejectCode {
    /// The command's `issued_at` is outside 1970-01-01T00:00:00Z to 9999-12-31 (`1`).
    IssuedAtOutOfRange,
    /// A payload field is outside its domain (`2`).
    PayloadInvalid,
    /// A row the command references does not exist on the base (`3`).
    ReferenceMissing,
    /// The base state forbids the command (`4`).
    StateConflict,
}

impl RejectCode {
    /// Every code, in numeric order.
    pub const ALL: [RejectCode; 4] = [
        Self::IssuedAtOutOfRange,
        Self::PayloadInvalid,
        Self::ReferenceMissing,
        Self::StateConflict,
    ];

    /// The stable numeric code.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        match self {
            Self::IssuedAtOutOfRange => 1,
            Self::PayloadInvalid => 2,
            Self::ReferenceMissing => 3,
            Self::StateConflict => 4,
        }
    }

    /// The code as the sync-core status machine records it.
    #[must_use]
    pub fn apply_failure_code(self) -> ApplyFailureCode {
        ApplyFailureCode::new(self.as_u16()).expect("registry codes are nonzero")
    }

    /// The registry name, as C.10 lists it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::IssuedAtOutOfRange => "IssuedAtOutOfRange",
            Self::PayloadInvalid => "PayloadInvalid",
            Self::ReferenceMissing => "ReferenceMissing",
            Self::StateConflict => "StateConflict",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RejectCode;
    use crate::DbError;

    #[test]
    fn a_rejection_yields_its_registry_code_and_nothing_else_does() {
        for code in RejectCode::ALL {
            let error = DbError::reject(code, "x");
            assert_eq!(
                error.apply_failure_code().map(|c| c.get()),
                Some(code.as_u16())
            );
            assert!(
                matches!(error, DbError::Rejected { site, .. } if site.file().ends_with("reject.rs"))
            );
        }
        assert!(DbError::InvalidCommand("x".to_owned())
            .apply_failure_code()
            .is_none());
    }
}
