//! `OSStatus` to [`SecretError`] mapping. Safe code with no platform calls, so it is tested on every
//! host. Only fixed enum values and the numeric status leave this function: platform messages can
//! echo what was passed in, so they are never used.

use platform_traits::{SecretError, StoreStep, UnavailableReason};

pub const ERR_USER_CANCELED: i32 = -128;
pub const ERR_SEC_AUTH_FAILED: i32 = -25293;
pub const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
pub const ERR_SEC_NOT_AVAILABLE: i32 = -25291;
pub const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;
/// A required entitlement is not present (the data-protection keychain without its entitlements).
pub const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34018;

/// Maps a Security framework status to the trait's typed error.
#[must_use]
pub fn map_os_status(status: i32) -> SecretError {
    match status {
        ERR_SEC_ITEM_NOT_FOUND => SecretError::NotFound,
        ERR_SEC_MISSING_ENTITLEMENT => {
            SecretError::Unavailable(UnavailableReason::MissingEntitlement)
        }
        ERR_USER_CANCELED => SecretError::AuthCancelled,
        ERR_SEC_AUTH_FAILED => SecretError::AuthFailed,
        // Locked or in the background: the system will not interact with the user right now.
        ERR_SEC_INTERACTION_NOT_ALLOWED => SecretError::Unavailable(UnavailableReason::Locked),
        // No keychain available (for example a headless session).
        ERR_SEC_NOT_AVAILABLE => SecretError::Unavailable(UnavailableReason::Unsupported),
        other => SecretError::Platform {
            code: i64::from(other),
        },
    }
}

/// The status [`SecretError::StoreFailed`] carries when the platform refused a step without an error
/// object to read (a null access control with no `CFError`). `0` is `errSecSuccess`, so it can never be
/// a real failure code and reads as "no code given".
pub const NO_STATUS_GIVEN: i32 = 0;

/// The error for a store step that failed with `status`: the typed error when the status means
/// something of its own for a store (a missing entitlement, which the app answers with its
/// signed-build copy; a locked keychain, which it answers with "try again when it is unlocked"),
/// otherwise [`SecretError::StoreFailed`] naming the step and the number. A store step never reports
/// "the user cancelled" or "authentication failed" from a status: those describe a prompt, and a store
/// shows none, so they would mislead a caller that read them as a wrong finger.
#[must_use]
pub fn store_step_error(step: StoreStep, status: i32) -> SecretError {
    match map_os_status(status) {
        error @ SecretError::Unavailable(
            UnavailableReason::MissingEntitlement | UnavailableReason::Locked,
        ) => error,
        _ => SecretError::StoreFailed {
            step,
            code: i64::from(status),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_statuses_map_to_their_typed_errors() {
        assert_eq!(map_os_status(-25300), SecretError::NotFound);
        assert_eq!(
            map_os_status(-34018),
            SecretError::Unavailable(UnavailableReason::MissingEntitlement)
        );
        assert_eq!(map_os_status(-128), SecretError::AuthCancelled);
        assert_eq!(map_os_status(-25293), SecretError::AuthFailed);
        assert_eq!(
            map_os_status(-25308),
            SecretError::Unavailable(UnavailableReason::Locked)
        );
        assert_eq!(
            map_os_status(-25291),
            SecretError::Unavailable(UnavailableReason::Unsupported)
        );
    }

    #[test]
    fn a_failed_store_step_names_the_step_and_the_number() {
        for step in [StoreStep::AccessControl, StoreStep::Delete, StoreStep::Add] {
            for code in [-25293, -25291, -128, -50, 1, NO_STATUS_GIVEN] {
                assert_eq!(
                    store_step_error(step, code),
                    SecretError::StoreFailed {
                        step,
                        code: i64::from(code)
                    },
                    "{step:?} {code}"
                );
            }
            // A missing entitlement and a locked keychain keep their typed meaning: the app has copy for both.
            assert_eq!(
                store_step_error(step, -34018),
                SecretError::Unavailable(UnavailableReason::MissingEntitlement)
            );
            assert_eq!(
                store_step_error(step, -25308),
                SecretError::Unavailable(UnavailableReason::Locked)
            );
        }
        let text = format!("{}", store_step_error(StoreStep::Add, -25293));
        assert_eq!(
            text,
            "the platform secret store refused the add step (status -25293)"
        );
    }

    #[test]
    fn anything_else_is_only_a_number() {
        for code in [-50, -25299, -1, 0, 1, i32::MIN, i32::MAX] {
            match map_os_status(code) {
                SecretError::Platform { code: c } => assert_eq!(c, i64::from(code)),
                other => assert!(
                    matches!(code, -50 | -25299) == matches!(other, SecretError::Platform { .. }),
                    "{code} -> {other:?}"
                ),
            }
        }
        // The text of a platform error is fixed and carries only the number.
        let text = format!("{}", map_os_status(-50));
        assert_eq!(text, "the platform secret store failed (status -50)");
    }
}
