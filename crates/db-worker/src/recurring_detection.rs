//! Recurring-detection driver (personal-cfo-98ql). Reads realized outflows, normalizes the
//! merchant, and runs the pure [`categorization::detect_recurring`] to surface candidate
//! recurring bills. Excludes merchants already tracked as an active recurring bill, so the
//! suggestions cover only what is not yet modeled. Suggestions only — never auto-creates.

use std::collections::{HashMap, HashSet};

use categorization::{detect_recurring, normalize_merchant, Observation, RecurringCandidate};
use chrono::{Datelike, NaiveDate};
use rusqlite::Connection;
use uuid::Uuid;

use crate::forecast::parse_date;
use crate::DbError;

/// A detected recurring candidate plus the db-worker-derived provenance
/// (personal-cfo-4d8.24.5 dominant category; personal-cfo-4d8.25.11 detection proof,
/// ADR 0047 §4). The pure [`RecurringCandidate`] stays category- and account-free; the
/// driver aggregates what the promote form and the proof expander need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurringCandidateView {
    pub candidate: RecurringCandidate,
    /// The most-common non-transfer category across the merchant's observed charges, or
    /// `None` when none of them were categorized.
    pub dominant_category_id: Option<Uuid>,
    /// Display names of the accounts the observations posted to, most-frequent first
    /// (deterministic tie-break by name) — "always on Venture X" when there is one.
    pub source_account_names: Vec<String>,
    /// The single account ALL observations posted to, when there is exactly one — the
    /// ADR 0047 §4 autopay/pay-from prefill rule. `None` for a multi-account series.
    pub source_account_id: Option<Uuid>,
    /// The median day-of-month of the observations, for month-stepped cadences only
    /// ("roughly the 19th of every month"); `None` for week-based cadences.
    pub typical_day_of_month: Option<u32>,
    /// The observations behind the suggestion, most recent first (capped) — the proof
    /// the user can expand before promoting.
    pub observations: Vec<CandidateObservation>,
}

/// One observed charge behind a candidate (the proof expander's row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateObservation {
    pub date: NaiveDate,
    pub amount_minor: i64,
    pub account_name: String,
}

/// How many observations the proof expander carries per candidate.
const MAX_PROOF_OBSERVATIONS: usize = 12;

/// Which side of the ledger the detector looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Realized outflows on spend accounts → recurring-bill candidates.
    Outflow,
    /// Realized inflows on liquid accounts → income-source candidates
    /// (personal-cfo-gmnk, onboarding epic 5fp6).
    Inflow,
}

/// Every recurring candidate detectable from the household's realized outflows, as of `today`
/// (drives the next-date projection). Deterministic: the pure detector sorts its output, and
/// this only filters it.
///
/// # Errors
/// Returns [`DbError`] if a read fails or a stored date is malformed.
pub(crate) fn recurring_candidates(
    conn: &Connection,
    today: NaiveDate,
) -> Result<Vec<RecurringCandidateView>, DbError> {
    candidates_for(conn, today, Flow::Outflow)
}

/// Recurring INBOUND deposits on liquid accounts that are not yet modeled as an
/// income source — paychecks, benefits, a tenant's rent. Same detector, same
/// suppression store (a dismissal is a dismissal), different sign and exclusions.
pub(crate) fn income_candidates(
    conn: &Connection,
    today: NaiveDate,
) -> Result<Vec<RecurringCandidateView>, DbError> {
    candidates_for(conn, today, Flow::Inflow)
}

fn candidates_for(
    conn: &Connection,
    today: NaiveDate,
    flow: Flow,
) -> Result<Vec<RecurringCandidateView>, DbError> {
    // Outflow: realized outflows on spend accounts (a bill paid from checking posts to
    // liquid_cash; a subscription charged to a card posts to the credit facility).
    // Inflow: realized deposits on liquid accounts only (a card refund is not income).
    // System/virtual postings are excluded because the join to `accounts` keeps only user
    // accounts. Postings the user has categorized as a transfer are excluded — a recurring
    // account-to-account move (savings sweep, card autopay) is neither a bill nor income.
    // (An *uncategorized* imported transfer can still slip through; it is only a suggestion
    // the user reviews.)
    let (sign_clause, roles_clause) = match flow {
        Flow::Outflow => (
            "lp.minor_units < 0",
            "a.cashflow_role IN ('liquid_cash', 'credit_facility')",
        ),
        Flow::Inflow => ("lp.minor_units > 0", "a.cashflow_role = 'liquid_cash'"),
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT td.counterparty, td.memo, lt.occurred_at, lp.minor_units, lp.currency,
                tc.category_id, a.id, a.name
         FROM ledger_postings lp
         JOIN ledger_transactions lt ON lt.id = lp.transaction_id
         JOIN ledger_accounts la ON la.id = lp.ledger_account_id
         JOIN accounts a ON a.ledger_account_id = la.id
         LEFT JOIN transaction_details td ON td.transaction_id = lt.id
         LEFT JOIN transaction_categorizations tc ON tc.transaction_id = lt.id
         LEFT JOIN categories cat ON cat.id = tc.category_id
         WHERE lt.voided_at IS NULL
           AND {sign_clause}
           AND {roles_clause}
           AND (cat.type IS NULL OR cat.type <> 'transfer')"
    ))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<Uuid>>(5)?,
                r.get::<_, Uuid>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut observations = Vec::new();
    // merchant_key → (category → occurrence count); the dominant category is the mode
    // (personal-cfo-4d8.24.5). Uncategorized rows don't count.
    let mut category_counts: HashMap<String, HashMap<Uuid, u32>> = HashMap::new();
    // (merchant_key, currency) → ((account id, name) → occurrence count): the pay-from
    // provenance ("always on Venture X") and the ADR 0047 §4 single-account prefill
    // signal. Keyed like the detector groups candidates — merchant AND currency — so a
    // merchant charged in two currencies never mixes provenance across its candidates
    // (adversarial review of 4d8.25.11); the suppression map below follows the same rule.
    let mut account_counts: HashMap<(String, String), HashMap<(Uuid, String), u32>> =
        HashMap::new();
    // (merchant_key, currency) → the observed charges — the proof rows.
    let mut proof_rows: HashMap<(String, String), Vec<CandidateObservation>> = HashMap::new();
    for (counterparty, memo, occurred_at, minor, currency, category_id, account_id, account_name) in
        rows
    {
        // Prefer the payee; fall back to the memo. Without either there is nothing to group on.
        let Some(raw) = counterparty.or(memo).filter(|s| !s.trim().is_empty()) else {
            continue;
        };
        let key = normalize_merchant(&raw);
        if key.is_empty() || occurred_at.len() < 10 {
            continue;
        }
        if let Some(category) = category_id {
            *category_counts
                .entry(key.clone())
                .or_default()
                .entry(category)
                .or_default() += 1;
        }
        let date = parse_date(&occurred_at[..10])?;
        let provenance_key = (key.clone(), currency.clone());
        *account_counts
            .entry(provenance_key.clone())
            .or_default()
            .entry((account_id, account_name.clone()))
            .or_default() += 1;
        proof_rows
            .entry(provenance_key)
            .or_default()
            .push(CandidateObservation {
                date,
                amount_minor: minor.abs(),
                account_name,
            });
        observations.push(Observation {
            merchant_key: key,
            display: raw.trim().to_owned(),
            date,
            amount_minor: minor.abs(),
            currency,
        });
    }

    // Reduce each merchant's tally to its dominant category — highest count, tie-broken by
    // the smallest id so the result is deterministic (the module's contract).
    let dominant_category: HashMap<String, Uuid> = category_counts
        .into_iter()
        .filter_map(|(merchant, counts)| {
            counts
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                .map(|(category, _)| (merchant, category))
        })
        .collect();

    let mut candidates = detect_recurring(&observations, today);

    // Drop merchants already tracked as an active recurring bill — those are modeled;
    // suggest only what is not. Three exclusion sources, all keyed on the normalized
    // merchant key (ADR 0047 §2):
    //   1. The bill NAME (normalized) — covers manually-created bills.
    //   2. The bill's persisted `source_merchant_key` (personal-cfo-5n4.8) — the durable
    //      link recorded when a bill is promoted from a candidate. This survives a rename,
    //      so renaming a promoted bill no longer re-surfaces its suggestion.
    //   3. EVIDENCE keys — the normalized payees of transactions actually linked to an
    //      active bill's instances (personal-cfo-4d8.25.9): whatever spelling the imports
    //      used, once postings link, their key consumes future candidates for that
    //      spelling. Grows with the data. A key must recur across ≥2 of one bill's linked
    //      occurrences before it counts: a single link can be an amount-only coincidence
    //      (the matcher links payee-less postings by amount within the window), and one
    //      coincidence must not silently consume an unrelated merchant's candidate.
    let mut tracked: HashSet<String> = match flow {
        Flow::Outflow => crate::read_recurring_bill_views(conn)?
            .iter()
            .filter(|b| b.active)
            .map(|b| normalize_merchant(&b.name))
            .collect(),
        // Income sources carry no persisted merchant key (yet) — the name is the link.
        Flow::Inflow => crate::read_income_source_views(conn)?
            .iter()
            .filter(|i| i.active)
            .map(|i| normalize_merchant(&i.name))
            .collect(),
    };
    if flow == Flow::Outflow {
        let mut stmt = conn.prepare(
            "SELECT source_merchant_key FROM recurring_events
              WHERE is_active = 1 AND source_merchant_key IS NOT NULL",
        )?;
        // The stored key is already a `normalize_merchant` output (the candidate's key),
        // so it compares directly against `c.merchant_key` — no re-normalization.
        let keys = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for key in keys {
            tracked.insert(key?);
        }
    }
    // Bills only (gmnk review): the evidence seam joins recurring_events, so
    // its keys are BILL identities and must never consume an income candidate.
    if flow == Flow::Outflow {
        // A link is only trusted as MERCHANT-IDENTITY evidence when it could not be a
        // cross-merchant amount coincidence (adversarial review of 4d8.25.9): the matcher
        // deliberately links payee-mismatched postings when the amount is within
        // max(5%, $5.00), and a $5-floor match between two small unrelated subscriptions at
        // a stable monthly offset recurs systematically — defeating any count floor. Trust
        // requires the posting amount within the RELATIVE band only (no absolute floor;
        // the floor exists for same-merchant payment drift, not cross-merchant tolerance)
        // OR a key that already conflicts with the bill's own name key (spelling variants).
        let mut evidence_stmt = conn.prepare(
            "SELECT i.recurring_event_id, e.name, i.expected_amount_minor, lp.minor_units,
                td.counterparty, td.memo
           FROM recurring_event_instances i
           JOIN recurring_events e ON e.id = i.recurring_event_id AND e.is_active = 1
           JOIN transaction_details td ON td.transaction_id = i.linked_transaction_id
           JOIN ledger_postings lp ON lp.transaction_id = i.linked_transaction_id
           JOIN ledger_accounts la ON la.id = lp.ledger_account_id
           JOIN accounts a ON a.ledger_account_id = la.id
          WHERE i.linked_transaction_id IS NOT NULL
            AND a.cashflow_role IN ('liquid_cash', 'credit_facility')",
        )?;
        let evidence = evidence_stmt.query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut evidence_counts: HashMap<(Uuid, String), u32> = HashMap::new();
        for row in evidence {
            let (event_id, bill_name, expected_minor, posting_minor, counterparty, memo) = row?;
            let Some(raw) = counterparty.or(memo).filter(|s| !s.trim().is_empty()) else {
                continue;
            };
            let key = normalize_merchant(&raw);
            if key.is_empty() {
                continue;
            }
            let relative_band =
                expected_minor.abs() * crate::recurring_instances::AMOUNT_TOLERANCE_BPS / 10_000;
            let amount_trusted =
                (posting_minor.abs() - expected_minor.abs()).abs() <= relative_band;
            let spelling_variant =
                categorization::keys_conflict(&key, &normalize_merchant(&bill_name));
            if amount_trusted || spelling_variant {
                *evidence_counts.entry((event_id, key)).or_default() += 1;
            }
        }
        for ((_event, key), count) in evidence_counts {
            if count >= 2 {
                tracked.insert(key);
            }
        }
    }

    // User dismissals (ADR 0046, personal-cfo-4d8.24.6): a suppressed (merchant_key, currency)
    // is dropped UNLESS the pattern materially changed since the dismissal — the inferred
    // cadence differs, or the amount moved outside the detector's amount band around the
    // dismissed amount — in which case the suggestion re-surfaces (the user sees the change).
    let mut supp_stmt = conn.prepare(
        "SELECT merchant_key, currency, amount_minor, frequency
           FROM recurring_suggestion_suppressions",
    )?;
    let suppressions: HashMap<(String, String), (i64, String)> = supp_stmt
        .query_map([], |r| {
            Ok((
                (r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                (r.get::<_, i64>(2)?, r.get::<_, String>(3)?),
            ))
        })?
        .collect::<Result<_, _>>()?;

    candidates.retain(|c| {
        // Exact key first (cheap), then the ADR 0047 §2.3 variant test — anchored
        // same-merchant grouping + the conservative truncation rule — against every
        // tracked key. `tracked` is small (the household's bill count plus linked
        // spellings), so the pairwise scan is bounded.
        if tracked.contains(&c.merchant_key)
            || tracked
                .iter()
                .any(|t| categorization::keys_conflict(&c.merchant_key, t))
        {
            return false;
        }
        if let Some((dismissed_amount, dismissed_frequency)) =
            suppressions.get(&(c.merchant_key.clone(), c.currency.clone()))
        {
            let same_cadence = c.frequency == dismissed_frequency.as_str();
            let within_band = (c.amount_minor - dismissed_amount).abs()
                <= categorization::amount_band_minor(*dismissed_amount);
            if same_cadence && within_band {
                return false; // still the dismissed pattern — keep it suppressed
            }
        }
        true
    });

    // Attach the db-worker-computed provenance to each surviving candidate. The pure
    // detector never sees categories or accounts (DB concerns), so this join lives here
    // (personal-cfo-4d8.24.5 category; personal-cfo-4d8.25.11 proof + prefill).
    Ok(candidates
        .into_iter()
        .map(|candidate| {
            let dominant_category_id = dominant_category.get(&candidate.merchant_key).copied();
            let provenance_key = (candidate.merchant_key.clone(), candidate.currency.clone());
            // Accounts: most-frequent first, name tie-break (deterministic contract).
            let mut accounts: Vec<((Uuid, String), u32)> = account_counts
                .get(&provenance_key)
                .map(|counts| counts.iter().map(|(k, &n)| (k.clone(), n)).collect())
                .unwrap_or_default();
            accounts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0 .1.cmp(&b.0 .1)));
            // The §4 prefill applies only when the series is single-account.
            let source_account_id = match accounts.as_slice() {
                [only] => Some(only.0 .0),
                _ => None,
            };
            let source_account_names: Vec<String> =
                accounts.into_iter().map(|((_, name), _)| name).collect();
            let mut observations = proof_rows.remove(&provenance_key).unwrap_or_default();
            observations.sort_by(|a, b| {
                b.date
                    .cmp(&a.date)
                    .then_with(|| b.amount_minor.cmp(&a.amount_minor))
                    .then_with(|| a.account_name.cmp(&b.account_name))
            });
            observations.truncate(MAX_PROOF_OBSERVATIONS);
            // "Roughly the 19th": the median observed day-of-month, meaningful only for
            // month-stepped cadences (a weekly bill has no typical day-of-month).
            let typical_day_of_month =
                if matches!(candidate.frequency, "monthly" | "quarterly" | "annual") {
                    let mut days: Vec<u32> = observations.iter().map(|o| o.date.day()).collect();
                    days.sort_unstable();
                    (!days.is_empty()).then(|| days[days.len() / 2])
                } else {
                    None
                };
            RecurringCandidateView {
                candidate,
                dominant_category_id,
                source_account_names,
                source_account_id,
                typical_day_of_month,
                observations,
            }
        })
        .collect())
}
