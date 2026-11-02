//! Auto-fuzzy merchant grouping — minting pass (personal-cfo-5n4.4, ADR 0030 addendum
//! 2026-06-30). The pure heuristic lives in [`categorization::merchant_grouping`]; this is the
//! db-worker driver that applies it to observed merchant keys.
//!
//! For each distinct, **still-unresolved** merchant counterparty, extract a brand anchor (a
//! location-peeled, distinctive spine). When ≥2 distinct keys that carry a location tail share
//! one anchor (a chain seen at two locations), mint an `is_ambiguous = false` identity for the
//! brand and link the brand as a **`prefix` alias** (`source = 'auto'`) — so every current and
//! future location of that chain resolves through the existing prefix machinery. Anchored
//! containment by construction: an anchor is only ever a curated-or-earned brand, never a
//! fusion of two arbitrary keys, so the false-merge blast radius is one identity.
//!
//! This module is the READ-ONLY planner (personal-cfo-5ymg8.5): it returns the brands to mint, and
//! the `MintMerchantAnchors` command writes them (`apply/merchant_cmds.rs`, which derives the
//! identity id from the key and skips a key an alias already holds). Planning is idempotent: a
//! brand once minted resolves its locations, so a re-plan returns nothing new.

use std::collections::{HashMap, HashSet};

use categorization::{brand_anchor, normalize_merchant};
use rusqlite::Connection;

use crate::merchant_identity::{alias_key_fits, AliasIndex};
use crate::{DbError, MerchantAnchor};

/// Minimum distinct location-bearing keys sharing a brand before it is minted as an anchor —
/// a chain must be seen at ≥2 locations, so a one-off sighting never invents a merchant.
const MIN_DISTINCT_LOCATIONS: usize = 2;

/// Plan the anchors to mint for unseeded multi-location chains, in strictly ascending key order.
///
/// # Errors
/// Returns [`DbError`] if a SQLite read fails.
pub(crate) fn plan_anchors(conn: &Connection) -> Result<Vec<MerchantAnchor>, DbError> {
    // Distinct merchant counterparties across non-voided transactions.
    let mut stmt = conn.prepare(
        "SELECT DISTINCT td.counterparty
         FROM transaction_details td
         JOIN ledger_transactions lt ON lt.id = td.transaction_id
         WHERE lt.voided_at IS NULL
           AND td.counterparty IS NOT NULL AND TRIM(td.counterparty) <> ''",
    )?;
    let raw_keys: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let index = AliasIndex::load(conn)?;

    // Group still-unresolved, location-bearing keys by their brand anchor.
    let mut groups: HashMap<String, HashSet<String>> = HashMap::new();
    for raw in &raw_keys {
        let key = normalize_merchant(raw);
        if key.is_empty() {
            continue;
        }
        // Skip keys an existing seed/user/auto alias already resolves (no duplicate anchors).
        if index.lookup(&key).is_some() {
            continue;
        }
        if let Some(anchor) = brand_anchor(&key) {
            if key != anchor {
                // The key carried a location tail → a real location sighting.
                groups.entry(anchor).or_default().insert(key);
            }
        }
    }

    let mut anchors: Vec<MerchantAnchor> = groups
        .into_iter()
        .filter(|(anchor, keys)| {
            // A chain must be seen at several locations, so a one-off sighting never invents a
            // merchant. An anchor derived from transaction text that would exceed the replicated
            // key width is not minted: ingestion must not fail on it, and the key is never
            // truncated, which would change what it identifies. An anchor key some alias already
            // holds is not minted either (the apply skips it too).
            keys.len() >= MIN_DISTINCT_LOCATIONS
                && alias_key_fits(anchor)
                && !index.holds_key(anchor)
        })
        .map(|(anchor, _)| MerchantAnchor {
            display_name: title_case(&anchor),
            key: anchor,
        })
        .collect();
    // Deterministic order: the payload is a function of the vault, never of HashMap iteration.
    anchors.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(anchors)
}

/// Title-case a normalized (uppercase) anchor for display: `RITUAL COFFEE` → `Ritual Coffee`.
fn title_case(anchor: &str) -> String {
    anchor
        .split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
