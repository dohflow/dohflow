//! Where a device keeps its Sync keys (`docs/architecture/vault-format.md` C.2;
//! personal-cfo-1cltt.2): in the platform [`SecretStore`], outside the vault, device-only.
//!
//! # Layout
//!
//! One store service, `app.dohflow.sync`, and one item per secret, named by the vault:
//!
//! | account | holds | public tag |
//! |---|---|---|
//! | `<vault_id>.sign` | the device's Ed25519 seed (32 bytes) | its Ed25519 public key |
//! | `<vault_id>.kem` | the device's X25519 private key (32 bytes) | its X25519 public key |
//! | `<vault_id>.ke.<epoch>` | `K_e` of that key epoch (32 bytes) | its commitment (`epoch_key_commitment`) |
//! | `<vault_id>.s` | the tag secret `S` (32 bytes) | its commitment (`tag_key_commitment`) |
//! | `<vault_id>.genesis` | the id of the genesis epoch record this device installed | the same id |
//!
//! `<vault_id>` is the 32 lowercase hex digits of the vault id. Every item is stored with
//! [`AccessPolicy::DeviceOnly`]: never synchronized, never restored to another device (two devices
//! sharing a fingerprint would reuse AES-GCM nonces, C.2). A tag is public routing data, read
//! without authentication. It is a **damage check only**: a value and its tag replaced together
//! agree with each other. What a stored `K_e` or `S` must match is the **verified chain's**
//! commitment, so [`SyncKeyCustody::load_epoch_key`] and [`SyncKeyCustody::load_tag_secret`] take
//! the `VerifiedState` and refuse a self-consistent swap as a conflict.
//!
//! # The store is a cache of the records
//!
//! `K_e` and `S` are always recoverable by unsealing the epoch records again with the device's
//! agreement key (see [`crate::sync_records::accept_records`]). So the agreement key is the root, and
//! a lost or damaged `K_e`/`S` item costs a re-unseal, not a re-enrollment.
//!
//! # Never overwritten in place
//!
//! [`SecretStore::store_secret`] may leave **no** item when a write fails after the replacement has
//! begun. So nothing here rewrites an item: one item per `(vault, key_epoch)` for `K_e`, one for `S`,
//! and a second install of the same epoch finds the item, compares its commitment with the verified
//! record's, and does nothing. A different value is [`CustodyError::Conflict`], a hard stop.
//!
//! # Install is all or nothing for the walk
//!
//! [`SyncKeyCustody::install`] checks every item the walk would write **before** it writes any, so a
//! conflict or a damaged item installs nothing. Past the checks only the platform can fail; a write
//! that fails midway leaves complete, valid items behind (each is a cache entry that a retry finds
//! and skips), never a half-written one that a load would accept.
//!
//! No error here names a key byte; an item is named by its [`Item`], never by its value.

use platform_traits::{
    AccessPolicy, AuthPrompt, PublicTag, SecretError, SecretKey, SecretStore, SecretValue,
};

use crate::secret::SecretBytes;
use crate::sync::TagSecret;
use crate::sync_envelope::EpochKey;
use crate::sync_keys::{epoch_key_commitment, tag_key_commitment, DeviceKeyError, DeviceKeys};
use crate::sync_records::{
    accept_records, create_genesis_with, fresh_epoch_key, fresh_tag_secret, AcceptError, Accepted,
    BuildError, GenesisRecords,
};
use sync_core::membership::VerifiedState;

/// The store service name of every Sync item.
pub const SERVICE: &str = "app.dohflow.sync";

/// Which item an error is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    /// The device's Ed25519 seed.
    SigningKey,
    /// The device's X25519 private key.
    AgreementKey,
    /// `K_e` of the named key epoch.
    EpochKey(u64),
    /// The tag secret `S`.
    TagSecret,
    /// The marker that this device installed a genesis: the id of the genesis epoch record.
    GenesisMarker,
}

impl core::fmt::Display for Item {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SigningKey => f.write_str("the device signing key"),
            Self::AgreementKey => f.write_str("the device agreement key"),
            Self::EpochKey(epoch) => write!(f, "the epoch {epoch} key"),
            Self::TagSecret => f.write_str("the tag secret"),
            Self::GenesisMarker => f.write_str("the genesis marker"),
        }
    }
}

/// Why custody did not read or write.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CustodyError {
    /// The item is not in the store (a typed error, never a regenerated key).
    #[error("{0} is not in the secret store")]
    Missing(Item),
    /// The item is already there and was not replaced.
    #[error("{0} is already in the secret store")]
    AlreadyPresent(Item),
    /// The store holds a different value than the verified record commits to.
    #[error("{0} in the secret store differs from the verified record")]
    Conflict(Item),
    /// The stored item is damaged: a wrong size, a missing or mismatched tag, or a key that fails
    /// its commitment.
    #[error("{0} in the secret store is damaged")]
    Corrupt(Item),
    /// The verified chain has no commitment for the item (an epoch it does not contain, or no
    /// epoch record at all), so a stored key cannot be checked against it.
    #[error("{0} is not in the verified chain")]
    NotInChain(Item),
    /// The platform store failed.
    #[error("the secret store failed: {0}")]
    Store(SecretError),
}

/// Why enabling Sync on this device did not complete.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EnableError {
    /// This device already holds the genesis key for the vault.
    #[error("sync is already enabled on this device for the vault")]
    AlreadyEnabled,
    /// Custody refused or failed.
    #[error(transparent)]
    Custody(#[from] CustodyError),
    /// The genesis could not be built.
    #[error(transparent)]
    Build(#[from] BuildError),
    /// The device's own acceptance of its genesis failed.
    #[error(transparent)]
    Accept(#[from] AcceptError),
}

/// A device's Sync keys in the platform secret store, for one vault.
pub struct SyncKeyCustody<'a> {
    store: &'a dyn SecretStore,
    vault_id: [u8; 16],
    prompt: AuthPrompt,
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

impl<'a> SyncKeyCustody<'a> {
    /// Custody for `vault_id` in `store`.
    #[must_use]
    pub fn new(store: &'a dyn SecretStore, vault_id: [u8; 16]) -> Self {
        Self {
            store,
            vault_id,
            prompt: AuthPrompt::new("Unlock your DohFlow Sync keys")
                .expect("the fixed prompt is valid"),
        }
    }

    fn key(&self, item: Item) -> SecretKey {
        let vault = hex(&self.vault_id);
        let account = match item {
            Item::SigningKey => format!("{vault}.sign"),
            Item::AgreementKey => format!("{vault}.kem"),
            Item::EpochKey(epoch) => format!("{vault}.ke.{epoch}"),
            Item::TagSecret => format!("{vault}.s"),
            Item::GenesisMarker => format!("{vault}.genesis"),
        };
        SecretKey::new(SERVICE, &account).expect("a fixed service and a hex account are valid")
    }

    /// The item's secret and its public tag, or `None` if it is absent.
    fn read(&self, item: Item) -> Result<Option<(SecretBytes<32>, PublicTag)>, CustodyError> {
        let key = self.key(item);
        let tag = match self.store.public_tag(&key) {
            Ok(tag) => tag,
            Err(SecretError::NotFound) => return Ok(None),
            Err(error) => return Err(CustodyError::Store(error)),
        };
        let value = match self.store.load_secret(&key, &self.prompt) {
            Ok(value) => value,
            Err(SecretError::NotFound) => return Ok(None),
            Err(error) => return Err(CustodyError::Store(error)),
        };
        let secret =
            SecretBytes::<32>::from_slice(value.expose()).ok_or(CustodyError::Corrupt(item))?;
        Ok(Some((secret, tag.ok_or(CustodyError::Corrupt(item))?)))
    }

    fn write(
        &self,
        item: Item,
        secret: &SecretBytes<32>,
        tag: &[u8; 32],
    ) -> Result<(), CustodyError> {
        let value = SecretValue::new(secret.expose()).map_err(CustodyError::Store)?;
        let tag = PublicTag::new(tag).map_err(CustodyError::Store)?;
        self.store
            .store_secret_tagged(
                &self.key(item),
                &value,
                AccessPolicy::DeviceOnly,
                Some(&tag),
            )
            .map_err(CustodyError::Store)
    }

    /// Whether this device already has Sync keys for the vault (both private halves present),
    /// asked without authentication.
    ///
    /// # Errors
    /// [`CustodyError::Store`].
    pub fn has_device_keys(&self) -> Result<bool, CustodyError> {
        let present = |item| match self.store.public_tag(&self.key(item)) {
            Ok(_) => Ok(true),
            Err(SecretError::NotFound) => Ok(false),
            Err(error) => Err(CustodyError::Store(error)),
        };
        Ok(present(Item::SigningKey)? && present(Item::AgreementKey)?)
    }

    /// Store a freshly generated device's keys. Refuses to replace **any** key that is already
    /// there, a lone half included: a device that holds a key for the vault keeps it, because
    /// replacing it would change the device's fingerprint under a chain that may already name the
    /// old one. (Only [`Self::prepare_genesis`], for a vault this device has never published, may
    /// replace a lone half it left behind itself.)
    ///
    /// # Errors
    /// [`CustodyError::AlreadyPresent`] or [`CustodyError::Store`]. If the second write fails the
    /// first is removed (best effort), so a failed call leaves no half of a pair behind.
    pub fn store_device_keys(&self, keys: &DeviceKeys) -> Result<(), CustodyError> {
        for item in [Item::SigningKey, Item::AgreementKey] {
            if self.holds(item)? {
                return Err(CustodyError::AlreadyPresent(item));
            }
        }
        self.write_device_keys(keys)
    }

    fn holds(&self, item: Item) -> Result<bool, CustodyError> {
        match self.store.public_tag(&self.key(item)) {
            Ok(_) => Ok(true),
            Err(SecretError::NotFound) => Ok(false),
            Err(error) => Err(CustodyError::Store(error)),
        }
    }

    fn write_device_keys(&self, keys: &DeviceKeys) -> Result<(), CustodyError> {
        let public = keys.public();
        self.write(Item::SigningKey, keys.signing_secret(), &public.ed25519_pk)?;
        if let Err(error) = self.write(
            Item::AgreementKey,
            keys.agreement_secret(),
            &public.kem_public_key,
        ) {
            // Best effort: the failure being reported is the write's, not the cleanup's.
            let _ = self.store.delete_secret(&self.key(Item::SigningKey));
            return Err(error);
        }
        Ok(())
    }

    /// The first half of enabling Sync on this, the founding, device: make the device's keys if it
    /// has none, and build and self-accept a genesis. **It writes neither `K_1` nor `S`.** The
    /// records seal a fresh `K_1` and `S` that are not kept anywhere: they are recovered by
    /// unsealing the records with the device's agreement key, which is the root (see the module
    /// doc).
    ///
    /// The order of enabling is: `prepare_genesis`, then the caller **persists the records**
    /// (`personal-cfo-1cltt.4`: the outbox that holds them until they are published), then
    /// [`Self::install_genesis`]. A crash before the records are persisted loses nothing that was
    /// written, so the next `prepare_genesis` simply builds another genesis; a crash after they are
    /// persisted and before the install is finished by installing the persisted records. At no
    /// point can `K_1` and `S` be in the store with no record that carries them.
    ///
    /// A lone half of the device keys, which only an earlier unfinished `prepare_genesis` of this
    /// device can have left, is replaced: nothing was ever published under it.
    ///
    /// # Errors
    /// [`EnableError::AlreadyEnabled`] if `K_1` or `S` is already held (an install happened, so
    /// the records were persisted first); otherwise [`EnableError`].
    pub fn prepare_genesis(
        &self,
        device_id: [u8; 16],
        issued_at: i64,
    ) -> Result<GenesisRecords, EnableError> {
        if self.holds(Item::GenesisMarker)?
            || self.holds(Item::EpochKey(1))?
            || self.holds(Item::TagSecret)?
        {
            return Err(EnableError::AlreadyEnabled);
        }
        let keys = if self.has_device_keys()? {
            self.load_device_keys()?
        } else {
            let keys = DeviceKeys::generate().map_err(|_| BuildError::Rng)?;
            for item in [Item::SigningKey, Item::AgreementKey] {
                if self.holds(item)? {
                    self.store
                        .delete_secret(&self.key(item))
                        .map_err(CustodyError::Store)?;
                }
            }
            self.write_device_keys(&keys)?;
            keys
        };
        let epoch_key = fresh_epoch_key(self.vault_id, 1)?;
        let tag_secret = fresh_tag_secret()?;
        Ok(create_genesis_with(
            self.vault_id,
            device_id,
            &keys,
            issued_at,
            &epoch_key,
            &tag_secret,
        )?)
    }

    /// The second half of enabling: accept the (persisted) genesis records with this device's own
    /// keys and install `K_1` and `S` from them, never from the values that sealed them. Idempotent:
    /// installing records already installed writes nothing, so a crash part-way is finished by
    /// calling it again with the same records. Records whose keys differ from what the store
    /// already holds are a [`CustodyError::Conflict`].
    ///
    /// # Errors
    /// [`EnableError`].
    pub fn install_genesis(&self, genesis: &GenesisRecords) -> Result<(), EnableError> {
        let keys = self.load_device_keys()?;
        let accepted = accept_records(
            &VerifiedState::empty(self.vault_id),
            &[&genesis.membership],
            &[&genesis.epoch],
            &keys,
        )?;
        // The marker goes first, so any partial install already blocks `prepare_genesis`: from the
        // moment a key could be in the store, a second genesis cannot be made.
        let genesis_id: [u8; 32] = accepted
            .installs
            .first()
            .map(|install| install.record_id)
            .ok_or(CustodyError::Corrupt(Item::GenesisMarker))?;
        // The marker's public tag holds the same id, so it is read without releasing the secret:
        // a platform store never prompts for it.
        let held = match self.store.public_tag(&self.key(Item::GenesisMarker)) {
            Ok(tag) => Some(tag.ok_or(CustodyError::Corrupt(Item::GenesisMarker))?),
            Err(SecretError::NotFound) => None,
            Err(error) => return Err(CustodyError::Store(error).into()),
        };
        match held {
            Some(tag) if tag.as_bytes() != &genesis_id[..] => {
                return Err(CustodyError::Conflict(Item::GenesisMarker).into());
            }
            Some(_) => {}
            None => self.write(
                Item::GenesisMarker,
                &SecretBytes::<32>::from_slice(&genesis_id)
                    .ok_or(CustodyError::Corrupt(Item::GenesisMarker))?,
                &genesis_id,
            )?,
        }
        self.install(&accepted)?;
        Ok(())
    }

    /// Whether `K_e` of `key_epoch` is held, asked without authentication.
    ///
    /// # Errors
    /// [`CustodyError::Store`].
    pub fn has_epoch_key(&self, key_epoch: u64) -> Result<bool, CustodyError> {
        match self.store.public_tag(&self.key(Item::EpochKey(key_epoch))) {
            Ok(_) => Ok(true),
            Err(SecretError::NotFound) => Ok(false),
            Err(error) => Err(CustodyError::Store(error)),
        }
    }

    /// Load the device's keys, only when needed. A missing half is a typed error naming it; a half
    /// whose public key no longer matches its tag is damaged.
    ///
    /// The tag is the store's own claim, so before using the keys for anything that matters the
    /// caller compares the loaded `public().fingerprint` with the fingerprint the **verified
    /// chain** holds for this device (`VerifiedState::members`): keys the chain does not name are
    /// not this device's enrollment.
    ///
    /// # Errors
    /// [`CustodyError::Missing`], [`CustodyError::Corrupt`] or [`CustodyError::Store`].
    pub fn load_device_keys(&self) -> Result<DeviceKeys, CustodyError> {
        let (signing, signing_tag) = self
            .read(Item::SigningKey)?
            .ok_or(CustodyError::Missing(Item::SigningKey))?;
        let (agreement, agreement_tag) = self
            .read(Item::AgreementKey)?
            .ok_or(CustodyError::Missing(Item::AgreementKey))?;
        let keys = DeviceKeys::from_secrets(signing, agreement).map_err(|error| match error {
            DeviceKeyError::UnacceptableAgreementKey => CustodyError::Corrupt(Item::AgreementKey),
            _ => CustodyError::Corrupt(Item::SigningKey),
        })?;
        let public = keys.public();
        if signing_tag.as_bytes() != public.ed25519_pk {
            return Err(CustodyError::Corrupt(Item::SigningKey));
        }
        if agreement_tag.as_bytes() != public.kem_public_key {
            return Err(CustodyError::Corrupt(Item::AgreementKey));
        }
        Ok(keys)
    }

    /// Install every key of a verified walk. See "Install is all or nothing" in the module doc.
    ///
    /// # Errors
    /// [`CustodyError::Conflict`], [`CustodyError::Corrupt`] or [`CustodyError::Store`].
    pub fn install(&self, accepted: &Accepted) -> Result<(), CustodyError> {
        // Phase 1, no writes: what is already there must be exactly what the record commits to.
        let mut pending: Vec<(Item, &SecretBytes<32>, [u8; 32])> = Vec::new();
        for install in &accepted.installs {
            let item = Item::EpochKey(install.epoch_key.key_epoch());
            let secret = install.epoch_key.secret();
            let commitment = epoch_key_commitment(secret);
            if !self.already_holds(item, &commitment, epoch_key_commitment)? {
                pending.push((item, secret, commitment));
            }
        }
        if let Some(tag_secret) = &accepted.tag_secret {
            let secret = tag_secret.secret();
            let commitment = tag_key_commitment(secret);
            if !self.already_holds(Item::TagSecret, &commitment, tag_key_commitment)? {
                pending.push((Item::TagSecret, secret, commitment));
            }
        }
        // Phase 2: write what is absent.
        for (item, secret, commitment) in pending {
            self.write(item, secret, &commitment)?;
        }
        Ok(())
    }

    /// Whether `item` is present with a value whose commitment is `expected`. A present item with a
    /// different value is a conflict; a present item that fails its own tag is damaged.
    fn already_holds(
        &self,
        item: Item,
        expected: &[u8; 32],
        commit: fn(&SecretBytes<32>) -> [u8; 32],
    ) -> Result<bool, CustodyError> {
        let Some((existing, tag)) = self.read(item)? else {
            return Ok(false);
        };
        let held = commit(&existing);
        if tag.as_bytes() != held {
            return Err(CustodyError::Corrupt(item));
        }
        if &held == expected {
            Ok(true)
        } else {
            Err(CustodyError::Conflict(item))
        }
    }

    /// Load `K_e` of one key epoch, checked against **the verified chain's** commitment for that
    /// epoch. The item's own public tag is only a damage check: a value and a tag replaced together
    /// agree with each other, so the tag alone proves nothing about which key it is.
    ///
    /// # Errors
    /// [`CustodyError::Missing`], [`CustodyError::Corrupt`] (the value does not match its own tag),
    /// [`CustodyError::Conflict`] (it does not match the chain), [`CustodyError::NotInChain`] or
    /// [`CustodyError::Store`].
    pub fn load_epoch_key(
        &self,
        chain: &VerifiedState,
        key_epoch: u64,
    ) -> Result<EpochKey, CustodyError> {
        let item = Item::EpochKey(key_epoch);
        let expected = chain
            .epoch_key_commitment(key_epoch)
            .ok_or(CustodyError::NotInChain(item))?;
        let (secret, tag) = self.read(item)?.ok_or(CustodyError::Missing(item))?;
        Self::check_tag(item, &secret, &tag, epoch_key_commitment)?;
        EpochKey::from_verified(self.vault_id, key_epoch, secret, &expected)
            .map_err(|_| CustodyError::Conflict(item))
    }

    /// Load the tag secret `S`, checked against the verified chain's commitment (see
    /// [`Self::load_epoch_key`]).
    ///
    /// # Errors
    /// As [`Self::load_epoch_key`].
    pub fn load_tag_secret(&self, chain: &VerifiedState) -> Result<TagSecret, CustodyError> {
        let expected = chain
            .tag_key_commitment()
            .ok_or(CustodyError::NotInChain(Item::TagSecret))?;
        let (secret, tag) = self
            .read(Item::TagSecret)?
            .ok_or(CustodyError::Missing(Item::TagSecret))?;
        Self::check_tag(Item::TagSecret, &secret, &tag, tag_key_commitment)?;
        TagSecret::from_verified(secret, &expected)
            .map_err(|_| CustodyError::Conflict(Item::TagSecret))
    }

    /// The item's value must hash to the commitment its own tag carries; otherwise the item is
    /// damaged.
    fn check_tag(
        item: Item,
        secret: &SecretBytes<32>,
        tag: &PublicTag,
        commit: fn(&SecretBytes<32>) -> [u8; 32],
    ) -> Result<(), CustodyError> {
        if tag.as_bytes() == commit(secret) {
            Ok(())
        } else {
            Err(CustodyError::Corrupt(item))
        }
    }
}
