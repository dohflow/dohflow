//! The persisted genesis (personal-cfo-1cltt.4 AC5; `vault-format.md` C.6).
//!
//! Enabling Sync on the founding device is `prepare_genesis`, then **persist the records**, then
//! `install_genesis`, then publish. The records are the start of the accepted epoch chain, and the
//! row in `sync_genesis` (class 3, migration 68) is the authority for them: a vault that holds one
//! never mints a second genesis, a retry installs from these exact bytes, and a lost `K_1` or `S` is
//! restored by unsealing them with the device's agreement key.
//!
//! The row is written once and its records never change. Its `state` only moves forward:
//!
//! ```text
//! Persisted -> Installed -> Published
//!     \            \
//!      +------------+--> VaultKnown          (terminal: the service already knows the vault)
//!      +------------+--> GenesisResumeLost   (terminal: the service holds a genesis snapshot
//!                                             this device can no longer send)
//! ```
//!
//! `VaultKnown` records that the service refused this genesis because the vault already exists
//! (R39). Such a genesis is never published, retried or re-minted, and nothing in the vault is
//! discarded: the device bootstraps from the service's snapshot instead.
//!
//! `GenesisResumeLost` records that an earlier attempt of **this** genesis reached the service (it
//! staged the snapshot, or the genesis is installed) and the saved container that was sent is gone,
//! so a fresh export would differ from what the service holds. Nothing is published, minted or
//! discarded, and the state is persisted so that a restart never retries: every retry would seal
//! again and burn a counter. The device shows it in Sync status with the support path; it does not
//! bootstrap on its own, because the vault holds changes the service's snapshot lacks.

use rusqlite::{params, Connection, OptionalExtension};
use sync_core::membership::verify_chain;
use thiserror::Error;
use vault_crypto::sync_counter::verified_mark_of;

use crate::DbError;
use platform_traits::VerifiedMark;

/// How far enabling got. See the module comment for the only moves allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum GenesisState {
    /// The records are durable; no key has been installed from them.
    Persisted,
    /// `K_1` and `S` are installed from the records; nothing has been published.
    Installed,
    /// The service accepted the genesis.
    Published,
    /// The service refused it because the vault already exists. Terminal.
    VaultKnown,
    /// An earlier attempt of this genesis reached the service and the container it sent is gone, so
    /// a fresh export would not be what the service holds. Terminal.
    GenesisResumeLost,
}

impl GenesisState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Persisted => "Persisted",
            Self::Installed => "Installed",
            Self::Published => "Published",
            Self::VaultKnown => "VaultKnown",
            Self::GenesisResumeLost => "GenesisResumeLost",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "Persisted" => Some(Self::Persisted),
            "Installed" => Some(Self::Installed),
            "Published" => Some(Self::Published),
            "VaultKnown" => Some(Self::VaultKnown),
            "GenesisResumeLost" => Some(Self::GenesisResumeLost),
            _ => None,
        }
    }

    /// Whether the state may move to `to` (or already is `to`).
    const fn may_become(self, to: Self) -> bool {
        matches!(
            (self, to),
            (Self::Persisted, Self::Persisted | Self::Installed)
                | (Self::Installed, Self::Installed | Self::Published)
                | (Self::Published, Self::Published)
                | (Self::VaultKnown, Self::VaultKnown)
                | (Self::GenesisResumeLost, Self::GenesisResumeLost)
                | (
                    Self::Persisted | Self::Installed,
                    Self::VaultKnown | Self::GenesisResumeLost
                )
        )
    }
}

/// The genesis this vault persisted.
#[derive(Clone, PartialEq, Eq)]
pub struct PersistedGenesis {
    /// The vault the genesis founds.
    pub vault_id: [u8; 16],
    /// The signed membership record (version 1, `add`).
    pub membership_record: Vec<u8>,
    /// The signed reason-1 epoch record, sealed to the founding device only.
    pub epoch_record: Vec<u8>,
    /// How far enabling got.
    pub state: GenesisState,
}

impl core::fmt::Debug for PersistedGenesis {
    /// The records are signed and sealed, but they are the root of the epoch chain: they are not
    /// printed.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PersistedGenesis")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// Why the genesis could not be read or changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum GenesisStoreError {
    /// The store could not be read or written.
    #[error("the persisted genesis is not available")]
    Unavailable,
    /// A genesis is already persisted and these are not its records. A vault has one genesis.
    #[error("a different genesis is already persisted")]
    Conflict,
    /// There is no persisted genesis to move.
    #[error("no genesis is persisted")]
    NotPersisted,
    /// The state cannot move that way.
    #[error("the genesis state cannot move from {from:?} to {to:?}")]
    BadTransition {
        /// Where it is.
        from: GenesisState,
        /// Where it was asked to go.
        to: GenesisState,
    },
    /// The records offered do not verify as a genesis for this vault (a chain of one membership
    /// record and one epoch record). Nothing was written.
    #[error("the records are not a valid genesis for this vault")]
    Invalid,
    /// The persisted records no longer verify as a chain for the vault, or the row is not what this
    /// module writes: damage after the write. Nothing is installed from them.
    #[error("the persisted genesis does not verify")]
    Corrupt,
}

impl From<DbError> for GenesisStoreError {
    fn from(_: DbError) -> Self {
        Self::Unavailable
    }
}

fn unavailable(_: rusqlite::Error) -> GenesisStoreError {
    GenesisStoreError::Unavailable
}

/// `vault_id`, `membership_record`, `epoch_record` and `state` as the table stores them.
type GenesisRow = (Vec<u8>, Vec<u8>, Vec<u8>, String);

/// The persisted genesis, if any.
///
/// # Errors
/// [`GenesisStoreError::Unavailable`] or [`GenesisStoreError::Corrupt`].
pub fn load(conn: &Connection) -> Result<Option<PersistedGenesis>, GenesisStoreError> {
    let row: Option<GenesisRow> = conn
        .query_row(
            "SELECT vault_id, membership_record, epoch_record, state
             FROM sync_genesis WHERE singleton = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(unavailable)?;
    row.map(|(vault_id, membership_record, epoch_record, state)| {
        Ok(PersistedGenesis {
            vault_id: vault_id
                .try_into()
                .map_err(|_| GenesisStoreError::Corrupt)?,
            membership_record,
            epoch_record,
            state: GenesisState::parse(&state).ok_or(GenesisStoreError::Corrupt)?,
        })
    })
    .transpose()
}

/// Persist the genesis records, durably, before any key is installed from them. Idempotent: the
/// same bytes again change nothing and return the current state, so a crash and a retry land on one
/// genesis. Different records are [`GenesisStoreError::Conflict`], because a vault has one.
///
/// The first write verifies the records as a genesis for `vault_id` and refuses anything else with
/// [`GenesisStoreError::Invalid`], writing nothing. Call it inside the writer's transaction; the caller records the verified-state birth mark in the
/// witness first (C.6 freshness rule 5), so a crash between the two is not read as a restore.
///
/// # Errors
/// [`GenesisStoreError`].
pub fn persist(
    conn: &Connection,
    vault_id: &[u8; 16],
    membership_record: &[u8],
    epoch_record: &[u8],
    now: &str,
) -> Result<GenesisState, GenesisStoreError> {
    if let Some(held) = load(conn)? {
        return if held.vault_id == *vault_id
            && held.membership_record == membership_record
            && held.epoch_record == epoch_record
        {
            Ok(held.state)
        } else {
            Err(GenesisStoreError::Conflict)
        };
    }
    // The first write is checked, so a torn record, or one vault's records filed under another,
    // can never become the vault's one genesis (a later correct genesis would be a Conflict).
    // One membership record and one epoch record verified from the empty state are a genesis by
    // construction: the chain checks refuse any first record that is not version 1.
    verify_chain(vault_id, &[membership_record], &[epoch_record])
        .map_err(|_| GenesisStoreError::Invalid)?;
    conn.execute(
        "INSERT INTO sync_genesis
             (singleton, vault_id, membership_record, epoch_record, state, persisted_at, updated_at)
         VALUES (1, ?1, ?2, ?3, 'Persisted', ?4, ?4)",
        params![vault_id, membership_record, epoch_record, now],
    )
    .map_err(unavailable)?;
    Ok(GenesisState::Persisted)
}

/// Move the persisted genesis forward to `to`. Moving to the state it is already in is a no-op, so
/// a retried step is safe; any other move that the diagram in the module comment does not draw is
/// [`GenesisStoreError::BadTransition`] and changes nothing.
///
/// # Errors
/// [`GenesisStoreError`].
pub fn advance(
    conn: &Connection,
    to: GenesisState,
    now: &str,
) -> Result<GenesisState, GenesisStoreError> {
    let held = load(conn)?.ok_or(GenesisStoreError::NotPersisted)?;
    if !held.state.may_become(to) {
        return Err(GenesisStoreError::BadTransition {
            from: held.state,
            to,
        });
    }
    if held.state != to {
        conn.execute(
            "UPDATE sync_genesis SET state = ?1, updated_at = ?2 WHERE singleton = 1",
            params![to.as_str(), now],
        )
        .map_err(unavailable)?;
    }
    Ok(to)
}

/// The mark of the verified Sync state this vault holds: `None` when it holds none, otherwise the
/// mark of the chain the persisted records verify to (one positioned epoch record, the genesis's).
/// This is the `loaded` value `VerifiedStateGuard::check` compares with the witness.
///
/// # Errors
/// [`GenesisStoreError::Corrupt`] if the persisted records do not verify as a chain for the vault.
pub fn loaded_verified_mark(conn: &Connection) -> Result<Option<VerifiedMark>, GenesisStoreError> {
    let Some(held) = load(conn)? else {
        return Ok(None);
    };
    let state = verify_chain(
        &held.vault_id,
        &[held.membership_record.as_slice()],
        &[held.epoch_record.as_slice()],
    )
    .map_err(|_| GenesisStoreError::Corrupt)?;
    Ok(Some(
        verified_mark_of(&state, 1).ok_or(GenesisStoreError::Corrupt)?,
    ))
}
