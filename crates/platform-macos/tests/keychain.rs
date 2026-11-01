//! macOS keychain and LocalAuthentication tests.
//!
//! * Always run: the unentitled behavior. An unsigned or ad-hoc development test binary has no
//!   keychain entitlement, so every call must fail with `Unavailable(MissingEntitlement)`, **and
//!   nothing may be written to any keychain** (checked through the login keychain too). This both
//!   proves the typed failure and guarantees an unentitled run can never read as a pass.
//! * `#[ignore]`d manual tests: the entitled behavior (round trip, the stored attributes, the
//!   biometric policy). They need the keychain entitlement, and **a restricted entitlement is honored
//!   only inside a signed app bundle that carries the provisioning profile**: a plain `cargo test`
//!   binary cannot have it, even if signed. They therefore run only from a harness inside such a
//!   bundle: a separate throwaway signed bundle with the real app id and profile (never shipped, never
//!   notarized for distribution; there is deliberately no smoke mode in the shipped app). That harness
//!   is `personal-cfo-md8h2.18` (`scripts/keychain-smoke.sh`); the expected access group is supplied as
//!   `DOHFLOW_KEYCHAIN_ACCESS_GROUP`. They never run in CI. The conformance suite adapts to the
//!   machine's runtime biometric state (xuu), so it runs on every Mac; the tests that need a person at
//!   the sensor stay manual.
#![cfg(target_os = "macos")]

use platform_macos::{own_access_group, MacBiometricAuth, MacBootClock, MacSecretStore};
use platform_traits::{
    conformance, AccessGroup, AccessPolicy, AuthPrompt, BiometricAuth, BiometricAvailability,
    PublicTag, SecretError, SecretKey, SecretStore, SecretValue, UnavailableReason,
};

/// The app's bundle id, which the entitlement's access group must end with.
const BUNDLE_ID: &str = "ai.personalcfo.desktop";

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
    assert_eq!(store.public_tag(&key()).unwrap_err(), MISSING);
    assert_eq!(
        store
            .update_public_tag(&key(), Some(&PublicTag::new(b"marker").unwrap()))
            .unwrap_err(),
        MISSING
    );
    let tag = PublicTag::new(b"fingerprint").unwrap();
    assert_eq!(
        store
            .store_secret_tagged(&key(), &value(), AccessPolicy::DeviceOnly, Some(&tag))
            .unwrap_err(),
        MISSING
    );
}

#[test]
fn the_boot_clock_passes_its_conformance_suite_on_this_mac() {
    // Needs no entitlement: the boot id comes from `kern.bootsessionuuid` and the monotonic clock from
    // `mach_continuous_time`. Stable id, a clock that never goes backwards and moves with real time.
    conformance::run_boot_clock(&MacBootClock::new());
}

#[test]
fn the_boot_id_is_the_kernels_boot_session_uuid() {
    // The same value `sysctl -n kern.bootsessionuuid` prints, parsed the same way, and stable across calls.
    let printed = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "kern.bootsessionuuid"])
        .output()
        .expect("run sysctl");
    let text = String::from_utf8(printed.stdout).unwrap();
    let expected = platform_macos::parse_boot_session_uuid(text.trim()).expect("a valid UUID");
    let clock = MacBootClock::new();
    assert_eq!(
        platform_traits::BootClock::boot_id(&clock).unwrap(),
        expected
    );
}

#[test]
fn an_unentitled_build_has_no_access_group() {
    // A `cargo test` binary is unsigned: no `keychain-access-groups` entitlement, so the group is
    // absent and the app takes the typed MissingEntitlement path for every keychain call (xuu, AC D).
    assert_eq!(own_access_group(BUNDLE_ID), None);
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
fn biometric_availability_is_a_typed_state_and_needs_no_user_interaction() {
    // `Available` on a Touch ID Mac with a finger enrolled, a typed reason elsewhere (this runs on both,
    // and must never show UI). The value depends on the machine; what is asserted is that asking is safe,
    // that `is_available` agrees with it, and that asking twice does not change the answer (not cached
    // state of ours: the system's current answer).
    let auth = MacBiometricAuth::new();
    let state = auth.availability();
    assert_eq!(
        auth.is_available(),
        state == BiometricAvailability::Available
    );
    assert_eq!(auth.availability(), state);
}

// ---------------------------------------------------------------------------------------------
// Manual: the entitled paths, from a harness inside a signed app bundle that carries the keychain
// entitlement and its provisioning profile, with DOHFLOW_KEYCHAIN_ACCESS_GROUP set (see the header).
// ---------------------------------------------------------------------------------------------

fn entitled_store() -> MacSecretStore {
    let group = std::env::var("DOHFLOW_KEYCHAIN_ACCESS_GROUP").expect(
        "set DOHFLOW_KEYCHAIN_ACCESS_GROUP to <TEAMID>.<bundle id> in a signed, entitled build",
    );
    MacSecretStore::new(AccessGroup::new(&group).expect("a team-prefixed access group"))
}

#[test]
#[ignore = "run by the signed harness with --nocapture; needs no entitlement but states what the harness ran"]
fn entitled_report_the_biometric_availability() {
    // The harness greps this line so its PASS block says which conformance branch this Mac took
    // (a typed refusal where biometrics are unavailable, a stored item where they are available), and
    // names the raw LAError code when the state is one this build does not map (`Unknown`).
    let (state, code) = MacBiometricAuth::new().availability_with_code();
    let code = code.map_or_else(|| "none".to_owned(), |code| code.to_string());
    println!("BIOMETRIC_AVAILABILITY={state:?} LA_CODE={code}");
}

#[test]
#[ignore = "needs a signed build with the keychain entitlement"]
fn entitled_the_access_group_is_the_signed_entitlement() {
    // The signed harness bundle carries `<TeamID>.<bundle id>`; the app reads it from itself.
    let group = own_access_group(BUNDLE_ID).expect("a signed, entitled build has an access group");
    assert_eq!(
        Some(group.as_str().to_owned()),
        std::env::var("DOHFLOW_KEYCHAIN_ACCESS_GROUP").ok(),
        "the group read from the entitlement is the one the harness signed with"
    );
    // A different bundle id is not this app's group: absent, not guessed.
    assert_eq!(own_access_group("ai.someone.else"), None);
}

#[test]
#[ignore = "needs a signed build with the keychain entitlement"]
fn entitled_a_public_tag_is_stored_and_read_back_without_authentication() {
    let store = entitled_store();
    let k = SecretKey::new(
        "app.dohflow.keychain-test",
        &format!("tag-{}", std::process::id()),
    )
    .unwrap();
    let tag = PublicTag::new(&[0x5A; 32]).unwrap();
    store
        .store_secret_tagged(&k, &value(), AccessPolicy::DeviceOnly, Some(&tag))
        .expect("store");
    let read = store.public_tag(&k);
    let attrs = store.stored_attributes(&k);
    store.delete_secret(&k).expect("cleanup");
    assert_eq!(read.expect("tag"), Some(tag.clone()));
    assert_eq!(attrs.expect("attributes").public_tag, Some(tag));
}

#[test]
#[ignore = "needs a signed build with the keychain entitlement"]
fn entitled_a_public_tag_is_updated_without_authentication_and_the_secret_is_untouched() {
    let store = entitled_store();
    let k = SecretKey::new(
        "app.dohflow.keychain-test",
        &format!("markers-{}", std::process::id()),
    )
    .unwrap();
    let first = PublicTag::new(&[0x11; 40]).unwrap();
    let second = PublicTag::new(&[0x22; 40]).unwrap();
    store
        .store_secret_tagged(&k, &value(), AccessPolicy::DeviceOnly, Some(&first))
        .expect("store");
    let updated = store.update_public_tag(&k, Some(&second));
    let after_update = store.public_tag(&k);
    let loaded = store.load_secret(&k, &prompt());
    let removed = store.update_public_tag(&k, None);
    let after_removal = store.public_tag(&k);
    let absent = store.update_public_tag(
        &SecretKey::new("app.dohflow.keychain-test", "never-stored").unwrap(),
        Some(&second),
    );
    store.delete_secret(&k).expect("cleanup");
    updated.expect("update");
    assert_eq!(after_update.expect("tag"), Some(second));
    assert_eq!(
        loaded.expect("secret"),
        value(),
        "the secret was not touched"
    );
    removed.expect("remove");
    assert_eq!(after_removal.expect("tag"), None);
    assert_eq!(
        absent.unwrap_err(),
        SecretError::NotFound,
        "never creates an item"
    );
}

#[test]
#[ignore = "needs a Touch ID Mac and a signed entitled build (AC K6: does an attribute-only update of a biometric item prompt?)"]
fn manual_updating_the_tag_of_a_biometric_item_needs_no_authentication() {
    // The owner's Touch ID run. The update runs with a context that may not show UI, so if the platform
    // would need the user to authenticate it FAILS (`Unavailable(Locked)`, errSecInteractionNotAllowed)
    // instead of prompting. Ok means the markers can live in the item's public tag. A failure means they
    // must move to a sibling non-secret item (same class, no access control): tell planning.
    let store = entitled_store();
    assert!(
        MacBiometricAuth::new().is_available(),
        "run this on a Mac with Touch ID enrolled"
    );
    let k = SecretKey::new(
        "app.dohflow.keychain-test",
        &format!("bio-markers-{}", std::process::id()),
    )
    .unwrap();
    let first = PublicTag::new(&[0x33; 40]).unwrap();
    let second = PublicTag::new(&[0x44; 40]).unwrap();
    store
        .store_secret_tagged(
            &k,
            &value(),
            AccessPolicy::DeviceOnlyBiometryCurrentSet,
            Some(&first),
        )
        .expect("store");
    let read = store.public_tag(&k);
    let updated = store.update_public_tag(&k, Some(&second));
    let after = store.public_tag(&k);
    store.delete_secret(&k).expect("cleanup");
    assert_eq!(
        read.expect("reading the tag of a biometric item needs no authentication"),
        Some(first)
    );
    updated.expect("updating the tag of a biometric item needs no authentication");
    assert_eq!(after.expect("tag"), Some(second));
}

#[test]
#[ignore = "needs a signed build with the keychain entitlement"]
fn entitled_a_device_only_item_round_trips() {
    // The always-possible entitled round trip: a device-only item needs no biometrics, so this runs on
    // a Mac without Touch ID too (the signed harness runs it everywhere).
    let store = entitled_store();
    let k = SecretKey::new(
        "app.dohflow.keychain-test",
        &format!("round-trip-{}", std::process::id()),
    )
    .unwrap();
    store
        .store_secret(&k, &value(), AccessPolicy::DeviceOnly)
        .expect("store");
    let loaded = store.load_secret(&k, &prompt());
    store.delete_secret(&k).expect("cleanup");
    assert_eq!(loaded.expect("load"), value());
    assert_eq!(
        store.load_secret(&k, &prompt()).unwrap_err(),
        SecretError::NotFound,
        "deleted means gone"
    );
}

#[test]
#[ignore = "needs a signed build with the keychain entitlement"]
fn entitled_the_store_passes_the_conformance_suite() {
    // On a Mac with no enrolled biometrics (the build Mac) the suite requires the typed refusal for a
    // biometric item and nothing stored; on a Touch ID Mac it requires the store to succeed.
    conformance::run(
        &entitled_store(),
        &MacBiometricAuth::new(),
        &format!("mac-{}", std::process::id()),
    );
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
