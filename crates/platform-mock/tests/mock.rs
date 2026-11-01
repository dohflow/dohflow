//! The mock passes the shared conformance suite, models the biometric access control separately from
//! the pre-flight prompt, and can play an unavailable platform.

use platform_mock::{BiometryDevice, BiometryScript, MockBiometricAuth, MockSecretStore};
use platform_traits::{
    conformance, AccessPolicy, AuthPrompt, BiometricAuth, BiometricAvailability, PreflightResult,
    PublicTag, SecretError, SecretKey, SecretStore, SecretValue, UnavailableReason,
};

fn prompt() -> AuthPrompt {
    AuthPrompt::new("Unlock DohFlow").unwrap()
}

fn key(name: &str) -> SecretKey {
    SecretKey::new("app.dohflow.test", name).unwrap()
}

fn value(bytes: &[u8]) -> SecretValue {
    SecretValue::new(bytes).unwrap()
}

#[test]
fn the_full_mock_passes_the_conformance_suite() {
    conformance::run(
        &MockSecretStore::new(),
        &MockBiometricAuth::matching(),
        "mock-full",
    );
}

#[test]
fn a_mock_without_biometry_passes_it_too_and_refuses_the_biometric_policy() {
    let store = MockSecretStore::without_biometry();
    conformance::run(&store, &MockBiometricAuth::matching(), "mock-nobio");
    assert!(!store.capabilities().biometry_current_set);
    assert_eq!(
        store
            .store_secret(
                &key("b"),
                &value(b"x"),
                AccessPolicy::DeviceOnlyBiometryCurrentSet
            )
            .unwrap_err(),
        SecretError::Unavailable(UnavailableReason::Unsupported)
    );
    assert!(store.is_empty(), "a refused policy stores nothing");
}

#[test]
fn the_biometric_access_control_decides_and_a_cancelled_prompt_leaves_the_item() {
    let store = MockSecretStore::new();
    let k = key("wrapped-dek");
    store
        .store_secret(
            &k,
            &value(b"wrapped"),
            AccessPolicy::DeviceOnlyBiometryCurrentSet,
        )
        .unwrap();

    store.script_biometry(BiometryScript::Cancel);
    assert_eq!(
        store.load_secret(&k, &prompt()).unwrap_err(),
        SecretError::AuthCancelled
    );
    store.script_biometry(BiometryScript::Fail);
    assert_eq!(
        store.load_secret(&k, &prompt()).unwrap_err(),
        SecretError::AuthFailed
    );
    store.script_biometry(BiometryScript::NotEnrolled);
    assert_eq!(
        store.load_secret(&k, &prompt()).unwrap_err(),
        SecretError::BiometryNotEnrolled
    );
    store.script_biometry(BiometryScript::Match);
    assert_eq!(
        store.load_secret(&k, &prompt()).unwrap(),
        value(b"wrapped"),
        "none of those destroyed it"
    );
}

#[test]
fn an_enrollment_change_invalidates_the_item_and_reads_as_not_found() {
    let store = MockSecretStore::new();
    let k = key("wrapped-dek");
    store
        .store_secret(
            &k,
            &value(b"wrapped"),
            AccessPolicy::DeviceOnlyBiometryCurrentSet,
        )
        .unwrap();
    store.script_biometry(BiometryScript::EnrollmentChanged);
    assert_eq!(
        store.load_secret(&k, &prompt()).unwrap_err(),
        SecretError::NotFound
    );
    // Even after the user matches again the item is gone: re-enroll with the vault password.
    store.script_biometry(BiometryScript::Match);
    assert_eq!(
        store.load_secret(&k, &prompt()).unwrap_err(),
        SecretError::NotFound
    );
}

#[test]
fn a_matched_preflight_prompt_changes_nothing_about_what_the_store_releases() {
    // S2: BiometricAuth never releases a secret. The prompt matching does not make a failing
    // access control succeed, and the prompt failing does not stop a succeeding one.
    let store = MockSecretStore::new();
    let auth = MockBiometricAuth::matching();
    let k = key("wrapped-dek");
    store
        .store_secret(
            &k,
            &value(b"wrapped"),
            AccessPolicy::DeviceOnlyBiometryCurrentSet,
        )
        .unwrap();

    store.script_biometry(BiometryScript::Fail);
    assert_eq!(
        auth.preflight_prompt(&prompt()).unwrap(),
        PreflightResult::Matched
    );
    assert_eq!(
        store.load_secret(&k, &prompt()).unwrap_err(),
        SecretError::AuthFailed
    );

    store.script_biometry(BiometryScript::Match);
    auth.script(Err(SecretError::AuthFailed));
    assert!(auth.preflight_prompt(&prompt()).is_err());
    assert!(store.load_secret(&k, &prompt()).is_ok());
    assert_eq!(auth.prompts_shown(), 2);
}

#[test]
fn an_unavailable_store_fails_every_call_with_the_reason_and_stores_nothing() {
    for reason in [
        UnavailableReason::MissingEntitlement,
        UnavailableReason::NoSecretService,
        UnavailableReason::Locked,
    ] {
        let store = MockSecretStore::unavailable(reason);
        let expected = SecretError::Unavailable(reason);
        assert_eq!(
            store
                .store_secret(&key("a"), &value(b"x"), AccessPolicy::DeviceOnly)
                .unwrap_err(),
            expected
        );
        assert_eq!(
            store.load_secret(&key("a"), &prompt()).unwrap_err(),
            expected
        );
        assert_eq!(store.delete_secret(&key("a")).unwrap_err(), expected);
        assert!(store.is_empty());
    }
}

#[test]
fn the_unavailable_biometric_reports_it() {
    let auth = MockBiometricAuth::unavailable();
    assert!(!auth.is_available());
    assert!(auth.preflight_prompt(&prompt()).is_err());
}

// ---- runtime biometric availability (xuu, AC A) ------------------------------------------------

const EVERY_STATE: [BiometricAvailability; 5] = [
    BiometricAvailability::Available,
    BiometricAvailability::NotEnrolled,
    BiometricAvailability::NotPresent,
    BiometricAvailability::LockedOut,
    BiometricAvailability::Unknown,
];

#[test]
fn the_store_passes_the_conformance_suite_on_a_device_in_every_biometric_state() {
    for state in EVERY_STATE {
        let device = BiometryDevice::new(state);
        conformance::run(
            &MockSecretStore::on_device(device.clone()),
            &MockBiometricAuth::on(device),
            &format!("mock-{state:?}"),
        );
    }
}

#[test]
fn a_biometric_item_is_refused_with_the_typed_reason_for_each_state_and_the_old_item_survives() {
    for (state, expected) in [
        (
            BiometricAvailability::NotEnrolled,
            SecretError::BiometryNotEnrolled,
        ),
        (
            BiometricAvailability::LockedOut,
            SecretError::BiometryLockedOut,
        ),
        (
            BiometricAvailability::NotPresent,
            SecretError::Unavailable(UnavailableReason::Unsupported),
        ),
    ] {
        let device = BiometryDevice::available();
        let store = MockSecretStore::on_device(device.clone());
        let k = key("wrapped-dek");
        store
            .store_secret(
                &k,
                &value(b"old"),
                AccessPolicy::DeviceOnlyBiometryCurrentSet,
            )
            .unwrap();

        device.set(state);
        assert_eq!(
            store
                .store_secret(
                    &k,
                    &value(b"new"),
                    AccessPolicy::DeviceOnlyBiometryCurrentSet
                )
                .unwrap_err(),
            expected,
            "{state:?}"
        );
        // The refusal came before anything was replaced.
        device.set(BiometricAvailability::Available);
        assert_eq!(
            store.load_secret(&k, &prompt()).unwrap(),
            value(b"old"),
            "{state:?}"
        );
    }
}

#[test]
fn enrolling_a_fingerprint_later_makes_it_available_without_a_restart() {
    let device = BiometryDevice::new(BiometricAvailability::NotEnrolled);
    let store = MockSecretStore::on_device(device.clone());
    let auth = MockBiometricAuth::on(device.clone());
    let k = key("wrapped-dek");

    assert!(!auth.is_available());
    assert_eq!(
        store
            .store_secret(
                &k,
                &value(b"dek"),
                AccessPolicy::DeviceOnlyBiometryCurrentSet
            )
            .unwrap_err(),
        SecretError::BiometryNotEnrolled
    );
    assert!(store.is_empty());

    // Same objects, no restart: the user enrolls a finger.
    device.set(BiometricAvailability::Available);
    assert!(auth.is_available());
    store
        .store_secret(
            &k,
            &value(b"dek"),
            AccessPolicy::DeviceOnlyBiometryCurrentSet,
        )
        .unwrap();
    assert_eq!(store.load_secret(&k, &prompt()).unwrap(), value(b"dek"));
}

#[test]
fn removing_every_fingerprint_invalidates_the_item_and_availability_says_why() {
    let device = BiometryDevice::available();
    let store = MockSecretStore::on_device(device.clone());
    let auth = MockBiometricAuth::on(device.clone());
    let k = key("wrapped-dek");
    store
        .store_secret(
            &k,
            &value(b"dek"),
            AccessPolicy::DeviceOnlyBiometryCurrentSet,
        )
        .unwrap();

    device.set(BiometricAvailability::NotEnrolled);
    store.script_biometry(BiometryScript::EnrollmentChanged);
    assert_eq!(auth.availability(), BiometricAvailability::NotEnrolled);
    // The item reads as not found: the caller deletes (a no-op) and offers the password.
    assert_eq!(
        store.load_secret(&k, &prompt()).unwrap_err(),
        SecretError::NotFound
    );
    assert!(store.is_empty());
}

// ---- the public tag ----------------------------------------------------------------------------

#[test]
fn a_public_tag_reads_without_a_prompt_even_when_the_item_would_need_one() {
    let store = MockSecretStore::new();
    let k = key("wrapped-dek");
    let tag = PublicTag::new(&[7_u8; 32]).unwrap();
    store
        .store_secret_tagged(
            &k,
            &value(b"dek"),
            AccessPolicy::DeviceOnlyBiometryCurrentSet,
            Some(&tag),
        )
        .unwrap();
    // The platform's access control would refuse a read right now; the tag is still readable.
    store.script_biometry(BiometryScript::Fail);
    assert_eq!(
        store.load_secret(&k, &prompt()).unwrap_err(),
        SecretError::AuthFailed
    );
    assert_eq!(store.public_tag(&k).unwrap(), Some(tag));
}

#[test]
fn a_public_tag_is_bounded_and_prints_only_its_length() {
    assert_eq!(PublicTag::new(&[]).unwrap_err(), SecretError::Empty);
    assert_eq!(
        PublicTag::new(&[1_u8; platform_traits::MAX_TAG_BYTES + 1]).unwrap_err(),
        SecretError::TooLarge
    );
    let tag = PublicTag::new(&[0xAB; 32]).unwrap();
    assert_eq!(format!("{tag:?}"), "PublicTag(32 bytes)");
}
