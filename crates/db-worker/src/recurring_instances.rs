//! Recurring-instance projection + transaction linking (ADR 0026 §9; beads
//! personal-cfo-5ie.4 / -5ie.5).
//!
//! `recurring_event_instances` is a **derived read model**: one row per scheduled
//! occurrence of an active recurring schedule (income source or recurring
//! obligation), expanded over a window through the same [`PaySchedule`] the forecast
//! uses (via [`crate::schedule_sources`]) so an instance and its forecast row share
//! `(entity, date, amount)`. After projecting, each scheduled instance is linked to
//! the realized liquid-account ledger posting that satisfies it
//! (`status='paid'` + `linked_transaction_id`). That durable, inspectable
//! per-occurrence link is the seam the actualization loop (`46jq`) scores against.
//!
//! Full rebuild, idempotent, deterministic given the rebuild date: the surrogate id
//! is a v5 hash of `(recurring_event_id, scheduled_date)`, so a re-run writes
//! byte-identical rows. v1 expands the **base** schedule only (no scenario overlay /
//! per-window overrides) — actualization scores the base deterministic run.
//!
//! Note: `recurring_event_id` stores the forecast *source entity* id, which is an
//! `income_sources.id` for inflows or a `recurring_events.id` for obligations —
//! unified exactly as the forecast unifies them under `source_event_id`.

use categorization::normalize_merchant;
use chrono::{Duration, NaiveDate};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::forecast::{parse_date, parse_frequency};
use crate::schedule_sources::{
    active_income_schedules, active_obligation_schedules, ScheduleKind, ScheduleSource,
};
use crate::{currency_from_code, DbError};
use pay_schedule::PaySchedule;

/// Lookback when the vault has no realized postings yet: project a year of history
/// so already-due occurrences exist to link against as transactions arrive.
const DEFAULT_LOOKBACK_DAYS: i64 = 365;
/// Forward window — matches the daily-persist horizon (A1) so every persisted
/// forecast row has a corresponding instance.
const FORWARD_HORIZON_DAYS: i64 = 365;
/// Date tolerance (days) for matching a realized posting to a scheduled recurring
/// occurrence (ADR 0026 §9 + the 4d8.24.4 addendum): ±7 days covers weekend/holiday/
/// bank-closure settlement drift without spanning a monthly cadence. Shared with the
/// forecast's confirmed-obligation suppression so the two windows can never diverge.
pub(crate) const RECURRING_MATCH_WINDOW_DAYS: i64 = 7;
/// Relative amount tolerance for linking (basis points): a posting matches when its
/// magnitude is within `max(5%, floor)` of the expected amount.
pub(crate) const AMOUNT_TOLERANCE_BPS: i64 = 500;
/// Absolute floor for the amount tolerance, so tiny expected amounts still allow a
/// few cents of drift.
const AMOUNT_TOLERANCE_FLOOR_MINOR: i64 = 500;

/// Namespace for the deterministic v5 instance id (stable across rebuilds).
const INSTANCE_ID_NAMESPACE: Uuid = Uuid::from_bytes([
    0x5e, 0x1e, 0x04, 0x5e, 0x1e, 0x05, 0x4a, 0x2c, 0x9b, 0x3d, 0x6f, 0x10, 0x2a, 0x6b, 0x0c, 0x95,
]);

/// A projected recurring instance (read view).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurringInstanceRow {
    pub id: Uuid,
    /// The forecast source entity (income source or recurring event) id.
    pub recurring_event_id: Uuid,
    pub scheduled_date: String,
    pub expected_amount_minor: i64,
    pub currency: String,
    pub status: String,
    pub linked_transaction_id: Option<Uuid>,
}

/// One expanded occurrence, before it is written / linked.
struct Occurrence {
    recurring_event_id: Uuid,
    scheduled_date: NaiveDate,
    /// Positive magnitude (income net / obligation expected); sign is `kind`.
    expected_amount_minor: i64,
    currency: String,
    kind: ScheduleKind,
    /// The schedule's normalized merchant key (from its name), the payee match signal
    /// (ADR 0026 4d8.24.4 addendum). Empty when the name normalizes to nothing.
    merchant_key: Option<String>,
    /// The obligation's pay-from account — the linking account gate (ADR 0047 §1): when
    /// set, only postings from THIS account are eligible (a card-charged bill links its
    /// card postings, and cannot steal a same-window liquid posting). `None` for income
    /// and for bills without a pay-from (liquid postings only, the pre-0047 behavior).
    autopay_account_id: Option<Uuid>,
}

/// One realized posting — a linking candidate (liquid accounts, plus credit-facility
/// accounts so card-charged bills can attach their card history, ADR 0047 §1).
struct RealizedPosting {
    transaction_id: Uuid,
    occurred_on: NaiveDate,
    /// Signed from the account's perspective (inflow positive, outflow negative).
    minor_units: i64,
    /// The posting's normalized merchant key (counterparty, else memo), the payee match
    /// signal. `None` when the posting has no counterparty/memo (links on amount only).
    merchant_key: Option<String>,
    /// The user account the posting belongs to (the gate's join key).
    account_id: Uuid,
    /// Whether that account is `liquid_cash` — un-gated occurrences only ever link
    /// liquid postings (income deposits and no-pay-from bills, the pre-0047 behavior).
    is_liquid: bool,
}

/// Full rebuild in its own transaction. `today` anchors the window + the instances'
/// `created_at` (kept out of `Utc::now()` so a rebuild stays deterministic and
/// testable). Returns the number of instances written.
///
/// # Errors
/// Returns [`DbError`] if a SQLite operation fails or a schedule is malformed.
pub(crate) fn rebuild(conn: &mut Connection, today: NaiveDate) -> Result<u64, DbError> {
    let tx = conn.transaction()?;
    let count = rebuild_in(&tx, today)?;
    // The one-off manual-entry matcher rides the same rebuild (ADR 0026
    // addendum, xtz5): both projections derive from the ledger, and every
    // seam-freshening touch keeps them in step. Runs after instance linking
    // so an instance-claimed transaction is never also claimed by an entry.
    crate::manual_entry::rebuild_links(&tx, today)?;
    tx.commit()?;
    Ok(count)
}

/// Re-derive the projection within an existing connection/transaction (no commit).
///
/// # Errors
/// Returns [`DbError`] if a SQLite operation fails or a schedule is malformed.
pub(crate) fn rebuild_in(conn: &Connection, today: NaiveDate) -> Result<u64, DbError> {
    conn.execute("DELETE FROM recurring_event_instances", [])?;

    let window_start = window_start(conn, today)?;
    let window_end = today + Duration::days(FORWARD_HORIZON_DAYS);
    let occurrences = expand_occurrences(conn, window_start, window_end)?;
    let postings = realized_postings(conn, window_start, window_end)?;
    let links = assign_links(&occurrences, &postings);

    let created_at = today.to_string();
    let mut count = 0u64;
    for (idx, occ) in occurrences.iter().enumerate() {
        let linked = links.get(&idx).copied();
        let status = if linked.is_some() {
            "paid"
        } else {
            "scheduled"
        };
        let scheduled = occ.scheduled_date.to_string();
        conn.execute(
            "INSERT INTO recurring_event_instances
                (id, recurring_event_id, scheduled_date, expected_amount_minor,
                 currency, status, linked_transaction_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                instance_id(occ.recurring_event_id, &scheduled),
                occ.recurring_event_id,
                scheduled,
                occ.expected_amount_minor,
                occ.currency,
                status,
                linked,
                created_at,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

/// Deterministic instance id from `(entity, scheduled_date)`, so a rebuild is
/// byte-identical and a later incremental path can address a row stably.
fn instance_id(recurring_event_id: Uuid, scheduled_date: &str) -> Uuid {
    Uuid::new_v5(
        &INSTANCE_ID_NAMESPACE,
        format!("{recurring_event_id}:{scheduled_date}").as_bytes(),
    )
}

/// The window start: the earliest realized posting date (so every already-due
/// occurrence is covered and linkable), falling back to a year before `today` when
/// the vault has no postings yet.
fn window_start(conn: &Connection, today: NaiveDate) -> Result<NaiveDate, DbError> {
    let earliest: Option<String> = conn.query_row(
        "SELECT MIN(occurred_at) FROM ledger_transactions WHERE voided_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    let fallback = today - Duration::days(DEFAULT_LOOKBACK_DAYS);
    match earliest {
        // `occurred_at` is RFC 3339; its first 10 chars are the calendar day.
        Some(ts) if ts.len() >= 10 => Ok(parse_date(&ts[..10]).unwrap_or(fallback).min(fallback)),
        _ => Ok(fallback),
    }
}

/// Expand every active income + obligation schedule into dated occurrences over
/// `[start, end]`, reusing the shared schedule readers + `PaySchedule` so the set
/// matches the forecast's events for the same schedules.
fn expand_occurrences(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<Occurrence>, DbError> {
    let mut out = Vec::new();
    let sources = active_income_schedules(conn)?
        .into_iter()
        .chain(active_obligation_schedules(conn)?);
    for source in sources {
        let ScheduleSource {
            id,
            name,
            amount_minor,
            currency_code,
            freq_token,
            anchor,
            kind,
            autopay_account_id,
            ..
        } = source;
        let currency = currency_from_code(&currency_code)?;
        let frequency = parse_frequency(&freq_token, "schedule")?;
        let Some(anchor_str) = anchor else { continue };
        let anchor = parse_date(&anchor_str)?;
        let merchant_key = normalize_key(&name);
        for date in PaySchedule::new(frequency, anchor).pay_dates(start, end) {
            out.push(Occurrence {
                recurring_event_id: id,
                scheduled_date: date,
                expected_amount_minor: amount_minor,
                currency: currency.code().to_owned(),
                kind,
                merchant_key: merchant_key.clone(),
                autopay_account_id,
            });
        }
    }
    Ok(out)
}

/// Normalize a raw payee/name into the shared merchant key used for the payee match
/// signal (the same `normalize_merchant` recurring detection uses). `None` when the
/// input is empty or normalizes to nothing, so it never spuriously matches.
fn normalize_key(raw: &str) -> Option<String> {
    let key = normalize_merchant(raw);
    (!key.is_empty()).then_some(key)
}

/// Realized, non-voided postings on liquid-cash AND credit-facility accounts within
/// `[start, end]`, each carrying its normalized payee (counterparty, else memo) for the
/// payee match signal and its account for the ADR 0047 §1 gate. Card postings are only
/// ever linked by occurrences explicitly gated to that card ([`assign_links`]).
fn realized_postings(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<RealizedPosting>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT lt.id, lt.occurred_at, lp.minor_units, td.counterparty, td.memo,
                a.id, a.cashflow_role
         FROM ledger_postings lp
         JOIN ledger_transactions lt ON lt.id = lp.transaction_id
         JOIN ledger_accounts la ON la.id = lp.ledger_account_id
         JOIN accounts a ON a.ledger_account_id = la.id
         LEFT JOIN transaction_details td ON td.transaction_id = lt.id
         WHERE a.cashflow_role IN ('liquid_cash', 'credit_facility')
           AND lt.voided_at IS NULL
           AND substr(lt.occurred_at, 1, 10) BETWEEN ?1 AND ?2
         ORDER BY lt.occurred_at, lt.id",
    )?;
    let rows = stmt
        .query_map(params![start.to_string(), end.to_string()], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Uuid>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    for (transaction_id, occurred_at, minor_units, counterparty, memo, account_id, role) in rows {
        if occurred_at.len() < 10 {
            continue;
        }
        // Prefer the counterparty (the payee); fall back to the memo.
        let raw = counterparty
            .filter(|s| !s.trim().is_empty())
            .or(memo.filter(|s| !s.trim().is_empty()));
        out.push(RealizedPosting {
            transaction_id,
            occurred_on: parse_date(&occurred_at[..10])?,
            minor_units,
            merchant_key: raw.as_deref().and_then(normalize_key),
            account_id,
            is_liquid: role == "liquid_cash",
        });
    }
    Ok(out)
}

/// Greedily assign realized postings to occurrences. A posting matches an occurrence
/// of the same sign whose date is within [`DATE_TOLERANCE_DAYS`] and whose magnitude
/// is within tolerance of the expected amount. Each posting links ≤1 occurrence and
/// each occurrence ≤1 posting; contention resolves by nearest date, then nearest
/// amount, then `(transaction_id, occurrence index)` — deterministic. Returns a map
/// `occurrence index → linked transaction id`.
fn assign_links(
    occurrences: &[Occurrence],
    postings: &[RealizedPosting],
) -> std::collections::HashMap<usize, Uuid> {
    // All viable (occurrence, posting) pairs. `payee_rank` is 0 when the payees match, 1
    // otherwise, so a payee-matching pair sorts ahead of an amount-only one (ADR 0026
    // 4d8.24.4 addendum) — this is what keeps two distinct merchants in one window from
    // cross-linking.
    let mut pairs: Vec<(i64, i64, i64, Uuid, usize, usize)> = Vec::new();
    for (oi, occ) in occurrences.iter().enumerate() {
        let tol = (occ.expected_amount_minor.abs() * AMOUNT_TOLERANCE_BPS / 10_000)
            .max(AMOUNT_TOLERANCE_FLOOR_MINOR);
        for (pi, posting) in postings.iter().enumerate() {
            let sign_ok = match occ.kind {
                ScheduleKind::Income => posting.minor_units > 0,
                ScheduleKind::Obligation => posting.minor_units < 0,
            };
            if !sign_ok {
                continue;
            }
            // The account gate (ADR 0047 §1): a pay-from-gated obligation links only
            // postings from that account (this is what lets a card-charged bill attach
            // its card history — and stops it stealing a same-window liquid posting);
            // un-gated occurrences (income, bills without a pay-from) link liquid
            // postings only, the pre-0047 behavior.
            let account_ok = match occ.autopay_account_id {
                Some(gate) => posting.account_id == gate,
                None => posting.is_liquid,
            };
            if !account_ok {
                continue;
            }
            let date_diff = (posting.occurred_on - occ.scheduled_date).num_days().abs();
            if date_diff > RECURRING_MATCH_WINDOW_DAYS {
                continue;
            }
            let amount_diff = (posting.minor_units.abs() - occ.expected_amount_minor).abs();
            let amount_ok = amount_diff <= tol;
            // A payee match requires BOTH sides to carry a (non-empty) merchant key.
            let payee_match = match (&occ.merchant_key, &posting.merchant_key) {
                (Some(o), Some(p)) => o == p,
                _ => false,
            };
            // Within-band amount OR a matching payee makes the pair viable; a drifted
            // amount still links when the payee matches, and a within-band amount still
            // links when the payee is missing (backward compatible).
            if !amount_ok && !payee_match {
                continue;
            }
            let payee_rank = i64::from(!payee_match);
            pairs.push((
                payee_rank,
                date_diff,
                amount_diff,
                posting.transaction_id,
                pi,
                oi,
            ));
        }
    }
    // Deterministic order: payee-match first, then nearest date, nearest amount, txn id, index.
    pairs.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
            .then(a.5.cmp(&b.5))
    });
    let mut occ_taken = vec![false; occurrences.len()];
    let mut posting_taken = vec![false; postings.len()];
    let mut links = std::collections::HashMap::new();
    for (_, _, _, txn_id, pi, oi) in pairs {
        if occ_taken[oi] || posting_taken[pi] {
            continue;
        }
        occ_taken[oi] = true;
        posting_taken[pi] = true;
        links.insert(oi, txn_id);
    }
    links
}

/// Read one recurring event's projected instances in a stable order — the retro-attach
/// surface (ADR 0047 §1): after approval the caller shows which occurrences matched a
/// realized transaction.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn read_rows_for_event(
    conn: &Connection,
    recurring_event_id: Uuid,
) -> Result<Vec<RecurringInstanceRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, recurring_event_id, scheduled_date, expected_amount_minor,
                currency, status, linked_transaction_id
         FROM recurring_event_instances
         WHERE recurring_event_id = ?1
         ORDER BY scheduled_date, id",
    )?;
    let rows = stmt
        .query_map([recurring_event_id], |r| {
            Ok(RecurringInstanceRow {
                id: r.get(0)?,
                recurring_event_id: r.get(1)?,
                scheduled_date: r.get(2)?,
                expected_amount_minor: r.get(3)?,
                currency: r.get(4)?,
                status: r.get(5)?,
                linked_transaction_id: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One past-due occurrence still waiting for the user to say what happened
/// (personal-cfo-4d8.27.7.6, ADR 0058).
#[derive(Debug, Clone)]
pub struct UnconfirmedOccurrence {
    pub recurring_event_id: Uuid,
    /// The bill's name, so the surface can identify it without a second read.
    pub name: String,
    pub scheduled_date: String,
    /// Positive magnitude of what was expected.
    pub expected_amount_minor: i64,
    pub currency: String,
    /// Whole days between the scheduled date and today, always >= 1.
    pub days_overdue: i64,
}

/// Obligations whose scheduled date has passed with nothing recorded against them.
///
/// The forecast cannot resolve these alone: either the household paid (and the cash is
/// already gone) or they did not (and it is still coming), and the projection is wrong in
/// one direction until someone says which.
///
/// **Proof of confirmation is `confirmed_obligations`, never
/// `recurring_event_instances.status`** (ADR 0058 §2). A confirm does not flip that status
/// when it lands more than ±7 days early or carries a $0 amount (`personal-cfo-vn6b`), so
/// a status-driven query would list occurrences the user already confirmed — and
/// confirming again posts a SECOND transaction. This reads the same table
/// `collect_bill_events` consults to suppress a confirmed occurrence, so this surface and
/// the projection always agree about what is outstanding.
///
/// Income occurrences are excluded: the question "did this bill get paid?" has an action
/// behind it, and a missing paycheck is not something the user confirms away.
///
/// **A pre-creation occurrence surfaces only while it is the bill's newest past occurrence,
/// full stop** (`personal-cfo-5ie.10`). The instance projection expands a schedule's
/// *entire* `[window_start, window_end]` lattice regardless of the anchor's position in it
/// (any occurrence is an equivalent anchor — `pay_schedule`, ADR 0047 §3), so a bill entered
/// today with a year-old anchor generates roughly a year of unlinked monthly occurrences,
/// none of which the household could have confirmed before the bill existed in the app.
/// Cutting the floor at `recurring_events.created_at` outright would also drop the ONE
/// occurrence that is genuinely actionable — entering a bill you know was already due is the
/// normal flow, and `created_at` is stamped essentially at the same moment as that due date.
/// So: an occurrence before `created_at` still surfaces when it is the bill's newest past
/// occurrence, but an *older* one only surfaces once it is at or after `created_at` — i.e.
/// once the household has actually been tracking the bill through it.
///
/// **The "newest past occurrence" reference is fixed over the bill's full history, not
/// recomputed against only what is still unresolved.** An earlier version of this query
/// computed it over the already-filtered (unlinked, unconfirmed) rows, which meant
/// confirming the one pre-creation row that surfaced made the *next-older* pre-creation
/// occurrence become the new "newest unresolved" one and surface in its place — repeatable
/// all the way back through the lattice, one confirm at a time, each one a real posted
/// transaction (ADR 0058's `MarkObligationPaid`/`ConfirmObligationEarly` path). Computing
/// the reference over every `scheduled_date < today` row for the bill regardless of link or
/// confirm status, in its own CTE evaluated before the resolved/unresolved filter, fixes it
/// at "the single most recent occurrence that has ever existed" and pins it there once and
/// for all: confirm it, and nothing older than `created_at` is offered again.
///
/// A bill that has been active for months, with several genuinely missed occurrences since
/// creation, is unaffected: every one of them already satisfies `scheduled_date >=
/// created_at` directly, so ADR 0058's "never hide a genuine backlog" consequence still
/// holds — this only caps the *pre-creation* portion of the lattice.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn read_unconfirmed_past_due(
    conn: &Connection,
    today: NaiveDate,
) -> Result<Vec<UnconfirmedOccurrence>, DbError> {
    let mut stmt = conn.prepare(
        "WITH latest_past AS (
             SELECT recurring_event_id, MAX(scheduled_date) AS scheduled_date
             FROM recurring_event_instances
             WHERE scheduled_date < ?1
             GROUP BY recurring_event_id
         )
         SELECT i.recurring_event_id, e.name, i.scheduled_date,
                i.expected_amount_minor, i.currency
         FROM recurring_event_instances i
         JOIN recurring_events e ON e.id = i.recurring_event_id
         JOIN latest_past l ON l.recurring_event_id = i.recurring_event_id
         WHERE i.scheduled_date < ?1
           AND i.linked_transaction_id IS NULL
           AND NOT EXISTS (
                 SELECT 1 FROM confirmed_obligations c
                  WHERE c.recurring_event_id = i.recurring_event_id
                    AND c.scheduled_date = i.scheduled_date
               )
           AND (
                 i.scheduled_date >= substr(e.created_at, 1, 10)
                 OR i.scheduled_date = l.scheduled_date
               )
         ORDER BY i.scheduled_date, e.name COLLATE NOCASE, i.recurring_event_id",
    )?;
    let today_str = today.to_string();
    let rows = stmt
        .query_map([&today_str], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    for (recurring_event_id, name, scheduled_date, expected_amount_minor, currency) in rows {
        let days_overdue = parse_date(&scheduled_date)
            .map(|d| (today - d).num_days())
            .unwrap_or(0);
        out.push(UnconfirmedOccurrence {
            recurring_event_id,
            name,
            scheduled_date,
            expected_amount_minor,
            currency,
            days_overdue,
        });
    }
    Ok(out)
}

/// Read all projected instances in a stable order.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn read_rows(conn: &Connection) -> Result<Vec<RecurringInstanceRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, recurring_event_id, scheduled_date, expected_amount_minor,
                currency, status, linked_transaction_id
         FROM recurring_event_instances
         ORDER BY scheduled_date, recurring_event_id, id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(RecurringInstanceRow {
                id: r.get(0)?,
                recurring_event_id: r.get(1)?,
                scheduled_date: r.get(2)?,
                expected_amount_minor: r.get(3)?,
                currency: r.get(4)?,
                status: r.get(5)?,
                linked_transaction_id: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, d).unwrap()
    }

    fn obligation(amount: i64, on: NaiveDate) -> Occurrence {
        obligation_named(amount, on, None)
    }

    fn obligation_named(amount: i64, on: NaiveDate, merchant: Option<&str>) -> Occurrence {
        Occurrence {
            recurring_event_id: Uuid::from_u128(1),
            scheduled_date: on,
            expected_amount_minor: amount,
            currency: "USD".to_owned(),
            kind: ScheduleKind::Obligation,
            merchant_key: merchant.and_then(normalize_key),
            autopay_account_id: None,
        }
    }

    /// An obligation gated to a pay-from account (ADR 0047 §1).
    fn obligation_gated(amount: i64, on: NaiveDate, gate: u128) -> Occurrence {
        Occurrence {
            autopay_account_id: Some(Uuid::from_u128(gate)),
            ..obligation(amount, on)
        }
    }

    /// A liquid-account posting (the default linking candidate).
    fn posting(txn: u128, minor: i64, on: NaiveDate) -> RealizedPosting {
        posting_named(txn, minor, on, None)
    }

    fn posting_named(
        txn: u128,
        minor: i64,
        on: NaiveDate,
        merchant: Option<&str>,
    ) -> RealizedPosting {
        RealizedPosting {
            transaction_id: Uuid::from_u128(txn),
            occurred_on: on,
            minor_units: minor,
            merchant_key: merchant.and_then(normalize_key),
            account_id: Uuid::from_u128(900),
            is_liquid: true,
        }
    }

    /// A posting on a specific (possibly non-liquid) account — the gate's test shape.
    fn posting_on_account(
        txn: u128,
        minor: i64,
        on: NaiveDate,
        account: u128,
        is_liquid: bool,
    ) -> RealizedPosting {
        RealizedPosting {
            account_id: Uuid::from_u128(account),
            is_liquid,
            ..posting(txn, minor, on)
        }
    }

    /// ADR 0047 §1: a card-gated bill links its card posting — and ONLY from its card.
    #[test]
    fn gated_obligation_links_only_its_own_accounts_postings() {
        let occ = vec![obligation_gated(180_000, day(1), 77)];
        // A same-amount liquid posting AND the card posting, both in-window: the gate must
        // pick the card posting even though the liquid one is nearer by date.
        let post = vec![
            posting_on_account(10, -180_000, day(2), 900, true),
            posting_on_account(11, -180_000, day(3), 77, false),
        ];
        let links = assign_links(&occ, &post);
        assert_eq!(
            links.get(&0),
            Some(&Uuid::from_u128(11)),
            "the gate admits only the pay-from account's posting"
        );
    }

    /// ADR 0047 §1: an un-gated obligation never links a card posting (pre-0047 behavior).
    #[test]
    fn ungated_obligation_ignores_card_postings() {
        let occ = vec![obligation(180_000, day(1))];
        let post = vec![posting_on_account(10, -180_000, day(1), 77, false)];
        let links = assign_links(&occ, &post);
        assert!(links.is_empty(), "no pay-from gate -> liquid postings only");
    }

    #[test]
    fn links_a_posting_within_tolerance() {
        // -1,800 bill on the 1st; a -1,800 posting on the 3rd (2 days, exact amount).
        let occ = vec![obligation(180_000, day(1))];
        let post = vec![posting(10, -180_000, day(3))];
        let links = assign_links(&occ, &post);
        assert_eq!(links.get(&0), Some(&Uuid::from_u128(10)));
    }

    #[test]
    fn does_not_link_an_off_amount_posting() {
        // Expected 1,800; posting 2,000 → diff 20,000 > tol (5% = 9,000).
        let occ = vec![obligation(180_000, day(1))];
        let post = vec![posting(10, -200_000, day(1))];
        assert!(assign_links(&occ, &post).is_empty());
    }

    #[test]
    fn does_not_link_an_out_of_window_posting() {
        // Same amount, 30 days away → beyond the 7-day window.
        let occ = vec![obligation(180_000, day(1))];
        let post = vec![posting(10, -180_000, day(31))];
        assert!(assign_links(&occ, &post).is_empty());
    }

    #[test]
    fn does_not_link_a_wrong_sign_posting() {
        // An obligation (outflow) must not match an inflow posting.
        let occ = vec![obligation(180_000, day(1))];
        let post = vec![posting(10, 180_000, day(1))];
        assert!(assign_links(&occ, &post).is_empty());
    }

    #[test]
    fn contention_resolves_to_the_nearest_date_deterministically() {
        // One posting on the 5th; two same-entity occurrences on the 5th and 10th.
        // The exact-date occurrence wins; the other stays unlinked.
        let occ = vec![obligation(180_000, day(5)), obligation(180_000, day(10))];
        let post = vec![posting(10, -180_000, day(5))];
        let links = assign_links(&occ, &post);
        assert_eq!(links.get(&0), Some(&Uuid::from_u128(10)));
        assert_eq!(links.get(&1), None);
    }

    #[test]
    fn each_posting_links_at_most_one_occurrence() {
        // Two occurrences both near one posting — only one may claim it.
        let occ = vec![obligation(180_000, day(4)), obligation(180_000, day(6))];
        let post = vec![posting(10, -180_000, day(5))];
        let links = assign_links(&occ, &post);
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn payee_match_links_an_amount_drifted_posting() {
        // A variable bill: expected 1,800 but the posting is 2,500 (diff 70,000 >> 5%
        // band). Off-band alone would NOT link, but the exact payee match within the
        // window does (ADR 0026 4d8.24.4 addendum).
        let occ = vec![obligation_named(180_000, day(1), Some("City Water"))];
        let post = vec![posting_named(10, -250_000, day(3), Some("CITY WATER"))];
        let links = assign_links(&occ, &post);
        assert_eq!(links.get(&0), Some(&Uuid::from_u128(10)));

        // Same off-band amount but a mismatched payee (and off band) does NOT link.
        let post_wrong = vec![posting_named(11, -250_000, day(3), Some("Comcast"))];
        assert!(assign_links(&occ, &post_wrong).is_empty());
    }

    #[test]
    fn distinct_payees_in_one_window_link_to_their_own_posting() {
        // Two same-sign, similar-amount obligations within one 7-day window, distinct
        // merchants; two postings, one per merchant. Payee-first ranking links each to
        // ITS OWN posting rather than cross-linking on date/amount alone.
        let occ = vec![
            obligation_named(180_000, day(5), Some("Netflix")),
            obligation_named(180_000, day(6), Some("Spotify")),
        ];
        let post = vec![
            posting_named(20, -180_000, day(6), Some("SPOTIFY")),
            posting_named(21, -180_000, day(5), Some("NETFLIX")),
        ];
        let links = assign_links(&occ, &post);
        assert_eq!(
            links.get(&0),
            Some(&Uuid::from_u128(21)),
            "Netflix occ → Netflix posting"
        );
        assert_eq!(
            links.get(&1),
            Some(&Uuid::from_u128(20)),
            "Spotify occ → Spotify posting"
        );
    }
}
