//! The cross-device identity a committed import row may carry (`personal-cfo-0g528.3`, ADR 0074
//! Addendum J items 3 and 4), as it travels in `CommitImportedTransaction`'s payload.
//!
//! The payload carries the identity's components and its derived id; it carries no snapshot. Apply
//! recomputes the id from the components, binds the components to the row being committed (the source
//! record and its batch), looks the identity up in `import_identities`, and computes the comparison
//! snapshot itself, so a payload cannot forge any of it.

use uuid::Uuid;

/// What an identity is made of, by kind.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportIdentityComponents {
    /// A provider transaction: ADR 0013-A section 9's three components.
    Provider {
        /// The batch's `connector_connection_id`.
        connection_id: Uuid,
        /// The provider's account key, as the adapter emitted it.
        provider_account_key: String,
        /// The provider's transaction id (the source record's `external_id`).
        provider_txn_id: String,
    },
    /// A file row: ADR 0013-A section 12's two components.
    File {
        /// The file's content fingerprint, in its `sha256:` plus 64 lowercase hex form.
        file_fingerprint: String,
        /// The row's one-based ordinal in the file.
        row_ordinal: u32,
    },
}

/// The identity a commit claims for its row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportIdentityClaim {
    /// The derived id of the components (section 9 or section 12), which apply recomputes.
    pub identity_id: Uuid,
    /// The components it derives from.
    pub components: ImportIdentityComponents,
}
