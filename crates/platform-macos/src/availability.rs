//! `LAError` to [`BiometricAvailability`]. Safe code with no platform calls, so it is tested on every
//! host; the platform call that produces the code is in `auth` (macOS only).

use platform_traits::BiometricAvailability;

/// `LAError` codes the availability check maps (stable public values).
pub const LA_BIOMETRY_NOT_AVAILABLE: i64 = -6;
pub const LA_BIOMETRY_NOT_ENROLLED: i64 = -7;
pub const LA_BIOMETRY_LOCKOUT: i64 = -8;
/// A Touch ID keyboard is supported but none is paired with this Mac (a desktop Mac with no sensor).
pub const LA_BIOMETRY_NOT_PAIRED: i64 = -12;
/// A paired Touch ID keyboard is not connected right now.
pub const LA_BIOMETRY_DISCONNECTED: i64 = -13;

/// The availability for the result of `canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics)`:
/// `None` is "yes, it can".
#[must_use]
pub fn availability_from_la_code(code: Option<i64>) -> BiometricAvailability {
    match code {
        None => BiometricAvailability::Available,
        Some(LA_BIOMETRY_NOT_ENROLLED) => BiometricAvailability::NotEnrolled,
        Some(LA_BIOMETRY_LOCKOUT) => BiometricAvailability::LockedOut,
        // No usable sensor on this Mac right now: no hardware, or a keyboard sensor that is not paired
        // or not connected. For the user and for the store these are the same state.
        Some(LA_BIOMETRY_NOT_AVAILABLE | LA_BIOMETRY_NOT_PAIRED | LA_BIOMETRY_DISCONNECTED) => {
            BiometricAvailability::NotPresent
        }
        // Anything else (no passcode set, an invalid context, a future code) is "not available" to
        // the user, but is not claimed to be one of the specific states.
        Some(_) => BiometricAvailability::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_documented_code_maps_to_its_state() {
        assert_eq!(
            availability_from_la_code(None),
            BiometricAvailability::Available
        );
        assert_eq!(
            availability_from_la_code(Some(-7)),
            BiometricAvailability::NotEnrolled
        );
        assert_eq!(
            availability_from_la_code(Some(-8)),
            BiometricAvailability::LockedOut
        );
        assert_eq!(
            availability_from_la_code(Some(-6)),
            BiometricAvailability::NotPresent
        );
    }

    #[test]
    fn a_mac_with_no_paired_or_connected_touch_id_keyboard_is_not_present() {
        // Measured on the build Mac, a Mac mini with no Touch ID keyboard: canEvaluatePolicy answers
        // LAErrorBiometryNotPaired (-12). -13 is a paired keyboard that is not connected.
        for code in [-12, -13] {
            assert_eq!(
                availability_from_la_code(Some(code)),
                BiometricAvailability::NotPresent,
                "{code}"
            );
        }
    }

    #[test]
    fn an_unmapped_code_is_unknown_never_available() {
        for code in [-1, -2, -5, -1000, 0, 1, i64::MIN, i64::MAX] {
            assert_eq!(
                availability_from_la_code(Some(code)),
                BiometricAvailability::Unknown,
                "{code}"
            );
        }
    }
}
