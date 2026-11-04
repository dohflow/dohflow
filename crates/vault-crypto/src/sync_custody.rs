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
//!
//! `<vault_id>` is the 32 lowercase hex digits of the vault id. Every item is stored with
//! [`AccessPolicy::DeviceOnly`]: never synchronized, never restored to another device (two devices
//! sharing a fingerprint would reuse AES-GCM nonces, C.2). A tag is public routing data, read
//! without authentication; it is what lets a load notice a damaged or swapped item.
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
use crate::sync_keys::{epoch_key_commitment, tag_key_commitment, DeviceKeys};
use crate::sync_records::{
    accept_records, create_genesis, AcceptError, Accepted, BuildError, GenesisRecords,
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
}

impl core::fmt::Display for Item {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SigningKey => f.write_str("the device signing key"),
            Self::AgreementKey => f.write_str("the device agreement key"),
            Self::EpochKey(epoch) => write!(f, "the epoch {epoch} key"),
            Self::TagSecret => f.write_str("the tag secret"),
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

    /// Store a freshly generated device's keys. Refuses to replace existing ones: a device that
    /// already holds keys for the vault keeps them, because replacing them would change its
    /// fingerprint under a chain that already names the old one.
    ///
    /// # Errors
    /// [`CustodyError::AlreadyPresent`] or [`CustodyError::Store`]. If the second write fails the
    /// first is removed, so a failed call leaves no half of a pair behind.
    pub fn store_device_keys(&self, keys: &DeviceKeys) -> Result<(), CustodyError> {
        for item in [Item::SigningKey, Item::AgreementKey] {
            match self.store.public_tag(&self.key(item)) {
                Ok(_) => return Err(CustodyError::AlreadyPresent(item)),
                Err(SecretError::NotFound) => {}
                Err(error) => return Err(CustodyError::Store(error)),
            }
        }
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

    /// Enable Sync for the vault on this, the founding, device (AC: `K_e` and `S` are generated at
    /// sync-enable and held in the secret store).
    ///
    /// Generates the device's two keys (or reuses a pair a failed earlier attempt left, which no
    /// chain has seen yet), creates the genesis records, and installs `K_1` and `S` **by accepting
    /// the device's own genesis record**, never from the values that sealed it. Returns the records
    /// for the caller to publish; publishing them, and refusing a vault the service already knows
    /// (R39), are the enable path's (`personal-cfo-1cltt.4`).
    ///
    /// # Errors
    /// [`EnableError`].
    pub fn enable_sync(
        &self,
        device_id: [u8; 16],
        issued_at: i64,
    ) -> Result<GenesisRecords, EnableError> {
        if self.has_epoch_key(1)? {
            return Err(EnableError::AlreadyEnabled);
        }
        let keys = if self.has_device_keys()? {
            self.load_device_keys()?
        } else {
            let keys = DeviceKeys::generate().map_err(|_| BuildError::Rng)?;
            self.store_device_keys(&keys)?;
            keys
        };
        let genesis = create_genesis(self.vault_id, device_id, &keys, issued_at)?;
        let accepted = accept_records(
            &VerifiedState::empty(self.vault_id),
            &[&genesis.membership],
            &[&genesis.epoch],
            &keys,
        )?;
        self.install(&accepted)?;
        Ok(genesis)
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
    /// # Errors
    /// [`CustodyError::Missing`], [`CustodyError::Corrupt`] or [`CustodyError::Store`].
    pub fn load_device_keys(&self) -> Result<DeviceKeys, CustodyError> {
        let (signing, signing_tag) = self
            .read(Item::SigningKey)?
            .ok_or(CustodyError::Missing(Item::SigningKey))?;
        let (agreement, agreement_tag) = self
            .read(Item::AgreementKey)?
            .ok_or(CustodyError::Missing(Item::AgreementKey))?;
        let keys = DeviceKeys::from_secrets(signing, agreement)
            .map_err(|_| CustodyError::Corrupt(Item::SigningKey))?;
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

    /// Load `K_e` of one key epoch. It becomes an [`EpochKey`] only through the commitment the
    /// item's tag carries (the one the verified record committed to at install).
    ///
    /// # Errors
    /// [`CustodyError::Missing`], [`CustodyError::Corrupt`] or [`CustodyError::Store`].
    pub fn load_epoch_key(&self, key_epoch: u64) -> Result<EpochKey, CustodyError> {
        let item = Item::EpochKey(key_epoch);
        let (secret, tag) = self.read(item)?.ok_or(CustodyError::Missing(item))?;
        let commitment: [u8; 32] = tag
            .as_bytes()
            .try_into()
            .map_err(|_| CustodyError::Corrupt(item))?;
        EpochKey::from_verified(self.vault_id, key_epoch, secret, &commitment)
            .map_err(|_| CustodyError::Corrupt(item))
    }

    /// Load the tag secret `S`, through the commitment its tag carries.
    ///
    /// # Errors
    /// [`CustodyError::Missing`], [`CustodyError::Corrupt`] or [`CustodyError::Store`].
    pub fn load_tag_secret(&self) -> Result<TagSecret, CustodyError> {
        let (secret, tag) = self
            .read(Item::TagSecret)?
            .ok_or(CustodyError::Missing(Item::TagSecret))?;
        let commitment: [u8; 32] = tag
            .as_bytes()
            .try_into()
            .map_err(|_| CustodyError::Corrupt(Item::TagSecret))?;
        TagSecret::from_verified(secret, &commitment)
            .map_err(|_| CustodyError::Corrupt(Item::TagSecret))
    }
}
