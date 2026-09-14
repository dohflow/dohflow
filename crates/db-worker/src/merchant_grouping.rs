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
//! Idempotent: deterministic v5 ids + `INSERT OR IGNORE`, and already-resolved keys are
//! skipped, so a re-run mints nothing new and a user's own mapping is never clobbered.

use std::collections::{HashMap, HashSet};

use categorization::{brand_anchor, normalize_merchant};
use chrono::Utc;
use rusqlite::Connection;
use uuid::Uuid;

use crate::{merchant_identity, DbError};

/// Minimum distinct location-bearing keys sharing a brand before it is minted as an anchor —
/// a chain must be seen at ≥2 locations, so a one-off sighting never invents a merchant.
const MIN_DISTINCT_LOCATIONS: usize = 2;

/// Confidence (bps) recorded on an auto-minted alias — below the 10000 of a seed/user mapping
/// so a weak grouping stays auditable and reversible (ADR 0030).
const AUTO_ALIAS_CONFIDENCE_BPS: i64 = 8000;

/// Mint anchors for unseeded multi-location chains. Returns the number of new identities minted.
///
/// # Errors
/// Returns [`DbError`] if a SQLite operation fails.
pub(crate) fn mint_anchors(conn: &Connection) -> Result<u32, DbError> {
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

    // Group still-unresolved, location-bearing keys by their brand anchor.
    let mut groups: HashMap<String, HashSet<String>> = HashMap::new();
    for raw in &raw_keys {
        let key = normalize_merchant(raw);
        if key.is_empty() {
            continue;
        }
        // Skip keys an existing seed/user/auto alias already resolves (no duplicate anchors).
        if merchant_identity::resolve(conn, &key)?.is_some() {
            continue;
        }
        if let Some(anchor) = brand_anchor(&key) {
            if key != anchor {
                // The key carried a location tail → a real location sighting.
                groups.entry(anchor).or_default().insert(key);
            }
        }
    }

    let now = Utc::now().to_rfc3339();
    let mut minted = 0u32;
    // Deterministic order so a run is reproducible regardless of HashMap iteration.
    let mut anchors: Vec<(&String, &HashSet<String>)> = groups.iter().collect();
    anchors.sort_by(|a, b| a.0.cmp(b.0));
    for (anchor, keys) in anchors {
        if keys.len() < MIN_DISTINCT_LOCATIONS {
            continue;
        }
        let id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("merchant-auto:{anchor}").as_bytes(),
        );
        let changed = conn.execute(
            "INSERT OR IGNORE INTO merchant_identities
                (id, display_name, default_category_id, is_ambiguous, source, created_at)
             VALUES (?1, ?2, NULL, 0, 'auto', ?3)",
            rusqlite::params![id, title_case(anchor), now],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO merchant_aliases
                (normalized_key, merchant_identity_id, source, confidence_bps, match_type, created_at)
             VALUES (?1, ?2, 'auto', ?3, 'prefix', ?4)",
            rusqlite::params![anchor, id, AUTO_ALIAS_CONFIDENCE_BPS, now],
        )?;
        minted += u32::try_from(changed).unwrap_or(0);
    }
    Ok(minted)
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
