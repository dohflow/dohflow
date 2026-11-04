//! Where a device keeps its Sync keys (vault-format.md C.2; personal-cfo-1cltt.2): the device-only
//! secret store, the item layout, and the rules that nothing is ever overwritten in place and an
//! install is all or nothing.

mod common;

use std::sync::Mutex;

use common::{chain, refs, Chain, VAULT};
use platform_mock::MockSecretStore;
use platform_traits::{
    AccessPolicy, AuthPrompt, PublicTag, SecretError, SecretKey, SecretStore, SecretValue,
    StoreCapabilities, UnavailableReason,
};
use sync_core::membership::VerifiedState;
use vault_crypto::sync_custody::{CustodyError, Item, SyncKeyCustody, SERVICE};
use vault_crypto::sync_envelope::{aead_key, ObjectType};
use vault_crypto::sync_keys::{epoch_key_commitment, DeviceKeys};
use vault_crypto::sync_records::{accept_records, Accepted};
use vault_crypto::SecretBytes;

const HEX_VAULT: &str = "11111111111111111111111111111111";

/// A store that records every write's policy and delegates to the in-memory mock.
struct Recording {
    inner: MockSecretStore,
    writes: Mutex<Vec<(String, AccessPolicy)>>,
}

impl Recording {
    fn new() -> Self {
        Self {
            inner: MockSecretStore::new(),
            writes: Mutex::new(Vec::new()),
        }
    }
}

impl SecretStore for Recording {
    fn store_secret_tagged(
        &self,
        key: &SecretKey,
        value: &SecretValue,
        policy: AccessPolicy,
        tag: Option<&PublicTag>,
    ) -> Result<(), SecretError> {
        self.writes.lock().unwrap().push((format!("{key}"), policy));
        self.inner.store_secret_tagged(key, value, policy, tag)
    }
    fn public_tag(&self, key: &SecretKey) -> Result<Option<PublicTag>, SecretError> {
        self.inner.public_tag(key)
    }
    fn load_secret(
        &self,
        key: &SecretKey,
        prompt: &AuthPrompt,
    ) -> Result<SecretValue, SecretError> {
        self.inner.load_secret(key, prompt)
    }
    fn update_public_tag(
        &self,
        key: &SecretKey,
        tag: Option<&PublicTag>,
    ) -> Result<(), SecretError> {
        self.inner.update_public_tag(key, tag)
    }
    fn delete_secret(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.inner.delete_secret(key)
    }
    fn capabilities(&self) -> StoreCapabilities {
        self.inner.capabilities()
    }
}

fn account(suffix: &str) -> SecretKey {
    SecretKey::new(SERVICE, &format!("{HEX_VAULT}.{suffix}")).unwrap()
}

fn accepted_for(c: &Chain, device: &DeviceKeys) -> Accepted {
    accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships),
        &refs(&c.epochs),
        device,
    )
    .unwrap()
}

#[test]
fn a_device_keeps_both_private_keys_device_only_under_the_documented_names() {
    let store = Recording::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    assert!(!custody.has_device_keys().unwrap());
    let keys = DeviceKeys::generate().unwrap();
    custody.store_device_keys(&keys).unwrap();
    assert!(custody.has_device_keys().unwrap());
    let writes = store.writes.lock().unwrap().clone();
    assert_eq!(
        writes,
        [
            (
                format!("{SERVICE}/{HEX_VAULT}.sign"),
                AccessPolicy::DeviceOnly
            ),
            (
                format!("{SERVICE}/{HEX_VAULT}.kem"),
                AccessPolicy::DeviceOnly
            ),
        ]
    );
    assert_eq!(SERVICE, "app.dohflow.sync");
    assert_eq!(
        store
            .public_tag(&account("sign"))
            .unwrap()
            .unwrap()
            .as_bytes(),
        keys.public().ed25519_pk
    );
    assert_eq!(
        store
            .public_tag(&account("kem"))
            .unwrap()
            .unwrap()
            .as_bytes(),
        keys.public().kem_public_key
    );
}

#[test]
fn the_stored_keys_load_back_as_the_same_device() {
    let store = MockSecretStore::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    let keys = DeviceKeys::generate().unwrap();
    custody.store_device_keys(&keys).unwrap();
    let loaded = custody.load_device_keys().unwrap();
    assert_eq!(loaded.public(), keys.public());
    let message = b"a record body";
    assert_eq!(
        loaded.sign(message),
        keys.sign(message),
        "Ed25519 is deterministic"
    );
}

#[test]
fn the_two_device_keys_are_independent() {
    let a = DeviceKeys::generate().unwrap();
    let b = DeviceKeys::generate().unwrap();
    assert_ne!(a.public().ed25519_pk, b.public().ed25519_pk);
    assert_ne!(a.public().kem_public_key, b.public().kem_public_key);
    assert_ne!(a.public().fingerprint, b.public().fingerprint);
    // Within one device the two public keys are unrelated values.
    assert_ne!(a.public().ed25519_pk, a.public().kem_public_key);
    // Swapping the private halves between two devices changes the fingerprint: nothing is derived
    // from a common seed.
    let store_a = MockSecretStore::new();
    let custody_a = SyncKeyCustody::new(&store_a, VAULT);
    custody_a.store_device_keys(&a).unwrap();
    let store_b = MockSecretStore::new();
    let custody_b = SyncKeyCustody::new(&store_b, VAULT);
    custody_b.store_device_keys(&b).unwrap();
    assert_ne!(
        custody_a.load_device_keys().unwrap().public().fingerprint,
        custody_b.load_device_keys().unwrap().public().fingerprint
    );
}

#[test]
fn existing_device_keys_are_never_replaced() {
    let store = MockSecretStore::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    let first = DeviceKeys::generate().unwrap();
    custody.store_device_keys(&first).unwrap();
    let second = DeviceKeys::generate().unwrap();
    assert_eq!(
        custody.store_device_keys(&second).unwrap_err(),
        CustodyError::AlreadyPresent(Item::SigningKey)
    );
    assert_eq!(custody.load_device_keys().unwrap().public(), first.public());
}

#[test]
fn a_missing_key_is_a_typed_error_and_is_never_regenerated() {
    let store = MockSecretStore::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    assert_eq!(
        custody.load_device_keys().unwrap_err(),
        CustodyError::Missing(Item::SigningKey)
    );
    let keys = DeviceKeys::generate().unwrap();
    custody.store_device_keys(&keys).unwrap();
    store.delete_secret(&account("kem")).unwrap();
    assert_eq!(
        custody.load_device_keys().unwrap_err(),
        CustodyError::Missing(Item::AgreementKey)
    );
    assert!(!custody.has_device_keys().unwrap());
    assert_eq!(
        custody.load_epoch_key(1).unwrap_err(),
        CustodyError::Missing(Item::EpochKey(1))
    );
    assert_eq!(
        custody.load_tag_secret().unwrap_err(),
        CustodyError::Missing(Item::TagSecret)
    );
}

#[test]
fn a_swapped_or_damaged_item_is_corrupt_not_trusted() {
    let store = MockSecretStore::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    let keys = DeviceKeys::generate().unwrap();
    custody.store_device_keys(&keys).unwrap();
    // The signing item's tag no longer names its key.
    store
        .update_public_tag(&account("sign"), Some(&PublicTag::new(&[7; 32]).unwrap()))
        .unwrap();
    assert_eq!(
        custody.load_device_keys().unwrap_err(),
        CustodyError::Corrupt(Item::SigningKey)
    );
    // A wrong-sized value.
    store
        .store_secret(
            &account("kem"),
            &SecretValue::new(&[1; 31]).unwrap(),
            AccessPolicy::DeviceOnly,
        )
        .unwrap();
    store
        .update_public_tag(
            &account("sign"),
            Some(&PublicTag::new(&keys.public().ed25519_pk).unwrap()),
        )
        .unwrap();
    assert_eq!(
        custody.load_device_keys().unwrap_err(),
        CustodyError::Corrupt(Item::AgreementKey)
    );
}

// ---- K_e and S ---------------------------------------------------------------------------------------

#[test]
fn an_accepted_walk_installs_one_item_per_epoch_and_one_for_s() {
    let c = chain();
    let store = Recording::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    let accepted = accepted_for(&c, &c.a.keys);
    custody.install(&accepted).unwrap();
    let mut written: Vec<String> = store
        .writes
        .lock()
        .unwrap()
        .iter()
        .map(|w| w.0.clone())
        .collect();
    written.sort();
    assert_eq!(
        written,
        [
            format!("{SERVICE}/{HEX_VAULT}.ke.1"),
            format!("{SERVICE}/{HEX_VAULT}.ke.2"),
            format!("{SERVICE}/{HEX_VAULT}.ke.3"),
            format!("{SERVICE}/{HEX_VAULT}.s"),
        ]
    );
    assert!(store
        .writes
        .lock()
        .unwrap()
        .iter()
        .all(|w| w.1 == AccessPolicy::DeviceOnly));
    // The keys load back through their commitments, and are the ones the chain sealed.
    for epoch in 1..=3 {
        let key = custody.load_epoch_key(epoch).unwrap();
        assert_eq!(key.key_epoch(), epoch);
        assert_eq!(
            aead_key(&key, &c.a.fingerprint(), ObjectType::Envelope).expose(),
            aead_key(
                &common::epoch_key(epoch),
                &c.a.fingerprint(),
                ObjectType::Envelope
            )
            .expose()
        );
    }
    let s = custody.load_tag_secret().unwrap();
    let tag = vault_crypto::sync::idempotency_tag_key(&s, &VAULT);
    let expected = vault_crypto::sync::idempotency_tag_key(&common::tag_secret(0x77), &VAULT);
    assert_eq!(tag.expose(), expected.expose());
}

#[test]
fn installing_the_same_walk_again_writes_nothing() {
    let c = chain();
    let store = Recording::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    custody.install(&accepted_for(&c, &c.b.keys)).unwrap();
    let before = store.writes.lock().unwrap().len();
    custody.install(&accepted_for(&c, &c.b.keys)).unwrap();
    assert_eq!(
        store.writes.lock().unwrap().len(),
        before,
        "no item is rewritten in place"
    );
}

#[test]
fn a_different_value_already_in_the_store_is_a_conflict_and_installs_nothing() {
    let c = chain();
    let store = MockSecretStore::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    // Epoch 2 already holds some other key.
    let other = SecretBytes::new([0xee; 32]);
    store
        .store_secret_tagged(
            &account("ke.2"),
            &SecretValue::new(other.expose()).unwrap(),
            AccessPolicy::DeviceOnly,
            Some(&PublicTag::new(&epoch_key_commitment(&other)).unwrap()),
        )
        .unwrap();
    let error = custody.install(&accepted_for(&c, &c.a.keys)).unwrap_err();
    assert_eq!(error, CustodyError::Conflict(Item::EpochKey(2)));
    // Nothing from the walk was written: not epoch 1 (which came first), not S.
    assert_eq!(
        store.public_tag(&account("ke.1")).unwrap_err(),
        SecretError::NotFound
    );
    assert_eq!(
        store.public_tag(&account("s")).unwrap_err(),
        SecretError::NotFound
    );
    assert_eq!(store.len(), 1);
}

#[test]
fn a_lost_key_item_comes_back_by_unsealing_the_records_again() {
    let c = chain();
    let store = MockSecretStore::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    custody.install(&accepted_for(&c, &c.b.keys)).unwrap();
    store.delete_secret(&account("ke.2")).unwrap();
    assert_eq!(
        custody.load_epoch_key(2).unwrap_err(),
        CustodyError::Missing(Item::EpochKey(2))
    );
    // The store is a cache: the records and the device's agreement key are the root.
    custody.install(&accepted_for(&c, &c.b.keys)).unwrap();
    assert!(custody.load_epoch_key(2).is_ok());
}

#[test]
fn a_key_that_fails_its_stored_commitment_is_corrupt() {
    let c = chain();
    let store = MockSecretStore::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    custody.install(&accepted_for(&c, &c.a.keys)).unwrap();
    // Replace the value, keep the tag: the value no longer matches its commitment.
    let item = account("ke.1");
    let tag = store.public_tag(&item).unwrap().unwrap();
    store
        .store_secret_tagged(
            &item,
            &SecretValue::new(&[5; 32]).unwrap(),
            AccessPolicy::DeviceOnly,
            Some(&tag),
        )
        .unwrap();
    assert_eq!(
        custody.load_epoch_key(1).unwrap_err(),
        CustodyError::Corrupt(Item::EpochKey(1))
    );
}

#[test]
fn an_unavailable_store_fails_with_a_typed_error_and_never_falls_back() {
    let store = MockSecretStore::unavailable(UnavailableReason::NoSecretService);
    let custody = SyncKeyCustody::new(&store, VAULT);
    let keys = DeviceKeys::generate().unwrap();
    assert_eq!(
        custody.store_device_keys(&keys).unwrap_err(),
        CustodyError::Store(SecretError::Unavailable(UnavailableReason::NoSecretService))
    );
    assert!(store.is_empty());
}

#[test]
fn errors_name_the_item_and_never_a_value() {
    let shown = [
        CustodyError::Missing(Item::SigningKey),
        CustodyError::Conflict(Item::EpochKey(7)),
        CustodyError::Corrupt(Item::TagSecret),
        CustodyError::AlreadyPresent(Item::AgreementKey),
    ]
    .map(|e| format!("{e} / {e:?}"));
    for line in shown {
        assert!(!line.contains("0x"), "{line}");
    }
}

// ---- enabling Sync ---------------------------------------------------------------------------------

#[test]
fn enabling_sync_generates_and_stores_every_key_and_installs_them_by_accepting_the_genesis() {
    let store = Recording::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    let genesis = custody
        .enable_sync([0xa1; 16], 1_700_000_000_000_000)
        .unwrap();
    // The records verify as a chain.
    let state =
        sync_core::membership::verify_chain(&VAULT, &[&genesis.membership], &[&genesis.epoch])
            .unwrap();
    assert_eq!(state.key_epoch(), 1);
    // Four items, all device-only: the two device keys, K_1 and S.
    let mut written: Vec<String> = store
        .writes
        .lock()
        .unwrap()
        .iter()
        .map(|w| w.0.clone())
        .collect();
    written.sort();
    assert_eq!(
        written,
        [
            format!("{SERVICE}/{HEX_VAULT}.ke.1"),
            format!("{SERVICE}/{HEX_VAULT}.kem"),
            format!("{SERVICE}/{HEX_VAULT}.s"),
            format!("{SERVICE}/{HEX_VAULT}.sign"),
        ]
    );
    assert!(store
        .writes
        .lock()
        .unwrap()
        .iter()
        .all(|w| w.1 == AccessPolicy::DeviceOnly));
    // What was installed is what the record seals: re-accepting the genesis with the stored device
    // key yields the same K_1 as the item.
    let keys = custody.load_device_keys().unwrap();
    let again = accept_records(
        &VerifiedState::empty(VAULT),
        &[&genesis.membership],
        &[&genesis.epoch],
        &keys,
    )
    .unwrap();
    let stored = custody.load_epoch_key(1).unwrap();
    assert_eq!(
        aead_key(&stored, &keys.public().fingerprint, ObjectType::Envelope).expose(),
        aead_key(
            &again.installs[0].epoch_key,
            &keys.public().fingerprint,
            ObjectType::Envelope
        )
        .expose()
    );
    custody.load_tag_secret().unwrap();
}

#[test]
fn enabling_twice_is_refused_and_changes_nothing() {
    let store = Recording::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    custody.enable_sync([0xa1; 16], 1).unwrap();
    let writes = store.writes.lock().unwrap().len();
    assert_eq!(
        custody.enable_sync([0xa1; 16], 1).unwrap_err(),
        vault_crypto::sync_custody::EnableError::AlreadyEnabled
    );
    assert_eq!(store.writes.lock().unwrap().len(), writes);
}

#[test]
fn enabling_after_a_failed_attempt_reuses_the_device_keys_it_left() {
    let store = MockSecretStore::new();
    let custody = SyncKeyCustody::new(&store, VAULT);
    let keys = DeviceKeys::generate().unwrap();
    custody.store_device_keys(&keys).unwrap();
    let genesis = custody.enable_sync([0xa1; 16], 1).unwrap();
    let body = sync_core::membership::EpochBody::decode(
        &sync_core::membership::SignedRecord::parse(&genesis.epoch, 1 << 22)
            .unwrap()
            .body,
    )
    .unwrap();
    assert_eq!(body.sealer_fingerprint, keys.public().fingerprint);
}

#[test]
fn enabling_on_an_unavailable_store_fails_and_stores_no_key_anywhere_else() {
    let store = MockSecretStore::unavailable(UnavailableReason::Locked);
    let custody = SyncKeyCustody::new(&store, VAULT);
    assert!(custody.enable_sync([0xa1; 16], 1).is_err());
    assert!(store.is_empty());
}
