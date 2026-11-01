//! When Touch ID is offered and when the vault password is required (`personal-cfo-xuu` AC K1 to K3,
//! the owner's decision 2026-11-01: match the iPhone). Pure, so every rule has its own test.
//!
//! The VAULT password is required, and Touch ID is not offered:
//!
//! * **K1, after a restart**: the boot id differs from the one recorded at the last password unlock;
//! * **K2, after 3 failed or cancelled prompts** since the last password unlock;
//! * **K3, 30 days after the last password unlock**, measured on the monotonic clock (changing the
//!   wall clock cannot extend it; a restart already requires the password under K1, so within one boot
//!   the monotonic clock is the right one).
//!
//! Anything missing, unreadable or inconsistent fails CLOSED: the password is required.

use platform_traits::{BiometricAvailability, BootClockError, BootId};

use super::tag::Markers;

/// Failed or cancelled prompts after which the password is required.
pub const MAX_FAILED_PROMPTS: u8 = 3;

/// 30 days, in nanoseconds.
pub const MAX_PASSWORD_AGE_NS: u64 = 30 * 24 * 60 * 60 * 1_000_000_000;

/// Why Touch ID is not offered right now. Each variant is a stable code the unlock screen turns into
/// plain copy (PR 4); none carries text, a path or a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// K1: the Mac restarted since the vault password was last entered.
    AfterRestart,
    /// K2: three Touch ID prompts failed or were cancelled since the vault password was last entered.
    TooManyFailedPrompts,
    /// K3: the vault password was last entered 30 days or more ago.
    PasswordTooOld,
    /// The markers (or the clock) could not be read, or do not make sense: fail closed.
    MarkersUnreadable,
    /// The vault's key envelope changed since Touch ID was enabled (a password change or a key
    /// rotation): the item is retired.
    Retired,
    /// This Mac has no Touch ID sensor available (not present, not paired, not connected).
    BiometryNotPresent,
    /// No fingerprint is enrolled.
    BiometryNotEnrolled,
    /// Too many failed matches: macOS wants its own passcode before biometrics work again.
    BiometryLockedOut,
    /// The biometric state could not be determined.
    BiometryUnknown,
    /// This build is not signed with the keychain entitlement (a development build): password only.
    SignedBuildNeeded,
    /// The secret store is not available (locked, or no keychain here).
    StoreUnavailable,
    /// Touch ID was never enabled for this vault.
    NotEnabled,
    /// A Touch ID prompt was cancelled or did not match; Touch ID is still offered next time (fewer
    /// than three in a row).
    PromptNotCompleted,
    /// The stored key is fine but the vault could not be opened with it right now (a damaged vault, a
    /// rotation that needs recovery): the password path decides what comes next.
    UnlockUnavailable,
    /// Another Touch ID attempt for this app is still running (a prompt is up). The second one is
    /// refused: no prompt, no count.
    AttemptInProgress,
}

impl Reason {
    /// The stable wire code (the frontend branches on these strings).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::AfterRestart => "after_restart",
            Self::TooManyFailedPrompts => "too_many_failed_prompts",
            Self::PasswordTooOld => "password_too_old",
            Self::MarkersUnreadable => "markers_unreadable",
            Self::Retired => "retired",
            Self::BiometryNotPresent => "biometry_not_present",
            Self::BiometryNotEnrolled => "biometry_not_enrolled",
            Self::BiometryLockedOut => "biometry_locked_out",
            Self::BiometryUnknown => "biometry_unknown",
            Self::SignedBuildNeeded => "signed_build_needed",
            Self::StoreUnavailable => "store_unavailable",
            Self::NotEnabled => "not_enabled",
            Self::PromptNotCompleted => "prompt_not_completed",
            Self::UnlockUnavailable => "unlock_unavailable",
            Self::AttemptInProgress => "attempt_in_progress",
        }
    }
}

/// The reason for a biometric state that is not `Available`.
#[must_use]
pub fn reason_for(availability: BiometricAvailability) -> Option<Reason> {
    match availability {
        BiometricAvailability::Available => None,
        BiometricAvailability::NotEnrolled => Some(Reason::BiometryNotEnrolled),
        BiometricAvailability::NotPresent => Some(Reason::BiometryNotPresent),
        BiometricAvailability::LockedOut => Some(Reason::BiometryLockedOut),
        _ => Some(Reason::BiometryUnknown),
    }
}

/// `Ok(())` when Touch ID may be offered for these markers; otherwise the FIRST rule that says no, in
/// the order restart, failed prompts, age, then the biometric state.
///
/// # Errors
/// The [`Reason`] Touch ID is not offered for.
pub fn eligibility(
    markers: &Markers,
    boot: Result<BootId, BootClockError>,
    now_ns: Result<u64, BootClockError>,
    availability: BiometricAvailability,
) -> Result<(), Reason> {
    // K1. An unreadable boot id is not "the same boot".
    let boot = boot.map_err(|_| Reason::MarkersUnreadable)?;
    if boot != markers.boot {
        return Err(Reason::AfterRestart);
    }
    // K2.
    if markers.failed_prompts >= MAX_FAILED_PROMPTS {
        return Err(Reason::TooManyFailedPrompts);
    }
    // K3. Within one boot the clock cannot read earlier than the stamp; if it does, something is
    // wrong with what was recorded and the password is required.
    let now_ns = now_ns.map_err(|_| Reason::MarkersUnreadable)?;
    let Some(age) = now_ns.checked_sub(markers.password_stamp_ns) else {
        return Err(Reason::MarkersUnreadable);
    };
    if age >= MAX_PASSWORD_AGE_NS {
        return Err(Reason::PasswordTooOld);
    }
    match reason_for(availability) {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOT: BootId = BootId::new([7; 16]);
    const SECOND: u64 = 1_000_000_000;

    fn markers(stamp: u64, failed: u8) -> Markers {
        Markers {
            fingerprint: [1; 32],
            boot: BOOT,
            password_stamp_ns: stamp,
            failed_prompts: failed,
        }
    }

    fn check(
        markers: &Markers,
        boot: Result<BootId, BootClockError>,
        now: Result<u64, BootClockError>,
    ) -> Result<(), Reason> {
        eligibility(markers, boot, now, BiometricAvailability::Available)
    }

    #[test]
    fn a_fresh_password_unlock_in_the_same_boot_offers_touch_id() {
        assert_eq!(
            check(&markers(10 * SECOND, 0), Ok(BOOT), Ok(11 * SECOND)),
            Ok(())
        );
        assert_eq!(
            check(&markers(10 * SECOND, 2), Ok(BOOT), Ok(11 * SECOND)),
            Ok(())
        );
    }

    #[test]
    fn k1_a_restart_requires_the_password() {
        let other = BootId::new([8; 16]);
        assert_eq!(
            check(&markers(10 * SECOND, 0), Ok(other), Ok(11 * SECOND)),
            Err(Reason::AfterRestart)
        );
    }

    #[test]
    fn k1_an_unreadable_boot_id_fails_closed() {
        assert_eq!(
            check(
                &markers(10 * SECOND, 0),
                Err(BootClockError),
                Ok(11 * SECOND)
            ),
            Err(Reason::MarkersUnreadable)
        );
    }

    #[test]
    fn k2_the_third_failed_or_cancelled_prompt_requires_the_password() {
        assert_eq!(
            check(
                &markers(10 * SECOND, MAX_FAILED_PROMPTS),
                Ok(BOOT),
                Ok(11 * SECOND)
            ),
            Err(Reason::TooManyFailedPrompts)
        );
        assert_eq!(
            check(&markers(10 * SECOND, 200), Ok(BOOT), Ok(11 * SECOND)),
            Err(Reason::TooManyFailedPrompts)
        );
    }

    #[test]
    fn k3_thirty_days_since_the_password_requires_it_again() {
        let stamp = 10 * SECOND;
        assert_eq!(
            check(
                &markers(stamp, 0),
                Ok(BOOT),
                Ok(stamp + MAX_PASSWORD_AGE_NS - 1)
            ),
            Ok(()),
            "one nanosecond short of 30 days"
        );
        assert_eq!(
            check(
                &markers(stamp, 0),
                Ok(BOOT),
                Ok(stamp + MAX_PASSWORD_AGE_NS)
            ),
            Err(Reason::PasswordTooOld),
            "exactly 30 days"
        );
    }

    #[test]
    fn k3_an_unreadable_clock_or_a_clock_before_the_stamp_fails_closed() {
        assert_eq!(
            check(&markers(10 * SECOND, 0), Ok(BOOT), Err(BootClockError)),
            Err(Reason::MarkersUnreadable)
        );
        // Within one boot the monotonic clock cannot be earlier than the recorded stamp.
        assert_eq!(
            check(&markers(10 * SECOND, 0), Ok(BOOT), Ok(5 * SECOND)),
            Err(Reason::MarkersUnreadable)
        );
    }

    #[test]
    fn the_rules_are_checked_in_order_restart_then_failures_then_age_then_the_sensor() {
        let old_and_failed = markers(0, MAX_FAILED_PROMPTS);
        let other = BootId::new([9; 16]);
        assert_eq!(
            eligibility(
                &old_and_failed,
                Ok(other),
                Ok(MAX_PASSWORD_AGE_NS),
                BiometricAvailability::NotPresent
            ),
            Err(Reason::AfterRestart)
        );
        assert_eq!(
            eligibility(
                &old_and_failed,
                Ok(BOOT),
                Ok(MAX_PASSWORD_AGE_NS),
                BiometricAvailability::NotPresent
            ),
            Err(Reason::TooManyFailedPrompts)
        );
        assert_eq!(
            eligibility(
                &markers(0, 0),
                Ok(BOOT),
                Ok(MAX_PASSWORD_AGE_NS),
                BiometricAvailability::NotPresent
            ),
            Err(Reason::PasswordTooOld)
        );
        assert_eq!(
            eligibility(
                &markers(0, 0),
                Ok(BOOT),
                Ok(SECOND),
                BiometricAvailability::NotPresent
            ),
            Err(Reason::BiometryNotPresent)
        );
    }

    #[test]
    fn every_biometric_state_has_its_reason() {
        for (state, reason) in [
            (
                BiometricAvailability::NotEnrolled,
                Reason::BiometryNotEnrolled,
            ),
            (
                BiometricAvailability::NotPresent,
                Reason::BiometryNotPresent,
            ),
            (BiometricAvailability::LockedOut, Reason::BiometryLockedOut),
            (BiometricAvailability::Unknown, Reason::BiometryUnknown),
        ] {
            assert_eq!(
                eligibility(&markers(0, 0), Ok(BOOT), Ok(SECOND), state),
                Err(reason)
            );
        }
    }

    #[test]
    fn the_codes_are_unique_and_stable() {
        let all = [
            Reason::AfterRestart,
            Reason::TooManyFailedPrompts,
            Reason::PasswordTooOld,
            Reason::MarkersUnreadable,
            Reason::Retired,
            Reason::BiometryNotPresent,
            Reason::BiometryNotEnrolled,
            Reason::BiometryLockedOut,
            Reason::BiometryUnknown,
            Reason::SignedBuildNeeded,
            Reason::StoreUnavailable,
            Reason::NotEnabled,
            Reason::PromptNotCompleted,
            Reason::UnlockUnavailable,
            Reason::AttemptInProgress,
        ];
        let codes: std::collections::BTreeSet<_> = all.iter().map(|r| r.code()).collect();
        assert_eq!(codes.len(), all.len());
        assert!(codes
            .iter()
            .all(|c| c.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_')));
    }
}
