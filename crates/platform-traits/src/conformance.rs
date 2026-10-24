//! The behavior every [`SecretStore`] implementation must show. Implementations call [`run`] from
//! their own tests; `platform-mock` runs it in CI, `platform-macos` runs it where the platform lets it
//! (and asserts the typed failure where it does not).
//!
//! Panics (assert) on a violation, as a test helper.

use crate::{
    AccessPolicy, AuthPrompt, Roaming, SecretError, SecretKey, SecretStore, SecretValue,
    UnavailableReason,
};

/// Runs the suite against `store`. `namespace` makes the keys unique to this run (callers pass a
/// per-run token) so a leftover item from an earlier run can never satisfy an assertion.
///
/// # Panics
/// On any violation of the contract.
pub fn run(store: &dyn SecretStore, namespace: &str) {
    let prompt = AuthPrompt::new("DohFlow conformance test").expect("prompt");
    let key = |name: &str| {
        SecretKey::new("app.dohflow.conformance", &format!("{namespace}-{name}")).expect("key")
    };
    let value = |bytes: &[u8]| SecretValue::new(bytes).expect("value");

    // capabilities are honest: a keychain-like store is never the anti-rollback witness
    let caps = store.capabilities();
    assert!(
        !caps.rollback_resistant,
        "a secret store is never the anti-rollback witness"
    );
    assert!(
        caps.device_only_class,
        "every implementation provides the device-only class or is not an implementation"
    );

    // never stored -> NotFound; deleting what is not there is fine (idempotent)
    let absent = key("absent");
    assert_eq!(
        store.load_secret(&absent, &prompt).unwrap_err(),
        SecretError::NotFound
    );
    store
        .delete_secret(&absent)
        .expect("delete of an absent item succeeds");
    store.delete_secret(&absent).expect("and again");

    // round trip
    let k = key("roundtrip");
    store
        .store_secret(&k, &value(b"first secret"), AccessPolicy::DeviceOnly)
        .expect("store");
    assert_eq!(
        store.load_secret(&k, &prompt).expect("load"),
        value(b"first secret")
    );

    // replace
    store
        .store_secret(
            &k,
            &value(b"second, longer secret"),
            AccessPolicy::DeviceOnly,
        )
        .expect("replace");
    assert_eq!(
        store.load_secret(&k, &prompt).expect("load after replace"),
        value(b"second, longer secret")
    );

    // two keys do not interfere
    let other = key("other");
    store
        .store_secret(&other, &value(b"other"), AccessPolicy::DeviceOnly)
        .expect("store other");
    assert_eq!(
        store.load_secret(&k, &prompt).expect("k"),
        value(b"second, longer secret")
    );
    assert_eq!(
        store.load_secret(&other, &prompt).expect("other"),
        value(b"other")
    );

    // binary-safe
    let binary = key("binary");
    let bytes: Vec<u8> = (0..=255).collect();
    store
        .store_secret(&binary, &value(&bytes), AccessPolicy::DeviceOnly)
        .expect("store binary");
    assert_eq!(
        store
            .load_secret(&binary, &prompt)
            .expect("binary")
            .expose(),
        &bytes[..]
    );

    // delete, then gone, and delete again
    store.delete_secret(&k).expect("delete");
    assert_eq!(
        store.load_secret(&k, &prompt).unwrap_err(),
        SecretError::NotFound
    );
    store.delete_secret(&k).expect("delete again");

    // the biometric policy is either provided or refused with the typed reason, never weakened
    let bio = key("biometric");
    let result = store.store_secret(
        &bio,
        &value(b"bio"),
        AccessPolicy::DeviceOnlyBiometryCurrentSet,
    );
    if caps.biometry_current_set {
        // Storing under a biometric access control needs no prompt; reading it would.
        result.expect("a store that advertises biometric access control stores it");
    } else {
        assert_eq!(
            result.unwrap_err(),
            SecretError::Unavailable(UnavailableReason::Unsupported),
            "an unsupported policy is refused, never stored with weaker protection"
        );
        assert_eq!(
            store.load_secret(&bio, &prompt).unwrap_err(),
            SecretError::NotFound,
            "and nothing was stored"
        );
    }

    // an item that must never roam says so honestly
    assert!(matches!(
        caps.roams_to_other_device,
        Roaming::Never | Roaming::Unknown | Roaming::May
    ));

    for name in ["other", "binary", "biometric"] {
        let _ = store.delete_secret(&key(name));
    }
}
