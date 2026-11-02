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
//!
//! This module is the READ-ONLY planner (personal-cfo-5ymg8.5): it returns the fills, and the
//! `ApplyMerchantMemory` command writes them (`apply/merchant_cmds.rs`). The thresholds below are
//! tunable, which is why the command carries the OUTCOME rather than re-deriving it in apply.

use std::collections::HashMap;

use categorization::normalize_merchant;
use rusqlite::Connection;
use uuid::Uuid;

use crate::merchant_identity::AliasIndex;
use crate::{CategoryId, DbError, MerchantAssignment, TransactionId};

/// Minimum agreement (winning category count / total) to auto-apply a merchant's learned
/// category, in basis points (60%). Conflicted merchants below this are left for review.
const MIN_AGREEMENT_BPS: i64 = 6000;

/// The grouping key for a counterparty: its resolved identity when an alias maps it,
/// else the normalized string. `None` means "skip" — an empty key, or an **ambiguous**
/// identity that must not receive a single auto-category.
fn group_key(resolution: &AliasIndex, counterparty: &str) -> Option<String> {
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

/// Plan the fills merchant memory would make now, ascending by transaction id.
///
/// Learns merchant→category from the user's manual categorizations and picks, for each merchant
/// whose winning category clears [`MIN_AGREEMENT_BPS`], every currently uncategorized transaction of
/// that merchant. Anchors are minted first by the caller (the plan must see them).
pub(crate) fn plan_assignments(conn: &Connection) -> Result<Vec<MerchantAssignment>, DbError> {
    // 0. Load the normalized-key → identity resolution (7yh0-v2 + 2a6r prefixes). Small (one
    //    row per alias); empty when no identities exist, so grouping is by normalized string.
    let resolution = AliasIndex::load(conn)?;

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
        return Ok(Vec::new());
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

    // 4. Fill where the grouping key (identity or normalized merchant) matches. An ambiguous
    //    identity's counterparty resolves to `None` and is skipped.
    let mut plan: Vec<MerchantAssignment> = Vec::new();
    for (txn_id, counterparty) in uncategorized {
        let Some(key) = group_key(&resolution, &counterparty) else {
            continue;
        };
        if let Some((category, agreement_bps)) = winners.get(&key) {
            plan.push(MerchantAssignment {
                transaction_id: TransactionId::from_uuid(txn_id),
                category_id: CategoryId::from_uuid(*category),
                confidence_bps: *agreement_bps,
            });
        }
    }
    plan.sort_by_key(|a| a.transaction_id.as_uuid());
    Ok(plan)
}
