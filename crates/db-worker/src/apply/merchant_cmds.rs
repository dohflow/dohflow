//! `MintMerchantAnchors` and `ApplyMerchantMemory` (personal-cfo-5ymg8.5).
//!
//! Both commands carry the OUTCOME of a heuristic the issuing device ran (the read-only planners in
//! `merchant_grouping.rs` and `merchant_memory.rs`). Apply validates and writes it; it never runs
//! `normalize_merchant`, `brand_anchor` or any other tunable rule, so a changed threshold or a device
//! on another engine version cannot change what a logged command writes. Both are Base-dependent
//! (ADR 0073 Addendum I): what is skipped depends on rows already in the vault, and the skip rules
//! below are the whole resolution.

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{CommandMeta, DbError, MerchantAnchor, MerchantAssignment, RejectCode};

/// The most entries one command carries. The kernel chunks a larger plan.
pub const MAX_MERCHANT_COMMAND_ENTRIES: usize = 5_000;
/// The longest `display_name` of a minted identity, in UTF-8 bytes.
pub const MAX_MERCHANT_DISPLAY_NAME_BYTES: usize = 128;
/// The confidence recorded on an auto-minted prefix alias (below the 10000 of a seed or user
/// mapping, so a weak grouping stays auditable and reversible, ADR 0030).
const AUTO_ALIAS_CONFIDENCE_BPS: i64 = 8000;

/// The identity id of an earned brand. FROZEN: vaults already hold identities minted under this
/// formula, so changing it needs a new command variant, never an edit here.
pub(crate) fn anchor_identity_id(key: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("merchant-auto:{key}").as_bytes(),
    )
}

/// Applies [`WriteCommand::MintMerchantAnchors`](crate::WriteCommand).
///
/// Per anchor, in payload order: if `merchant_aliases` already holds the key (any source), the
/// anchor is skipped whole, so an identity is never left without the alias that points at it and a
/// user's own mapping is never clobbered; otherwise the identity (the first one sequenced wins its
/// `display_name`) and its `prefix` alias are inserted.
pub(crate) fn write_merchant_anchors(
    tx: &Connection,
    meta: &CommandMeta,
    anchors: &[MerchantAnchor],
) -> Result<Uuid, DbError> {
    if anchors.is_empty() || anchors.len() > MAX_MERCHANT_COMMAND_ENTRIES {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            format!("a mint carries 1 to {MAX_MERCHANT_COMMAND_ENTRIES} anchors"),
        ));
    }
    if anchors.windows(2).any(|pair| pair[0].key >= pair[1].key) {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "the anchors are not in strictly ascending key order".to_owned(),
        ));
    }
    for anchor in anchors {
        if anchor.key.is_empty() || anchor.key.len() > crate::MAX_ALIAS_KEY_BYTES {
            return Err(DbError::reject(
                RejectCode::PayloadInvalid,
                "an anchor key is empty or wider than the replicated key width".to_owned(),
            ));
        }
        if anchor.display_name.trim().is_empty()
            || anchor.display_name.len() > MAX_MERCHANT_DISPLAY_NAME_BYTES
        {
            return Err(DbError::reject(
                RejectCode::PayloadInvalid,
                "an anchor display name is empty or too long".to_owned(),
            ));
        }
    }

    let now = crate::apply::issued_at_text(meta);
    for anchor in anchors {
        let held: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM merchant_aliases WHERE normalized_key = ?1)",
            [&anchor.key],
            |r| r.get(0),
        )?;
        if held {
            continue;
        }
        let id = anchor_identity_id(&anchor.key);
        tx.execute(
            "INSERT OR IGNORE INTO merchant_identities
                (id, display_name, default_category_id, is_ambiguous, source, created_at)
             VALUES (?1, ?2, NULL, 0, 'auto', ?3)",
            params![id, anchor.display_name, now],
        )?;
        tx.execute(
            "INSERT INTO merchant_aliases
                (normalized_key, merchant_identity_id, source, confidence_bps, match_type, created_at)
             VALUES (?1, ?2, 'auto', ?3, 'prefix', ?4)",
            params![anchor.key, id, AUTO_ALIAS_CONFIDENCE_BPS, now],
        )?;
    }
    // The provenance handle: the first brand's identity (the list is non-empty, validated above).
    Ok(anchor_identity_id(&anchors[0].key))
}

/// Applies [`WriteCommand::ApplyMerchantMemory`](crate::WriteCommand).
///
/// Per assignment, in payload order: the transaction and the category must exist (else the whole
/// command is refused); a voided transaction, or one that already has a categorization of ANY
/// source, is skipped (no silent re-tag, ADR 0030); an archived category is NOT a reason to skip
/// (it mirrors `RecategorizeTransaction`); otherwise a `rule` categorization is written.
pub(crate) fn write_merchant_assignments(
    tx: &Connection,
    meta: &CommandMeta,
    assignments: &[MerchantAssignment],
) -> Result<Uuid, DbError> {
    if assignments.is_empty() || assignments.len() > MAX_MERCHANT_COMMAND_ENTRIES {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            format!("a memory run carries 1 to {MAX_MERCHANT_COMMAND_ENTRIES} assignments"),
        ));
    }
    if assignments
        .windows(2)
        .any(|pair| pair[0].transaction_id.as_uuid() >= pair[1].transaction_id.as_uuid())
    {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "the assignments are not in strictly ascending transaction order".to_owned(),
        ));
    }
    if assignments
        .iter()
        .any(|a| !(0..=10_000).contains(&a.confidence_bps))
    {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "an assignment confidence is outside 0 to 10000 basis points".to_owned(),
        ));
    }

    let now = crate::apply::issued_at_text(meta);
    for assignment in assignments {
        let txn = assignment.transaction_id.as_uuid();
        let voided: Option<bool> = tx
            .query_row(
                "SELECT voided_at IS NOT NULL FROM ledger_transactions WHERE id = ?1",
                [txn],
                |r| r.get(0),
            )
            .optional()?;
        let Some(voided) = voided else {
            return Err(DbError::reject(
                RejectCode::ReferenceMissing,
                "an assignment names a transaction that does not exist".to_owned(),
            ));
        };
        let category_known: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM categories WHERE id = ?1)",
            [assignment.category_id.as_uuid()],
            |r| r.get(0),
        )?;
        if !category_known {
            return Err(DbError::reject(
                RejectCode::ReferenceMissing,
                "an assignment names a category that does not exist".to_owned(),
            ));
        }
        if voided {
            continue;
        }
        tx.execute(
            "INSERT OR IGNORE INTO transaction_categorizations
                (transaction_id, category_id, source, confidence_bps, assigned_at)
             VALUES (?1, ?2, 'rule', ?3, ?4)",
            params![
                txn,
                assignment.category_id.as_uuid(),
                assignment.confidence_bps,
                now
            ],
        )?;
    }
    // The provenance handle: the first transaction of the run (the list is non-empty, validated).
    Ok(assignments[0].transaction_id.as_uuid())
}
