//! The mock passes the shared conformance suite, models the biometric access control separately from
//! the pre-flight prompt, and can play an unavailable platform.

use platform_mock::{BiometryScript, MockBiometricAuth, MockSecretStore};
use platform_traits::{
    conformance, AccessPolicy, AuthPrompt, BiometricAuth, PreflightResult, SecretError, SecretKey,
    SecretStore, SecretValue, UnavailableReason,
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
    conformance::run(&MockSecretStore::new(), "mock-full");
}

#[test]
fn a_mock_without_biometry_passes_it_too_and_refuses_the_biometric_policy() {
    let store = MockSecretStore::without_biometry();
    conformance::run(&store, "mock-nobio");
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
