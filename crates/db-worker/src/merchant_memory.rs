//! Merchant-memory auto-categorization (personal-cfo-7yh0 follow-on, ADR 0030 addendum
//! 2026-06-29). Learn merchant→category from the user's **manual** categorizations, then
//! apply the learned category to currently-**uncategorized** transactions of the same
//! merchant. Never overwrites an existing assignment ("no silent re-tag", ADR 0030);
//! `source = 'rule'`, confidence = the agreement ratio.
//!
//! **Identity-aware grouping (7yh0-v2, ADR 0030 addendum 2026-06-30).** The grouping key is
//! the resolved merchant **identity** (`merchant_aliases` → `merchant_identities`) when one
//! exists, so a category learned under one alias (`AMAZON COM`) fills same-identity
//! transactions under a different alias (`AMZN MKTP US`). With no identities the key falls
//! back to the normalized string — behaviour identical to v1. **Ambiguous** identities
//! (Amazon, Costco) are excluded from auto-fill: they span categories, so a single learned
//! category must not propagate — they defer to disambiguation (ADR 0030 / zrpg).

use std::collections::HashMap;

use categorization::normalize_merchant;
use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::DbError;

/// Minimum agreement (winning category count / total) to auto-apply a merchant's learned
/// category, in basis points (60%). Conflicted merchants below this are left for review.
const MIN_AGREEMENT_BPS: i64 = 6000;

/// How a normalized merchant key resolves to its canonical identity (7yh0-v2).
struct Resolved {
    identity: Uuid,
    ambiguous: bool,
}

/// The alias index loaded once per apply: exact-key lookups plus prefix aliases for
/// multi-location chains (2a6r), longest-first so a more specific brand token wins.
struct Resolution {
    exact: HashMap<String, Resolved>,
    prefixes: Vec<(String, Resolved)>,
}

impl Resolution {
    /// Resolve a normalized key: exact alias, then a prefix alias the key begins with on a
    /// word boundary (`COSTCO` matches `COSTCO WHSE SEATTLE`, never `COSTCOX`).
    fn get(&self, norm: &str) -> Option<&Resolved> {
        if let Some(r) = self.exact.get(norm) {
            return Some(r);
        }
        self.prefixes
            .iter()
            .find(|(p, _)| norm.starts_with(&format!("{p} ")))
            .map(|(_, r)| r)
    }
}

/// The grouping key for a counterparty: its resolved identity when an alias maps it,
/// else the normalized string. `None` means "skip" — an empty key, or an **ambiguous**
/// identity that must not receive a single auto-category.
fn group_key(resolution: &Resolution, counterparty: &str) -> Option<String> {
    let norm = normalize_merchant(counterparty);
    if norm.is_empty() {
        return None;
    }
    match resolution.get(&norm) {
        Some(r) if r.ambiguous => None,
        Some(r) => Some(format!("id:{}", r.identity)),
        None => Some(format!("k:{norm}")),
    }
}

/// Apply merchant memory; returns the number of transactions newly categorized.
pub(crate) fn apply(conn: &Connection) -> Result<u32, DbError> {
    // Mint earned anchors for unseeded multi-location chains first (5n4.4), so the resolution
    // load below sees them and the memory groups those chains' locations together.
    crate::merchant_grouping::mint_anchors(conn)?;

    // 0. Load the normalized-key → identity resolution (7yh0-v2 + 2a6r prefixes). Small (one
    //    row per alias); empty when no identities exist, so grouping is by normalized string.
    let resolution = {
        let mut exact: HashMap<String, Resolved> = HashMap::new();
        let mut prefixes: Vec<(String, Resolved)> = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT ma.normalized_key, ma.merchant_identity_id, mi.is_ambiguous, ma.match_type
             FROM merchant_aliases ma
             JOIN merchant_identities mi ON mi.id = ma.merchant_identity_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Uuid>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (key, identity, ambiguous, match_type) = row?;
            let resolved = Resolved {
                identity,
                ambiguous: ambiguous != 0,
            };
            if match_type == "prefix" {
                prefixes.push((key, resolved));
            } else {
                exact.insert(key, resolved);
            }
        }
        // Longest prefix first so the most specific brand token wins.
        prefixes.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));
        Resolution { exact, prefixes }
    };

    // 1. Build the memory: grouping key → (category → count) from USER assignments.
    let mut memory: HashMap<String, HashMap<Uuid, u32>> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT td.counterparty, tc.category_id
             FROM transaction_categorizations tc
             JOIN transaction_details td ON td.transaction_id = tc.transaction_id
             WHERE tc.source = 'user'
               AND td.counterparty IS NOT NULL AND TRIM(td.counterparty) <> ''",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Uuid>(1)?)))?;
        for row in rows {
            let (counterparty, category_id) = row?;
            if let Some(key) = group_key(&resolution, &counterparty) {
                *memory
                    .entry(key)
                    .or_default()
                    .entry(category_id)
                    .or_default() += 1;
            }
        }
    }

    // 2. Resolve each merchant to a winning category that clears the agreement threshold.
    let winners: HashMap<String, (Uuid, i64)> = memory
        .into_iter()
        .filter_map(|(key, counts)| {
            let total: u32 = counts.values().copied().sum();
            let (category, win) = counts.into_iter().max_by_key(|&(_, c)| c)?;
            let agreement_bps = i64::from(win) * 10_000 / i64::from(total);
            (agreement_bps >= MIN_AGREEMENT_BPS).then_some((key, (category, agreement_bps)))
        })
        .collect();
    if winners.is_empty() {
        return Ok(0);
    }

    // 3. Collect uncategorized transactions that carry a counterparty.
    let uncategorized: Vec<(Uuid, String)> = {
        let mut stmt = conn.prepare(
            "SELECT lt.id, td.counterparty
             FROM ledger_transactions lt
             JOIN transaction_details td ON td.transaction_id = lt.id
             LEFT JOIN transaction_categorizations tc ON tc.transaction_id = lt.id
             WHERE lt.voided_at IS NULL
               AND tc.transaction_id IS NULL
               AND td.counterparty IS NOT NULL AND TRIM(td.counterparty) <> ''",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Uuid>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    // 4. Apply the learned category where the grouping key (identity or normalized merchant)
    //    matches. An ambiguous-identity counterparty resolves to `None` and is skipped.
    let now = Utc::now().to_rfc3339();
    let mut applied = 0u32;
    for (txn_id, counterparty) in uncategorized {
        let Some(key) = group_key(&resolution, &counterparty) else {
            continue;
        };
        if let Some((category, agreement_bps)) = winners.get(&key) {
            conn.execute(
                "INSERT INTO transaction_categorizations
                    (transaction_id, category_id, source, confidence_bps, assigned_at)
                 VALUES (?1, ?2, 'rule', ?3, ?4)",
                params![txn_id, category, agreement_bps, now],
            )?;
            applied += 1;
        }
    }
    Ok(applied)
}
