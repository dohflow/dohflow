//! `OSStatus` to [`SecretError`] mapping. Safe code with no platform calls, so it is tested on every
//! host. Only fixed enum values and the numeric status leave this function: platform messages can
//! echo what was passed in, so they are never used.

use platform_traits::{SecretError, UnavailableReason};

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
