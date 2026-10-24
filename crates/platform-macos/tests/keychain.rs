//! macOS keychain and LocalAuthentication tests.
//!
//! * Always run: the unentitled behavior. An unsigned or ad-hoc development test binary has no
//!   keychain entitlement, so every call must fail with `Unavailable(MissingEntitlement)`, **and
//!   nothing may be written to any keychain** (checked through the login keychain too). This both
//!   proves the typed failure and guarantees an unentitled run can never read as a pass.
//! * `#[ignore]`d manual tests: the entitled behavior (round trip, the stored attributes, the
//!   biometric policy). They run only in a signed build that carries the keychain entitlement and an
//!   access group, supplied as `DOHFLOW_KEYCHAIN_ACCESS_GROUP` (for example by the release gate of
//!   `personal-cfo-md8h2.17`, or by the owner on a signed build). They never run in CI.
#![cfg(target_os = "macos")]

use platform_macos::{MacBiometricAuth, MacSecretStore};
use platform_traits::{
    conformance, AccessGroup, AccessPolicy, AuthPrompt, BiometricAuth, SecretError, SecretKey,
    SecretStore, SecretValue, UnavailableReason,
};

const MISSING: SecretError = SecretError::Unavailable(UnavailableReason::MissingEntitlement);

fn group() -> AccessGroup {
    AccessGroup::new("TESTTEAM12.app.dohflow.keychain-test").unwrap()
}

fn key() -> SecretKey {
    SecretKey::new(
        "app.dohflow.keychain-test",
        &format!("unentitled-{}", std::process::id()),
    )
    .unwrap()
}

fn prompt() -> AuthPrompt {
    AuthPrompt::new("DohFlow keychain test").unwrap()
}

fn value() -> SecretValue {
    SecretValue::new(b"unentitled-canary").unwrap()
}

#[test]
fn an_unentitled_build_fails_every_call_with_the_typed_error() {
    let store = MacSecretStore::new(group());
    for policy in [
        AccessPolicy::DeviceOnly,
        AccessPolicy::DeviceOnlyBiometryCurrentSet,
    ] {
        assert_eq!(
            store.store_secret(&key(), &value(), policy).unwrap_err(),
            MISSING,
            "{policy:?}"
        );
    }
    assert_eq!(store.load_secret(&key(), &prompt()).unwrap_err(), MISSING);
    assert_eq!(store.delete_secret(&key()).unwrap_err(), MISSING);
    assert_eq!(store.stored_attributes(&key()).unwrap_err(), MISSING);
}

#[test]
fn an_unentitled_build_never_falls_back_to_the_login_keychain() {
    // After the failed stores above, the login (file) keychain must hold nothing under our service.
    // `security find-generic-password` exits non-zero when there is no such item.
    let store = MacSecretStore::new(group());
    let k = key();
    let _ = store.store_secret(&k, &value(), AccessPolicy::DeviceOnly);
    let status = std::process::Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            k.service(),
            "-a",
            k.account(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run /usr/bin/security");
    assert!(
        !status.success(),
        "an item reached the login keychain: a silent fallback (ADR 0002 addendum, decision 2)"
    );
}

#[test]
fn the_capabilities_are_honest() {
    let caps = MacSecretStore::new(group()).capabilities();
    assert!(caps.device_only_class && caps.biometry_current_set);
    assert_eq!(caps.roams_to_other_device, platform_traits::Roaming::Never);
    assert!(
        !caps.rollback_resistant,
        "a keychain is never the anti-rollback witness"
    );
}

#[test]
fn biometric_availability_is_a_bool_and_needs_no_user_interaction() {
    // True on a Touch ID Mac with a finger enrolled, false elsewhere (this runs on both, and must
    // never show UI). Not asserting the value: only that asking is safe.
    let _ = MacBiometricAuth::new().is_available();
}

// ---------------------------------------------------------------------------------------------
// Manual: the entitled paths. `cargo test -p platform-macos --test keychain -- --ignored`
// in a SIGNED build with the keychain entitlement and DOHFLOW_KEYCHAIN_ACCESS_GROUP set.
// ---------------------------------------------------------------------------------------------

fn entitled_store() -> MacSecretStore {
    let group = std::env::var("DOHFLOW_KEYCHAIN_ACCESS_GROUP").expect(
        "set DOHFLOW_KEYCHAIN_ACCESS_GROUP to <TEAMID>.<bundle id> in a signed, entitled build",
    );
    MacSecretStore::new(AccessGroup::new(&group).expect("a team-prefixed access group"))
}

#[test]
#[ignore = "needs a signed build with the keychain entitlement"]
fn entitled_the_store_passes_the_conformance_suite() {
    conformance::run(&entitled_store(), &format!("mac-{}", std::process::id()));
}

#[test]
#[ignore = "needs a signed build with the keychain entitlement"]
fn entitled_a_stored_item_has_the_device_only_class_and_is_not_synchronizable() {
    let store = entitled_store();
    let k = SecretKey::new(
        "app.dohflow.keychain-test",
        &format!("attrs-{}", std::process::id()),
    )
    .unwrap();
    store
        .store_secret(&k, &value(), AccessPolicy::DeviceOnly)
        .expect("store");
    let attrs = store.stored_attributes(&k).expect("attributes");
    store.delete_secret(&k).expect("cleanup");
    // AC8: both attributes, asserted on the stored item.
    assert_eq!(
        attrs.accessible.as_deref(),
        Some("akpu"),
        "WhenPasscodeSetThisDeviceOnly: {attrs:?}"
    );
    assert_eq!(attrs.synchronizable, Some(false), "{attrs:?}");
    assert_eq!(
        attrs.access_group.as_deref(),
        std::env::var("DOHFLOW_KEYCHAIN_ACCESS_GROUP")
            .ok()
            .as_deref(),
        "the item lives in the explicit access group"
    );
}

#[test]
#[ignore = "needs a Touch ID Mac, a signed entitled build, and a person at the sensor"]
fn entitled_a_biometric_item_needs_a_fresh_match() {
    let store = entitled_store();
    let k = SecretKey::new(
        "app.dohflow.keychain-test",
        &format!("bio-{}", std::process::id()),
    )
    .unwrap();
    store
        .store_secret(&k, &value(), AccessPolicy::DeviceOnlyBiometryCurrentSet)
        .expect("store");
    // Touch the sensor when prompted; cancel it once to see AuthCancelled.
    let loaded = store.load_secret(
        &k,
        &AuthPrompt::new("DohFlow keychain test: touch the sensor").unwrap(),
    );
    store.delete_secret(&k).expect("cleanup");
    assert_eq!(loaded.expect("load"), value());
}

#[test]
#[ignore = "needs a Touch ID Mac and a person at the sensor"]
fn manual_preflight_prompt_matches() {
    let auth = MacBiometricAuth::new();
    assert!(auth.is_available());
    assert!(auth
        .preflight_prompt(&AuthPrompt::new("DohFlow keychain test: touch the sensor").unwrap())
        .is_ok());
}
