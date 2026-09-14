//! Credit-card statement + payment forecast (revolving cycles) and the loan
//! payment projection that feeds the same event stream (moved verbatim from
//! `forecast.rs`).

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Months, NaiveDate, Utc};
use core_money::{Currency, Money};
use forecast_engine::layer2::LumpInjection;
use forecast_engine::revolving::{
    minimum_due, project_revolving, CycleProjection, PaymentPolicy, RevolvingCycle, RevolvingTerms,
};
use forecast_engine::{AssumptionBasis, EventKind, ForecastEvent};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use pay_schedule::{Frequency, PaySchedule};

use super::aggregate::{distinct_spend_months, LAYER2_HISTORY_WINDOW_MONTHS};
use super::events::{
    clamped_day, day_of_month_after, day_of_month_on_or_after, liquid_starting_balance, prev_month,
    read_household_tz,
};
use super::read_variable_spend_history;
use super::{parse_date, parse_frequency};
use crate::forecast_overrides::{entity_overrides, EntityOverride};
use crate::{currency_from_code, DbError};

// ===== Credit-card statement + payment forecast (ADR 0039 §2, personal-cfo-4lhm) =====

/// How many upcoming billing cycles to project per card.
const CARD_FORECAST_CYCLES: usize = 3;

/// One projected billing cycle for a credit card (ADR 0039 §2): a statement balance composed
/// of known card-charged charges + projected ordinary variable card spend, with the minimum
/// due and the payment the card's repayment philosophy selects on the due date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardCycleView {
    /// The statement close date.
    pub close_date: NaiveDate,
    /// The payment due date.
    pub due_date: NaiveDate,
    /// Carried (owed) balance at the start of this cycle (ADR 0035 §4, llx5).
    pub carried_opening_balance_minor: i64,
    /// Known recurring card-charged bill charges posting in this cycle.
    pub known_charges_minor: i64,
    /// Projected ordinary variable card spend for this cycle (pezm.1 baseline).
    pub projected_variable_minor: i64,
    /// Projected finance charge (interest) accrued this cycle (llx5; 0 under grace / 0% APR).
    pub accrued_interest_minor: i64,
    /// Statement balance = carried opening + known charges + projected spend + interest.
    pub statement_balance_minor: i64,
    /// The minimum payment due (greater-of percent-of-balance / floor, capped at the balance).
    pub minimum_due_minor: i64,
    /// The full-statement payoff (== the statement balance).
    pub full_pay_minor: i64,
    /// The payment the card's repayment philosophy selects on the due date.
    pub forecast_payment_minor: i64,
    /// Whether `statement_balance_minor` is the user's recorded REAL statement rather than
    /// the estimate (feedback 2026-07-03) — the UI badges it as fact, not projection.
    pub statement_is_actual: bool,
    /// Whether this cycle has CLOSED as of the household calendar day (ADR 0039 addendum
    /// 2026-07-10 §1). Computed server-side so the UI's record-statement affordance can
    /// never disagree with the write guard across timezones.
    pub is_closed: bool,
}

/// A credit card's projected statement + payment forecast over its next cycles (4lhm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardStatementForecastView {
    /// The credit-card account id.
    pub account_id: Uuid,
    /// The account display name.
    pub account_name: String,
    /// The card's currency.
    pub currency: Currency,
    /// The credit limit in minor units, if set (for utilization); `0` when unset.
    pub credit_limit_minor: i64,
    /// The repayment philosophy token driving the projected payment.
    pub repayment_philosophy: String,
    /// The upcoming projected cycles, soonest first.
    pub cycles: Vec<CardCycleView>,
    /// Every user-recorded statement row for the card, newest first — the management list
    /// (ADR 0039 addendum 2026-07-10 §1): a row keyed to a close the derivation no longer
    /// leads with stays visible and clearable instead of silently replaying
    /// (personal-cfo-4d8.25.2).
    pub stored_statements: Vec<StoredStatementView>,
    /// Which signal tier produced the projected-spend estimate (ADR 0039 addendum
    /// 2026-07-10 §2): `card_history` / `statement_history` / `categorized_average` /
    /// `none` — surfaced for forecast explainability (PROJECT_PROFILE non-negotiable).
    pub estimate_basis: String,
    /// The statement estimator's walk-forward MAPE (bps, `card_statement_estimator_v1`), `0`
    /// when none is recorded yet — sizes the owed-balance uncertainty band on the Account
    /// Detail chart, exactly like the payment-date lump (ADR 0050).
    pub estimate_mape_bps: i64,
    /// How many samples (cycle windows or recorded statements) the estimate is fitted on.
    pub estimate_sample_cycles: i64,
}

/// One user-recorded statement row on a card (the management list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredStatementView {
    /// The cycle-close date the row is keyed to.
    pub close_date: NaiveDate,
    /// The recorded statement balance (minor units, >= 0).
    pub statement_balance_minor: i64,
    /// Whether the projection APPLIES this row (its close is on/before the household
    /// calendar day). A future-keyed stale row is inert — flagged, not silently replayed.
    /// Computed server-side so the badge can never disagree with the projection gate.
    pub applied: bool,
}

/// A credit card with a derivable billing cycle + its repayment terms.
pub(super) struct CardForecastTerms {
    pub(super) account_id: Uuid,
    ledger_account_id: Uuid,
    name: String,
    currency: Currency,
    close_day: u32,
    due_day: u32,
    apr_bps: i64,
    credit_limit_minor: i64,
    philosophy: String,
    min_percent_bps: i64,
    min_floor_minor: i64,
    fixed_amount_minor: i64,
    pub(super) paying_source: Option<Uuid>,
}

/// Project each credit card's upcoming statement balances + payments (ADR 0039 §2,
/// personal-cfo-4lhm). For each active `credit_facility` account whose `debt_terms` carry a
/// derivable cycle, the next [`CARD_FORECAST_CYCLES`] cycles are folded forward (ADR 0035 §4):
/// statement balance = carried owed balance + known card-charged bills + projected ordinary
/// variable card spend + revolving interest (`llx5`); the minimum due; and the payment the
/// repayment philosophy selects, carrying the remainder into the next cycle. The risk flag
/// (`lqmz`) is out of scope here.
///
/// # Errors
/// Returns [`DbError`] on a read failure or a malformed stored schedule.
pub(crate) fn card_statement_forecast(
    conn: &Connection,
    as_of: DateTime<Utc>,
) -> Result<Vec<CardStatementForecastView>, DbError> {
    let tz = read_household_tz(conn)?;
    let today = as_of.with_timezone(&tz).date_naive();
    // The statement view is the base projection; honor base assumption overlays on the bills.
    let overrides = entity_overrides(conn, &[])?;
    // One global walk-forward MAPE sizes every card's estimate uncertainty (ADR 0050; a
    // per-card breakdown is a later refinement). `0` until a metric row exists.
    let estimate_mape_bps =
        crate::forecast_backtest::latest_card_statement_mape(conn)?.map_or(0, |(bps, _sample)| bps);
    let mut out = Vec::new();
    for card in read_cards_with_cycle(conn)? {
        let cp = project_card_cycles(conn, &card, today, CARD_FORECAST_CYCLES, &overrides)?;
        let asserted = crate::debt::read_card_statement_balances(conn, card.account_id)?;
        let cycle_views = (0..cp.cycles.len())
            .map(|i| {
                let (_open, close_date, due_date) = cp.cycles[i];
                let proj = &cp.projections[i];
                CardCycleView {
                    close_date,
                    due_date,
                    carried_opening_balance_minor: proj.opening_cents,
                    known_charges_minor: cp.known[i],
                    projected_variable_minor: cp.variable[i],
                    accrued_interest_minor: proj.finance_charge_cents,
                    statement_balance_minor: proj.statement_balance_cents,
                    minimum_due_minor: proj.minimum_due_cents,
                    full_pay_minor: proj.statement_balance_cents,
                    forecast_payment_minor: proj.payment_cents,
                    // Matches the projection's gate: a row on a not-yet-closed cycle is
                    // inert (ADR 0039 addendum 2026-07-10 §1), so it must not badge as
                    // "Actual" either.
                    statement_is_actual: asserted.contains_key(&close_date) && close_date <= today,
                    is_closed: close_date <= today,
                }
            })
            .collect();
        let mut stored_statements: Vec<StoredStatementView> = asserted
            .iter()
            .map(
                |(&close_date, &statement_balance_minor)| StoredStatementView {
                    close_date,
                    statement_balance_minor,
                    applied: close_date <= today,
                },
            )
            .collect();
        stored_statements.sort_by_key(|row| std::cmp::Reverse(row.close_date));
        out.push(CardStatementForecastView {
            account_id: card.account_id,
            account_name: card.name,
            currency: card.currency,
            credit_limit_minor: card.credit_limit_minor,
            repayment_philosophy: card.philosophy,
            cycles: cycle_views,
            stored_statements,
            estimate_basis: cp.estimate_basis.to_owned(),
            estimate_mape_bps,
            estimate_sample_cycles: cp.estimate_samples as i64,
        });
    }
    Ok(out)
}

/// One past billing-cycle window for a card: the derived-from-imports charge total (when
/// the card's transaction history fully covers the window) and any user-recorded actual
/// statement — the statement-history capture surface (ADR 0039 addendum 2026-07-10 §2,
/// personal-cfo-4d8.25.4). Windows walk the statement close day backwards (due day for a
/// cycle-less card); newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardStatementHistoryView {
    /// The window's open date (the previous close).
    pub window_open: NaiveDate,
    /// The cycle-close date the window (and any recorded statement) is keyed to.
    pub close_date: NaiveDate,
    /// Sum of imported charges in `[open, close)` (sign-filtered, uncategorized included),
    /// or `None` when the card's history does not fully cover the window — a candidate the
    /// user can confirm into a recorded statement.
    pub derived_charges_minor: Option<i64>,
    /// The user-recorded actual statement for this close, if any.
    pub stored_statement_minor: Option<i64>,
}

/// The past cycle windows for one card with derived + recorded statement values
/// (personal-cfo-4d8.25.4). Empty when the account has no payment boundary (neither a
/// statement close day nor a payment due day).
///
/// # Errors
/// Returns [`DbError`] on a read failure or malformed stored dates.
pub(crate) fn card_statement_history(
    conn: &Connection,
    account_id: Uuid,
    as_of: DateTime<Utc>,
) -> Result<Vec<CardStatementHistoryView>, DbError> {
    let tz = read_household_tz(conn)?;
    let today = as_of.with_timezone(&tz).date_naive();
    let boundary: Option<i64> = conn
        .query_row(
            "SELECT COALESCE(statement_close_day, payment_due_day) FROM debt_terms
             WHERE account_id = ?1",
            [account_id],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten();
    let Some(boundary_day) = boundary else {
        return Ok(Vec::new());
    };
    let windows = past_cycle_windows(today, boundary_day as u32, ESTIMATOR_MAX_WINDOWS);
    let totals = card_charge_totals_per_window(conn, account_id, &windows)?;
    let bounds = card_posting_date_bounds(conn, account_id)?;
    let asserted = crate::debt::read_card_statement_balances(conn, account_id)?;
    Ok(windows
        .iter()
        .zip(&totals)
        .map(|(&window, &total)| CardStatementHistoryView {
            window_open: window.0,
            close_date: window.1,
            derived_charges_minor: bounds
                .is_some_and(|b| window_covered(window, b))
                .then_some(total),
            stored_statement_minor: asserted.get(&window.1).copied(),
        })
        .collect())
}

/// Project a card's revolving cycles (shared by the statement view `4lhm` and the liquid
/// card-payment collector `6wk.10`): `count` cycles from `today`, each carrying known
/// card-charged bills + prorated projected ordinary variable card spend, folded forward through
/// [`project_revolving`] from the card's owed opening balance (ADR 0035 §4).
struct CardProjection {
    cycles: Vec<(NaiveDate, NaiveDate, NaiveDate)>,
    known: Vec<i64>,
    variable: Vec<i64>,
    projections: Vec<CycleProjection>,
    /// Which signal tier produced the per-cycle spend estimate (ADR 0039 addendum
    /// 2026-07-10 §2): `card_history` / `statement_history` / `categorized_average` / `none`.
    estimate_basis: &'static str,
    /// How many samples (cycles or statements) the estimate is fitted on.
    estimate_samples: usize,
}

fn project_card_cycles(
    conn: &Connection,
    card: &CardForecastTerms,
    today: NaiveDate,
    count: usize,
    overrides: &HashMap<Uuid, EntityOverride>,
) -> Result<CardProjection, DbError> {
    let cycles = derive_card_cycles(today, card.close_day, card.due_day, count);
    project_cycles_from(conn, card, today, cycles, overrides)
}

/// The shared projection body over pre-derived `(open, close, due)` triples — used by the real
/// close-day cycles ([`derive_card_cycles`]) and the due-day pseudo-cycles
/// ([`derive_pseudo_cycles`], ADR 0039 addendum 2026-07-10 §3).
fn project_cycles_from(
    conn: &Connection,
    card: &CardForecastTerms,
    today: NaiveDate,
    cycles: Vec<(NaiveDate, NaiveDate, NaiveDate)>,
    overrides: &HashMap<Uuid, EntityOverride>,
) -> Result<CardProjection, DbError> {
    // Charges are projected FORWARD from today: the current cycle contributes only the portion
    // still to come, since today's owed balance already captures the elapsed part (ADR 0035 §4).
    let known = card_known_charges_per_cycle(conn, card.account_id, today, &cycles, overrides)?;
    // The multi-signal new-charges estimator (ADR 0039 addendum 2026-07-10 §2) — re-fitted on
    // every projection run, so it updates continually as data arrives (ADR 0026 §15).
    let (monthly_variable, estimate_basis, estimate_samples) =
        estimate_card_new_charges(conn, card, today)?;
    // The carried (owed) balance the projection opens from: a liability's stored balance is
    // negative, so the owed amount is its negation (a credit balance clamps to 0).
    let balance = crate::assertion_anchored_balance(conn, card.account_id, card.ledger_account_id)?;
    let opening = balance.saturating_neg().max(0);

    // Per cycle: the effective window is `[max(open, today), close)`. The current cycle uses only
    // its remaining days; full future cycles the whole span. Variable spend is prorated to that
    // fraction; interest accrues over those days.
    // A user-recorded REAL statement (feedback 2026-07-03) replaces the estimate for its
    // cycle; the fold then carries the owed balance forward from the asserted number.
    let asserted = crate::debt::read_card_statement_balances(conn, card.account_id)?;
    let mut variable = Vec::with_capacity(cycles.len());
    let rev_cycles: Vec<RevolvingCycle> = cycles
        .iter()
        .zip(&known)
        .map(|(&(open, close, _due), &k)| {
            let effective_days = (close - open.max(today)).num_days().max(0) as u32;
            let full_days = (close - open).num_days().max(1);
            let v = monthly_variable.saturating_mul(i64::from(effective_days)) / full_days;
            variable.push(v);
            RevolvingCycle {
                new_charges_cents: k.saturating_add(v),
                days_in_cycle: effective_days,
                // An actual statement can only exist for a cycle that has CLOSED (ADR 0039
                // addendum 2026-07-10 §1): a stale row keyed to a future close — recorded
                // while the leading cycle pointed elsewhere, or under the pre-4d8.23.1
                // derivation — must not replay an old statement into an upcoming cycle
                // (personal-cfo-4d8.25.2).
                statement_override_cents: if close <= today {
                    asserted.get(&close).copied()
                } else {
                    None
                },
            }
        })
        .collect();
    let policy = payment_policy(
        &card.philosophy,
        card.fixed_amount_minor,
        card.min_percent_bps,
        card.min_floor_minor,
    );
    let (min_percent_bps, min_floor_cents) =
        effective_min_terms(policy, card.min_percent_bps, card.min_floor_minor);
    let terms = RevolvingTerms {
        apr_bps: card.apr_bps,
        policy,
        min_percent_bps,
        min_floor_cents,
    };
    let projections = project_revolving(opening, &rev_cycles, &terms);
    Ok(CardProjection {
        cycles,
        known,
        variable,
        projections,
        estimate_basis,
        estimate_samples,
    })
}

/// Every active credit-card account with a derivable billing cycle (both close + due day set).
pub(super) fn read_cards_with_cycle(conn: &Connection) -> Result<Vec<CardForecastTerms>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.ledger_account_id, a.name, a.currency,
                dt.statement_close_day, dt.payment_due_day,
                COALESCE(dt.apr_bps, 0), COALESCE(dt.credit_limit_minor, 0),
                dt.repayment_philosophy,
                COALESCE(dt.min_payment_percent_bps, 0),
                COALESCE(dt.min_payment_floor_minor, 0),
                COALESCE(dt.fixed_amount_minor, 0),
                dt.paying_source_account_id
         FROM accounts a
         JOIN debt_terms dt ON dt.account_id = a.id
         WHERE a.cashflow_role = 'credit_facility'
           AND a.active = 1
           AND dt.statement_close_day IS NOT NULL
           AND dt.payment_due_day IS NOT NULL
         ORDER BY a.name COLLATE NOCASE, a.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, Uuid>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, i64>(9)?,
                r.get::<_, i64>(10)?,
                r.get::<_, i64>(11)?,
                r.get::<_, Option<Uuid>>(12)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut cards = Vec::new();
    for (
        id,
        ledger_id,
        name,
        code,
        close_day,
        due_day,
        apr_bps,
        credit_limit,
        philosophy,
        min_pct,
        min_floor,
        fixed,
        paying_source,
    ) in rows
    {
        cards.push(CardForecastTerms {
            account_id: id,
            ledger_account_id: ledger_id,
            name,
            currency: currency_from_code(&code)?,
            close_day: close_day as u32,
            due_day: due_day as u32,
            apr_bps,
            credit_limit_minor: credit_limit,
            philosophy,
            min_percent_bps: min_pct,
            min_floor_minor: min_floor,
            fixed_amount_minor: fixed,
            paying_source,
        });
    }
    Ok(cards)
}

/// The next `count` `(open, close, due)` date triples from `today`, oldest first. The sequence
/// begins at the earliest cycle whose statement is still UNPAID — i.e. the earliest close whose
/// due date is on/after `today`. Usually that's the first close on/after today; but when this
/// month's statement has already closed yet its payment isn't due until later (the grace window,
/// `close_day < today <= due_day`), that just-closed statement is the imminent one and must lead —
/// otherwise its due date is skipped and the next payment is mis-dated a month late
/// (personal-cfo-4d8.23.1). Each subsequent cycle closes a month later. `open` is the previous
/// close (the cycle covers `[open, close)`), used for bucketing charges and the interest window.
fn derive_card_cycles(
    today: NaiveDate,
    close_day: u32,
    due_day: u32,
    count: usize,
) -> Vec<(NaiveDate, NaiveDate, NaiveDate)> {
    let mut cycles = Vec::with_capacity(count);
    let mut close = day_of_month_on_or_after(today, close_day);
    // Step back to the prior statement's close when that statement, though already closed, is not
    // yet due — its due date is still on/after today AND strictly earlier than the current
    // statement's due date. The strict-earlier guard matters when a due day past a short month's
    // end clamps the prior statement's due date FORWARD onto the current statement's due date
    // (e.g. close 28 / due 29 across February): stepping back there would emit two cycles with the
    // same due date and double-count the payment (personal-cfo-4d8.23.1).
    let (py, pm) = prev_month(close.year(), close.month());
    let prev_close = clamped_day(py, pm, close_day);
    let prev_due = day_of_month_after(prev_close, due_day);
    if prev_due >= today && prev_due < day_of_month_after(close, due_day) {
        close = prev_close;
    }
    let (py, pm) = prev_month(close.year(), close.month());
    let mut open = clamped_day(py, pm, close_day);
    let mut last_due: Option<NaiveDate> = None;
    for _ in 0..count {
        let mut due = day_of_month_after(close, due_day);
        // Strictly increasing due dates (ADR 0039 addendum 2026-07-10 §4): a month-end clamp
        // can collide two closes onto one due date (close 30 / due 31 across February — the
        // Feb-close statement is due Mar 31, and the Mar-30 close also resolves to Mar 31).
        // The later statement's due advances to the next occurrence of the due day, so each
        // derived date carries exactly one payment (personal-cfo-4d8.23.9).
        if let Some(prev) = last_due {
            while due <= prev {
                due = day_of_month_after(due, due_day);
            }
        }
        last_due = Some(due);
        cycles.push((open, close, due));
        open = close;
        close = day_of_month_after(close, close_day);
    }
    cycles
}

/// Pseudo-cycles for a cycle-less card that has charged bills (ADR 0039 addendum 2026-07-10
/// §3, personal-cfo-4d8.23.10): with no `statement_close_day` the due day is the only boundary,
/// so each cycle closes AND falls due on it — window `[open, close)` runs due-to-due, bucketing
/// the card's charged bills into the payment that services them. Due dates strictly increase by
/// construction.
fn derive_pseudo_cycles(
    today: NaiveDate,
    due_day: u32,
    count: usize,
) -> Vec<(NaiveDate, NaiveDate, NaiveDate)> {
    let mut cycles = Vec::with_capacity(count);
    let mut close = day_of_month_on_or_after(today, due_day);
    let (py, pm) = prev_month(close.year(), close.month());
    let mut open = clamped_day(py, pm, due_day);
    for _ in 0..count {
        cycles.push((open, close, close));
        open = close;
        close = day_of_month_after(close, due_day);
    }
    cycles
}

/// Sum the known card-charged bill charges per cycle, **from `today` forward**. Each active
/// recurring bill paid from the card has its future occurrences bucketed into the cycle whose
/// `[open, close)` window contains them. Occurrences before `today` are elapsed (already in the
/// card's owed balance) and are skipped. Interval containment (not recomputing the close from
/// the charge) keeps a month-end-clamped monthly bill colliding with a 28–30 close day from
/// being double-counted into one cycle.
fn card_known_charges_per_cycle(
    conn: &Connection,
    card_id: Uuid,
    today: NaiveDate,
    cycles: &[(NaiveDate, NaiveDate, NaiveDate)],
    overrides: &HashMap<Uuid, EntityOverride>,
) -> Result<Vec<i64>, DbError> {
    let mut per_cycle = vec![0i64; cycles.len()];
    if cycles.is_empty() {
        return Ok(per_cycle);
    }
    let last_close = cycles[cycles.len() - 1].1;
    let mut stmt = conn.prepare(
        "SELECT id, amount_expected_minor, frequency, next_expected_date
         FROM recurring_events
         WHERE autopay_account_id = ?1 AND is_active = 1 AND include_in_forecast = 1
           AND next_expected_date IS NOT NULL",
    )?;
    let rows = stmt
        .query_map(params![card_id], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, amount_minor, freq_token, anchor_str) in rows {
        // Honor base + scenario overrides on the card-charged bill, exactly as the liquid bill
        // path does (amount override, windowed exclusion, anchor shift) — else the card payment
        // would count the raw amount and silently drop the override (6wk.10 review).
        let over = overrides.get(&id);
        if over.is_some_and(EntityOverride::fully_excluded) {
            continue;
        }
        let frequency = parse_frequency(&freq_token, "card bill")?;
        let base_anchor = parse_date(&anchor_str)?;
        let anchor = over.map_or(base_anchor, |o| o.anchor_or(base_anchor));
        for charge in PaySchedule::new(frequency, anchor).pay_dates(today, last_close) {
            let Some(expected) =
                over.map_or(Some(amount_minor), |o| o.amount_for(charge, amount_minor))
            else {
                continue; // excluded on this date
            };
            if let Some(idx) =
                (0..cycles.len()).find(|&i| cycles[i].0 <= charge && charge < cycles[i].1)
            {
                per_cycle[idx] = per_cycle[idx].saturating_add(expected);
            }
        }
    }
    Ok(per_cycle)
}

/// The card's projected ordinary variable spend for one cycle (≈ one month). v1 estimate: the
/// average spend over the **months the card was actually used** (`distinct_spend_months`), via
/// the `pezm.1` classifier (which drops one-offs). For a regularly-used card this is the true
/// monthly run-rate; for a sporadically-used card it is the "when used" average, so projecting
/// it onto every cycle is an upper estimate — refining sporadic-card projection is later work.
///
/// Per ADR 0039 §2 the projected spend should exclude a card-charged bill counted in
/// `known_charges`. v1 relies on the natural separation: this reads only `variable_regular` /
/// `variable_lumpy` categories, while a properly-categorized recurring bill is `deterministic`
/// and therefore already excluded; precise dedup by `recurring_event` link is deferred (it
/// needs the posting↔instance seam, personal-cfo-6wk.9 / 46jq).
fn projected_variable_card_spend(
    conn: &Connection,
    card_id: Uuid,
    today: NaiveDate,
) -> Result<i64, DbError> {
    let window_start = today
        .checked_sub_months(Months::new(LAYER2_HISTORY_WINDOW_MONTHS))
        .unwrap_or(today);
    let history = read_variable_spend_history(conn, window_start, today, Some(card_id))?;
    let months = distinct_spend_months(&history);
    if months == 0 {
        return Ok(0);
    }
    let total: i64 = history.iter().map(|o| o.amount_cents).sum();
    Ok(total / months as i64)
}

// ===== Statement new-charges estimator (ADR 0039 addendum 2026-07-10 §2, =====
// ===== personal-cfo-4d8.25.5)                                            =====

/// How many past cycle windows the estimator fits on, at most.
const ESTIMATOR_MAX_WINDOWS: usize = 12;
/// The minimum samples a tier needs before it is trusted over the next tier down.
const ESTIMATOR_MIN_SAMPLES: usize = 2;

/// The past `count` cycle windows `(open, close]`-style pairs `(open, close)`, most recent
/// first, walking the boundary day backwards from the most recent close on/before `today`.
fn past_cycle_windows(
    today: NaiveDate,
    boundary_day: u32,
    count: usize,
) -> Vec<(NaiveDate, NaiveDate)> {
    let mut close = day_of_month_on_or_after(today, boundary_day);
    if close > today {
        let (py, pm) = prev_month(close.year(), close.month());
        close = clamped_day(py, pm, boundary_day);
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let (py, pm) = prev_month(close.year(), close.month());
        let open = clamped_day(py, pm, boundary_day);
        out.push((open, close));
        close = open;
    }
    out
}

/// The card's posting-history span `(earliest, latest)` (any sign), or `None` when the card
/// has no transaction history at all — the coverage bounds for T1 windows: a window counts
/// only when history spans BOTH edges, else a lagging import would feed zero-spend samples
/// and drag the median toward bills-only (adversarial review of 4d8.25.5).
fn card_posting_date_bounds(
    conn: &Connection,
    card_id: Uuid,
) -> Result<Option<(NaiveDate, NaiveDate)>, DbError> {
    let bounds: (Option<String>, Option<String>) = conn.query_row(
        "SELECT min(substr(lt.occurred_at, 1, 10)), max(substr(lt.occurred_at, 1, 10))
         FROM ledger_postings lp
         JOIN ledger_transactions lt ON lt.id = lp.transaction_id
         JOIN ledger_accounts la ON la.id = lp.ledger_account_id
         JOIN accounts a ON a.ledger_account_id = la.id
         WHERE lt.voided_at IS NULL AND a.id = ?1",
        [card_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    match bounds {
        (Some(min), Some(max)) => Ok(Some((parse_date(&min)?, parse_date(&max)?))),
        _ => Ok(None),
    }
}

/// Whether history spanning `(earliest, latest)` fully covers the window `[open, close)`:
/// the last coverable day is `close − 1`, so `latest` must reach it.
fn window_covered(
    (open, close): (NaiveDate, NaiveDate),
    (earliest, latest): (NaiveDate, NaiveDate),
) -> bool {
    open >= earliest && close <= latest + chrono::Days::new(1)
}

/// Per-window totals of the card's CHARGES — sign-filtered outflow postings
/// (`minor_units < 0`, so payments/refunds are excluded), **including uncategorized rows**
/// (the whole point of tier T1: raw OFX history counts). Returned as positive magnitudes,
/// one per `windows` entry (window = `[open, close)`).
fn card_charge_totals_per_window(
    conn: &Connection,
    card_id: Uuid,
    windows: &[(NaiveDate, NaiveDate)],
) -> Result<Vec<i64>, DbError> {
    let mut totals = vec![0i64; windows.len()];
    // Span by min/max, not positional — callers pass windows newest-first (estimator) AND
    // oldest-first (walk-forward backtest); a positional span would invert to an empty range.
    let Some(span_start) = windows.iter().map(|w| w.0).min() else {
        return Ok(totals);
    };
    let span_end = windows.iter().map(|w| w.1).max().unwrap_or(span_start);
    let mut stmt = conn.prepare(
        "SELECT substr(lt.occurred_at, 1, 10), lp.minor_units
         FROM ledger_postings lp
         JOIN ledger_transactions lt ON lt.id = lp.transaction_id
         JOIN ledger_accounts la ON la.id = lp.ledger_account_id
         JOIN accounts a ON a.ledger_account_id = la.id
         WHERE lt.voided_at IS NULL
           AND lp.minor_units < 0
           AND a.id = ?1
           AND substr(lt.occurred_at, 1, 10) >= ?2
           AND substr(lt.occurred_at, 1, 10) < ?3",
    )?;
    let rows = stmt.query_map(
        params![card_id, span_start.to_string(), span_end.to_string()],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    )?;
    for row in rows {
        let (date_str, minor) = row?;
        let date = parse_date(&date_str)?;
        if let Some(idx) = windows
            .iter()
            .position(|&(open, close)| open <= date && date < close)
        {
            totals[idx] = totals[idx].saturating_add(minor.saturating_neg());
        }
    }
    Ok(totals)
}

/// Sum of the card's active, forecast-included charged bills scheduled in each window —
/// the deterministic backbone the statistical tiers must NOT re-count (ADR 0039 §2 dedupe).
///
/// A bill only deducts from windows it could actually have POSTED in: the schedule's
/// mathematical expansion runs backwards without bound, so an unfloored subtraction would
/// let a freshly-created bill cancel itself out of the forecast — every historical window
/// loses the amount the bill never posted, the median drops by exactly that amount, and the
/// forward `known_charges` adds it back, leaving the projection unchanged while the bill's
/// charge-date outflow is suppressed (adversarial review of 4d8.25.5 — money vanishing).
/// The per-bill floor is the earlier of its earliest LINKED realized instance (a bill
/// promoted from history has matched postings that ARE in the raw totals — full-span dedupe
/// is correct there) and its `created_at` day. Overrides do not apply to the past.
fn scheduled_bill_totals_in_windows(
    conn: &Connection,
    card_id: Uuid,
    windows: &[(NaiveDate, NaiveDate)],
) -> Result<Vec<i64>, DbError> {
    let mut totals = vec![0i64; windows.len()];
    let Some(span_start) = windows.iter().map(|w| w.0).min() else {
        return Ok(totals);
    };
    let span_end = windows.iter().map(|w| w.1).max().unwrap_or(span_start);
    let mut stmt = conn.prepare(
        "SELECT e.amount_expected_minor, e.frequency, e.next_expected_date,
                substr(e.created_at, 1, 10),
                (SELECT min(i.scheduled_date) FROM recurring_event_instances i
                  WHERE i.recurring_event_id = e.id
                    AND i.linked_transaction_id IS NOT NULL)
         FROM recurring_events e
         WHERE e.autopay_account_id = ?1 AND e.is_active = 1 AND e.include_in_forecast = 1
           AND e.next_expected_date IS NOT NULL",
    )?;
    let rows = stmt
        .query_map(params![card_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (amount_minor, freq_token, anchor_str, created_str, linked_min) in rows {
        let frequency = parse_frequency(&freq_token, "card bill")?;
        let anchor = parse_date(&anchor_str)?;
        let created = parse_date(&created_str)?;
        let floor = match linked_min {
            Some(linked) => created.min(parse_date(&linked)?),
            None => created,
        };
        for charge in PaySchedule::new(frequency, anchor).pay_dates(span_start, span_end) {
            if charge < floor {
                continue;
            }
            if let Some(idx) = windows
                .iter()
                .position(|&(open, close)| open <= charge && charge < close)
            {
                totals[idx] = totals[idx].saturating_add(amount_minor);
            }
        }
    }
    Ok(totals)
}

/// Deterministic integer median (even length → mean of the two middles).
fn median_minor(values: &mut [i64]) -> i64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        // i128 keeps the sum overflow-safe for any pair of i64 amounts.
        ((i128::from(values[mid - 1]) + i128::from(values[mid])) / 2) as i64
    }
}

/// The per-cycle **new-charges estimate beyond known bills** for a card, from the best
/// available signal tier (ADR 0039 addendum 2026-07-10 §2):
///
/// - **T1 `card_history`** — median of per-window charge totals (uncategorized included,
///   payments sign-excluded) over the past complete cycle windows, minus each window's
///   scheduled bill total (dedupe vs the deterministic backbone). Needs ≥2 windows fully
///   covered by the card's transaction history.
/// - **T2 `statement_history`** — median of the user's recorded actual statements minus the
///   typical scheduled-bill total. Statements bundle carried balance + interest, so this
///   tier only applies under a full-payment policy (carried ≈ 0); needs ≥2 applied rows.
/// - **T3 `categorized_average`** — the pre-existing categorized variable-spend average
///   ([`projected_variable_card_spend`]), until `9h1s` replaces it with Layer-2 bands.
///
/// Returns `(per_cycle_minor, basis_token, samples)`. Fitted on read — every projection run
/// re-estimates, so the forecast updates continually as data arrives (ADR 0026 §15).
fn estimate_card_new_charges(
    conn: &Connection,
    card: &CardForecastTerms,
    today: NaiveDate,
) -> Result<(i64, &'static str, usize), DbError> {
    let windows = past_cycle_windows(today, card.close_day, ESTIMATOR_MAX_WINDOWS);

    // T1: raw card history, bucketed per derived cycle window. Note (documented v1
    // limitation, deferred): imported interest/fee lines count as charges here, so a
    // revolver's T1 signal can bake in historical interest that project_revolving also
    // models — see the follow-up bead filed on the 4d8.25 wave.
    if let Some(bounds) = card_posting_date_bounds(conn, card.account_id)? {
        let complete: Vec<(NaiveDate, NaiveDate)> = windows
            .iter()
            .copied()
            .filter(|&w| window_covered(w, bounds))
            .collect();
        if complete.len() >= ESTIMATOR_MIN_SAMPLES {
            let totals = card_charge_totals_per_window(conn, card.account_id, &complete)?;
            let bills = scheduled_bill_totals_in_windows(conn, card.account_id, &complete)?;
            let mut beyond: Vec<i64> = totals
                .iter()
                .zip(&bills)
                .map(|(&t, &b)| t.saturating_sub(b).max(0))
                .collect();
            return Ok((median_minor(&mut beyond), "card_history", complete.len()));
        }
    }

    // T2: recorded statement history — full-payment policies only (a revolver's statement
    // bundles carried balance + interest, which would overstate new charges).
    let full_payer = matches!(
        payment_policy(
            &card.philosophy,
            card.fixed_amount_minor,
            card.min_percent_bps,
            card.min_floor_minor,
        ),
        PaymentPolicy::FullStatement
    );
    if full_payer {
        let asserted = crate::debt::read_card_statement_balances(conn, card.account_id)?;
        let mut applied: Vec<i64> = asserted
            .iter()
            .filter(|(&close, _)| close <= today)
            .map(|(_, &v)| v)
            .collect();
        if applied.len() >= ESTIMATOR_MIN_SAMPLES {
            let samples = applied.len();
            let median_statement = median_minor(&mut applied);
            let mut bills = scheduled_bill_totals_in_windows(conn, card.account_id, &windows)?;
            let typical_bills = median_minor(&mut bills);
            return Ok((
                median_statement.saturating_sub(typical_bills).max(0),
                "statement_history",
                samples,
            ));
        }
    }

    // T3: the categorized variable-spend average (pre-estimator behavior).
    let average = projected_variable_card_spend(conn, card.account_id, today)?;
    if average > 0 {
        return Ok((average, "categorized_average", 0));
    }
    Ok((0, "none", 0))
}

/// Walk-forward `(predicted, realized)` charge pairs per card for the estimator backtest
/// metric (ADR 0039 addendum 2026-07-10 §2, personal-cfo-4d8.25.6): for each fully-covered
/// past window with at least [`ESTIMATOR_MIN_SAMPLES`] prior covered windows, the prediction
/// is exactly what the T1 estimator would have produced at that window's open — the median
/// of the PRIOR windows' beyond-bills totals plus the window's scheduled bills — compared to
/// the window's realized charge total.
pub(crate) fn card_statement_walk_forward_pairs(
    conn: &Connection,
    today: NaiveDate,
) -> Result<Vec<(i64, i64)>, DbError> {
    let mut pairs = Vec::new();
    for card in read_cards_with_cycle(conn)? {
        let Some(bounds) = card_posting_date_bounds(conn, card.account_id)? else {
            continue;
        };
        let mut covered: Vec<(NaiveDate, NaiveDate)> =
            past_cycle_windows(today, card.close_day, ESTIMATOR_MAX_WINDOWS)
                .into_iter()
                .filter(|&w| window_covered(w, bounds))
                .collect();
        covered.reverse(); // oldest first for the walk
        if covered.len() <= ESTIMATOR_MIN_SAMPLES {
            continue;
        }
        let totals = card_charge_totals_per_window(conn, card.account_id, &covered)?;
        let bills = scheduled_bill_totals_in_windows(conn, card.account_id, &covered)?;
        let beyond: Vec<i64> = totals
            .iter()
            .zip(&bills)
            .map(|(&t, &b)| t.saturating_sub(b).max(0))
            .collect();
        for i in ESTIMATOR_MIN_SAMPLES..covered.len() {
            let mut prior = beyond[..i].to_vec();
            let predicted = median_minor(&mut prior).saturating_add(bills[i]);
            pairs.push((predicted, totals[i]));
        }
    }
    Ok(pairs)
}

/// Map a stored `repayment_philosophy` token (ADR 0035 §1) onto the pure crate's
/// [`PaymentPolicy`]. `unknown` (and any unexpected token) defaults to the minimum
/// when minimum terms are configured; with NO minimum terms a zero minimum would
/// project a $0 payment, so the card's due dates silently vanish from the forecast
/// (feedback 2026-07-03). That degenerate case falls back to the statement balance —
/// the descriptive "the bill gets paid" assumption until the user picks a philosophy.
fn payment_policy(
    philosophy: &str,
    fixed_amount_minor: i64,
    min_percent_bps: i64,
    min_floor_minor: i64,
) -> PaymentPolicy {
    match philosophy {
        "pay_in_full" | "pay_statement_balance" | "pay_current_balance" => {
            PaymentPolicy::FullStatement
        }
        "pay_fixed_amount" if fixed_amount_minor > 0 => PaymentPolicy::Fixed(fixed_amount_minor),
        // A zero/unset fixed amount degrades to the minimum rule rather than a perpetual
        // $0 payment (adversarial review of 4d8.23.10) — the caller substitutes the
        // ADR 0035 §5 default minimum terms when neither is configured.
        "pay_fixed_amount" | "pay_minimum" => PaymentPolicy::Minimum,
        _ if min_percent_bps > 0 || min_floor_minor > 0 => PaymentPolicy::Minimum,
        _ => PaymentPolicy::FullStatement,
    }
}

/// The minimum terms the revolving fold should use for a card: the stored terms, or — when a
/// minimum-paying policy has NEITHER term configured — the ADR 0035 §5 default rule
/// (1%-of-balance / $25), exactly like the naive loan path (`loan_payment`). Without this a
/// pay_minimum card with unset terms projects `minimum_due(_, 0, 0) == 0` forever: its due
/// dates vanish from the forecast while its charged bills are suppressed — money silently
/// disappearing (adversarial review of 4d8.23.10).
fn effective_min_terms(
    policy: PaymentPolicy,
    min_percent_bps: i64,
    min_floor_minor: i64,
) -> (i64, i64) {
    if matches!(policy, PaymentPolicy::Minimum) && min_percent_bps == 0 && min_floor_minor == 0 {
        (DEFAULT_MIN_PERCENT_BPS, DEFAULT_MIN_FLOOR_MINOR)
    } else {
        (min_percent_bps, min_floor_minor)
    }
}

// ===== Loan payment outflow (ADR 0035 §3, personal-cfo-6wk.4) =====

/// A loan account whose `debt_terms` can produce a payment (both a due day + a payment amount).
pub(crate) struct LoanPaymentTerms {
    pub account_id: Uuid,
    ledger_account_id: Uuid,
    pub name: String,
    pub currency: Currency,
    payment_due_day: u32,
    pub philosophy: String,
    pub fixed_amount_minor: i64,
    min_percent_bps: i64,
    min_floor_minor: i64,
    pub(super) paying_source: Option<Uuid>,
}

/// Every active `loan_liability` account whose `debt_terms` carry a `payment_due_day`.
pub(super) fn read_loans_with_payment(conn: &Connection) -> Result<Vec<LoanPaymentTerms>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.ledger_account_id, a.name, a.currency, dt.payment_due_day,
                dt.repayment_philosophy, COALESCE(dt.fixed_amount_minor, 0),
                COALESCE(dt.min_payment_percent_bps, 0), COALESCE(dt.min_payment_floor_minor, 0),
                dt.paying_source_account_id
         FROM accounts a
         JOIN debt_terms dt ON dt.account_id = a.id
         WHERE a.cashflow_role = 'loan_liability'
           AND a.active = 1
           AND dt.payment_due_day IS NOT NULL
         ORDER BY a.name COLLATE NOCASE, a.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, Uuid>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, Option<Uuid>>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut loans = Vec::new();
    for (id, ledger, name, code, due_day, philosophy, fixed, min_pct, min_floor, paying) in rows {
        loans.push(LoanPaymentTerms {
            account_id: id,
            ledger_account_id: ledger,
            name,
            currency: currency_from_code(&code)?,
            payment_due_day: due_day as u32,
            philosophy,
            fixed_amount_minor: fixed,
            min_percent_bps: min_pct,
            min_floor_minor: min_floor,
            paying_source: paying,
        });
    }
    Ok(loans)
}

/// The loan's payment for the projection: `(amount, recurring_monthly)`, or `None` when nothing
/// is owed or no amount is derivable. A full-payoff philosophy pays the owed balance **once**;
/// fixed / minimum philosophies pay each month.
fn loan_payment(
    philosophy: &str,
    owed: i64,
    fixed: i64,
    min_percent_bps: i64,
    min_floor_minor: i64,
) -> Option<(i64, bool)> {
    if owed <= 0 {
        return None;
    }
    match philosophy {
        "pay_fixed_amount" if fixed > 0 => Some((fixed, true)),
        "pay_in_full" | "pay_statement_balance" | "pay_current_balance" => Some((owed, false)),
        // pay_minimum / unknown (ADR 0035 §1 default). When neither minimum term is set, fall
        // back to the ADR 0035 §5 default rule (1%-of-balance-or-$25) so the payment projects a
        // realistic amount rather than $0 (which would make cash look healthier than reality).
        _ => {
            let (pct, floor) = if min_percent_bps == 0 && min_floor_minor == 0 {
                (DEFAULT_MIN_PERCENT_BPS, DEFAULT_MIN_FLOOR_MINOR)
            } else {
                (min_percent_bps, min_floor_minor)
            };
            let minimum = minimum_due(owed, pct, floor);
            (minimum > 0).then_some((minimum, true))
        }
    }
}

/// The ADR 0035 §5 default minimum-payment rule (1%-of-balance / $25) applied when a loan's
/// `debt_terms` carry neither a percent nor a floor.
pub(super) const DEFAULT_MIN_PERCENT_BPS: i64 = 100;
pub(super) const DEFAULT_MIN_FLOOR_MINOR: i64 = 2_500;

/// The loan accounts that actually contribute a payment to the current forecast, alongside the
/// forecast's fold currency: in that currency, owed `> 0`, with a derivable payment — exactly the
/// set [`collect_loan_payment_events`] projects. A paid-off / never-asserted / foreign-currency
/// loan is excluded because it emits nothing. Used by the double-count detection
/// (personal-cfo-6wk.11) so it only warns about loans that are genuinely counted twice.
pub(crate) fn loans_emitting_payments(
    conn: &Connection,
) -> Result<(Currency, Vec<LoanPaymentTerms>), DbError> {
    let (currency, _) = liquid_starting_balance(conn)?;
    let mut out = Vec::new();
    for loan in read_loans_with_payment(conn)? {
        if loan.currency != currency {
            continue;
        }
        let owed =
            crate::assertion_anchored_balance(conn, loan.account_id, loan.ledger_account_id)?
                .saturating_neg()
                .max(0);
        if loan_payment(
            &loan.philosophy,
            owed,
            loan.fixed_amount_minor,
            loan.min_percent_bps,
            loan.min_floor_minor,
        )
        .is_none()
        {
            continue;
        }
        out.push(loan);
    }
    Ok((currency, out))
}

/// Project each loan account's payment as a liquid outflow on its `payment_due_day` (ADR 0035
/// §3): `−amount` leaves the paying source. The aggregate carries it (a debt payment is a real
/// outflow, not a self-cancelling transfer); `build_attribution` routes it to the paying
/// source in the per-account path. The owed balance opens from the loan's current balance
/// (negated); its trajectory / amortization interest split are the debt view's concern
/// (personal-cfo-6wk.5).
///
/// A loan whose currency differs from the forecast (liquid) currency is **skipped** — the
/// forecast has no offline FX rate (personal-cfo-d63), and mixing currencies into the event
/// stream would fail the whole projection. Such a loan simply isn't modeled in v1.
///
/// **Do not model the same loan twice:** a loan tracked as a recurring `loan_payment` bill
/// (`collect_bill_events`) AND as a `loan_liability` account with `debt_terms` would be
/// double-counted — the two collectors read disjoint tables with no link between them. Pick one
/// (a warning on the overlap is personal-cfo-6wk.11).
pub(super) fn collect_loan_payment_events(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    currency: Currency,
    events: &mut Vec<ForecastEvent>,
    names: &mut HashMap<Uuid, String>,
) -> Result<(), DbError> {
    for loan in read_loans_with_payment(conn)? {
        emit_naive_debt_payments(conn, &loan, start, end, currency, events, names)?;
    }
    Ok(())
}

/// Emit one debt account's payment outflows over `[start, end]` — the NAIVE due-date projection
/// shared by loans and close-day-less cards (ADR 0035 §3 / ADR 0039 addendum 2026-07-06,
/// personal-cfo-4d8.23.2): the `repayment_philosophy`'s payment on the current owed balance, on
/// each `payment_due_day` (a full-payoff philosophy pays once; fixed / minimum pay monthly),
/// defaulting to the ADR 0035 §5 minimum when no philosophy or minimum terms are set. Nothing is
/// emitted when the debt is paid off, no amount is derivable, or its currency differs from the
/// forecast's fold currency (no offline FX). Both loans and cards surface as `LoanPayment`.
fn emit_naive_debt_payments(
    conn: &Connection,
    terms: &LoanPaymentTerms,
    start: NaiveDate,
    end: NaiveDate,
    currency: Currency,
    events: &mut Vec<ForecastEvent>,
    names: &mut HashMap<Uuid, String>,
) -> Result<(), DbError> {
    if terms.currency != currency {
        return Ok(());
    }
    let owed = crate::assertion_anchored_balance(conn, terms.account_id, terms.ledger_account_id)?
        .saturating_neg()
        .max(0);
    let Some((amount, recurring)) = loan_payment(
        &terms.philosophy,
        owed,
        terms.fixed_amount_minor,
        terms.min_percent_bps,
        terms.min_floor_minor,
    ) else {
        return Ok(());
    };
    let outflow = Money::new(amount, terms.currency).checked_neg()?;
    let due_dates = if recurring {
        let anchor = day_of_month_on_or_after(start, terms.payment_due_day);
        PaySchedule::new(Frequency::Monthly, anchor).pay_dates(start, end)
    } else {
        let first = day_of_month_on_or_after(start, terms.payment_due_day);
        if first <= end {
            vec![first]
        } else {
            Vec::new()
        }
    };
    for date in due_dates {
        events.push(ForecastEvent {
            occurs_on: date,
            kind: EventKind::LoanPayment,
            amount: outflow,
            source_event_id: terms.account_id,
            assumption_basis: AssumptionBasis::RecurringSchedule {
                frequency: Frequency::Monthly,
            },
        });
    }
    names.insert(terms.account_id, format!("{} payment", terms.name));
    Ok(())
}

/// Every active `credit_facility` with a `payment_due_day` but **no** `statement_close_day` — the
/// naive, cycle-less cards (ADR 0039 addendum, personal-cfo-4d8.23.2). Returns the same payment
/// terms shape as a loan; these project through [`emit_naive_debt_payments`], disjoint from the
/// cycle cards ([`read_cards_with_cycle`] requires a close day).
pub(super) fn read_cards_without_cycle(
    conn: &Connection,
) -> Result<Vec<LoanPaymentTerms>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.ledger_account_id, a.name, a.currency, dt.payment_due_day,
                dt.repayment_philosophy, COALESCE(dt.fixed_amount_minor, 0),
                COALESCE(dt.min_payment_percent_bps, 0), COALESCE(dt.min_payment_floor_minor, 0),
                dt.paying_source_account_id
         FROM accounts a
         JOIN debt_terms dt ON dt.account_id = a.id
         WHERE a.cashflow_role = 'credit_facility'
           AND a.active = 1
           AND dt.payment_due_day IS NOT NULL
           AND dt.statement_close_day IS NULL
         ORDER BY a.name COLLATE NOCASE, a.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, Uuid>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, Option<Uuid>>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut cards = Vec::new();
    for (id, ledger, name, code, due_day, philosophy, fixed, min_pct, min_floor, paying) in rows {
        cards.push(LoanPaymentTerms {
            account_id: id,
            ledger_account_id: ledger,
            name,
            currency: currency_from_code(&code)?,
            payment_due_day: due_day as u32,
            philosophy,
            fixed_amount_minor: fixed,
            min_percent_bps: min_pct,
            min_floor_minor: min_floor,
            paying_source: paying,
        });
    }
    Ok(cards)
}

/// Project each credit card's **payment** as a liquid outflow on its cycle due dates (ADR 0039
/// §2, personal-cfo-6wk.10): one `−forecast_payment_minor` per cycle — the payment its
/// `repayment_philosophy` selects on the projected statement (known charges, projected variable,
/// interest) — from the card's paying source. This is the card's real liquid impact, modeled once
/// at the card level: it supersedes 6wk.8's per-bill retiming (those bills are now suppressed in
/// `collect_bill_events`) and folds the revolving balance forward across the whole horizon. A
/// foreign-currency card is skipped (no offline FX).
pub(super) fn collect_card_payment_events(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    currency: Currency,
    overrides: &HashMap<Uuid, EntityOverride>,
    events: &mut Vec<ForecastEvent>,
    names: &mut HashMap<Uuid, String>,
) -> Result<(), DbError> {
    let today = start;
    // Enough monthly cycles to reach the end of the window, plus a small buffer.
    let months = (i64::from(end.year()) - i64::from(today.year())) * 12 + i64::from(end.month())
        - i64::from(today.month());
    let count = usize::try_from(months.max(0)).unwrap_or(0) + 2;
    for card in read_cards_with_cycle(conn)? {
        if card.currency != currency {
            continue;
        }
        let cp = project_card_cycles(conn, &card, today, count, overrides)?;
        emit_cycle_payment_events(
            &cp,
            card.account_id,
            &card.name,
            start,
            end,
            currency,
            events,
            names,
        )?;
    }
    // Cards with only a due day (no derivable cycle): one WITH forecast-included charged bills
    // folds them into due-day pseudo-cycles — the bills are suppressed in `collect_bill_events`,
    // so the payment must carry them (ADR 0039 addendum 2026-07-10 §3, personal-cfo-4d8.23.10).
    // One with no such bills projects naively, like a loan — the philosophy's payment on the
    // owed balance on each due day (ADR 0039 addendum 2026-07-06, personal-cfo-4d8.23.2).
    // Disjoint from the cycle cards above (that set requires a close day).
    for card in read_cards_without_cycle(conn)? {
        if has_forecast_charged_bills(conn, card.account_id)? {
            // A foreign-currency card is skipped, exactly like the with-cycle loop.
            if card.currency != currency {
                continue;
            }
            let terms = CardForecastTerms {
                account_id: card.account_id,
                ledger_account_id: card.ledger_account_id,
                name: card.name.clone(),
                currency: card.currency,
                close_day: card.payment_due_day,
                due_day: card.payment_due_day,
                // Interest is not modeled on the cycle-less path (no APR is read for it),
                // matching the naive path it replaces.
                apr_bps: 0,
                credit_limit_minor: 0,
                philosophy: card.philosophy.clone(),
                min_percent_bps: card.min_percent_bps,
                min_floor_minor: card.min_floor_minor,
                fixed_amount_minor: card.fixed_amount_minor,
                paying_source: card.paying_source,
            };
            let cycles = derive_pseudo_cycles(today, card.payment_due_day, count);
            let cp = project_cycles_from(conn, &terms, today, cycles, overrides)?;
            emit_cycle_payment_events(
                &cp,
                card.account_id,
                &card.name,
                start,
                end,
                currency,
                events,
                names,
            )?;
        } else {
            emit_naive_debt_payments(conn, &card, start, end, currency, events, names)?;
        }
    }
    Ok(())
}

/// Emit one card's per-cycle payments (`−payment` on each due date inside the window) into the
/// event stream — shared by the real-cycle and pseudo-cycle paths.
#[allow(clippy::too_many_arguments)]
fn emit_cycle_payment_events(
    cp: &CardProjection,
    account_id: Uuid,
    name: &str,
    start: NaiveDate,
    end: NaiveDate,
    currency: Currency,
    events: &mut Vec<ForecastEvent>,
    names: &mut HashMap<Uuid, String>,
) -> Result<(), DbError> {
    for (i, &(_open, _close, due)) in cp.cycles.iter().enumerate() {
        if due < start || due > end {
            continue;
        }
        let payment = cp.projections[i].payment_cents;
        if payment <= 0 {
            continue;
        }
        events.push(ForecastEvent {
            occurs_on: due,
            kind: EventKind::LoanPayment,
            amount: Money::new(payment, currency).checked_neg()?,
            source_event_id: account_id,
            assumption_basis: AssumptionBasis::RecurringSchedule {
                frequency: Frequency::Monthly,
            },
        });
    }
    names.insert(account_id, format!("{name} payment"));
    Ok(())
}

/// `z_0.90` (the standard-normal 0.90 quantile) — the 80% central-interval half-width in σ, the
/// large-df limit the cash cone's Student-t already uses (`layer2::t_quantile_90` → 1.2816).
const CARD_LUMP_Z: f64 = 1.2816;
/// `sqrt(π/2)`: converts a mean-absolute error `E|Δ|` to a Gaussian σ (`E|Δ| = σ·sqrt(2/π)`).
const SQRT_HALF_PI: f64 = 1.2533;

/// The 80% one-sided spread (cents) of a full-payer card payment's variable component, from the
/// statement estimator's walk-forward MAPE. The MAPE (bps) is `E|Δ_variable| / total_charges`
/// (its denominator is the predicted TOTAL new charges, `known + variable`), so
/// `σ = sqrt(π/2)·E|Δ| = SQRT_HALF_PI·(mape/1e4)·total` and the 80% half-width is `z_0.90·σ`.
/// Sizing on the total (not the variable alone) avoids understating the CV (grounding survey B).
/// Zero for a non-positive MAPE or scale.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn card_lump_half_width(mape_bps: i64, total_charges_cents: i64) -> i64 {
    if mape_bps <= 0 || total_charges_cents <= 0 {
        return 0;
    }
    let mape_frac = mape_bps as f64 / 10_000.0;
    (CARD_LUMP_Z * SQRT_HALF_PI * mape_frac * total_charges_cents as f64).round() as i64
}

/// The analytic full-payer card lumps (ADR 0050, personal-cfo-4d8.27.5.7.3): per **pay-in-full**
/// card with a derivable cycle, the variable part of each future OPEN statement is uncertain, and
/// that uncertainty lands on the paying liquid account as a symmetric spread around the
/// (deterministic) payment on its due date. Returns `paying-account-id → [LumpInjection]`.
///
/// Scoped to full-payers because their grace-period payoff **decouples** the cycles — a variable
/// shock in one cycle affects only that payment, so an independent per-payment injection is exact.
/// Revolvers (minimum/fixed) carry the shock forward with compounding, a path-dependent case
/// deferred to the Monte-Carlo follow-up (personal-cfo-4d8.27.5.7.6). Closed cycles inject nothing
/// (the elapsed spend is already in the asserted balance / recorded statement).
///
/// # Errors
/// Returns [`DbError`] on a read failure or a malformed schedule.
pub(super) fn collect_card_lump_injections(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    currency: Currency,
    overrides: &HashMap<Uuid, EntityOverride>,
    mape_bps: i64,
) -> Result<HashMap<Uuid, Vec<LumpInjection>>, DbError> {
    let today = start;
    // Same cycle count as collect_card_payment_events, so a lump sits on every projected payment.
    let months = (i64::from(end.year()) - i64::from(today.year())) * 12 + i64::from(end.month())
        - i64::from(today.month());
    let count = usize::try_from(months.max(0)).unwrap_or(0) + 2;

    let mut out: HashMap<Uuid, Vec<LumpInjection>> = HashMap::new();
    for card in read_cards_with_cycle(conn)? {
        if card.currency != currency {
            continue;
        }
        let Some(paying) = card.paying_source else {
            continue;
        };
        // Full-payers only; revolvers are the path-dependent MC follow-up.
        if !matches!(
            payment_policy(
                &card.philosophy,
                card.fixed_amount_minor,
                card.min_percent_bps,
                card.min_floor_minor,
            ),
            PaymentPolicy::FullStatement
        ) {
            continue;
        }
        let cp = project_card_cycles(conn, &card, today, count, overrides)?;
        for (i, &(_open, close, due)) in cp.cycles.iter().enumerate() {
            if due < start || due > end || close <= today {
                continue; // outside the horizon, or a closed statement (no forward uncertainty)
            }
            let scale = cp.known[i].saturating_add(cp.variable[i]); // predicted total new charges
            let half_width = card_lump_half_width(mape_bps, scale);
            if half_width > 0 {
                out.entry(paying).or_default().push(LumpInjection {
                    date: due,
                    half_width_cents: half_width,
                });
            }
        }
    }
    Ok(out)
}

/// Whether the card has at least one active, forecast-included recurring bill charged to it —
/// the gate for the pseudo-cycle path, matching [`card_known_charges_per_cycle`]'s criteria
/// (and the `card_charged_bill_ids` suppression set in `events.rs`, which must stay in step).
fn has_forecast_charged_bills(conn: &Connection, card_id: Uuid) -> Result<bool, DbError> {
    let exists: i64 = conn.query_row(
        "SELECT EXISTS (
            SELECT 1 FROM recurring_events
             WHERE autopay_account_id = ?1 AND is_active = 1 AND include_in_forecast = 1
               AND next_expected_date IS NOT NULL)",
        [card_id],
        |r| r.get(0),
    )?;
    Ok(exists != 0)
}

#[cfg(test)]
mod derive_card_cycles_tests {
    use super::{derive_card_cycles, derive_pseudo_cycles};
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// personal-cfo-4d8.23.9 (ADR 0039 addendum 2026-07-10 §4): due dates are strictly
    /// increasing for EVERY (close_day, due_day, today) — a month-end clamp collision
    /// advances the later cycle's due to the next occurrence of the due day.
    #[test]
    fn due_dates_are_strictly_increasing_for_all_day_combinations() {
        let todays = [
            d(2026, 1, 15),
            d(2026, 2, 10),
            d(2026, 7, 9),
            d(2024, 2, 29), // leap day
            d(2026, 12, 31),
        ];
        for close_day in 1..=31 {
            for due_day in 1..=31 {
                for today in todays {
                    let cycles = derive_card_cycles(today, close_day, due_day, 14);
                    for w in cycles.windows(2) {
                        assert!(
                            w[1].2 > w[0].2,
                            "due date must strictly increase: close_day {close_day} \
                             due_day {due_day} today {today}: {:?} then {:?}",
                            w[0],
                            w[1]
                        );
                    }
                }
            }
        }
    }

    /// The concrete 4d8.23.9 collision: close 30 / due 31 across February. The Feb close
    /// clamps to Feb 28 with due Mar 31; the Mar-30 close would also resolve to Mar 31, so
    /// its due advances to Apr 30 — one payment per statement, never two on one date.
    #[test]
    fn close_30_due_31_across_february_advances_the_second_due() {
        let cycles = derive_card_cycles(d(2026, 2, 1), 30, 31, 3);
        assert_eq!(cycles[0], (d(2026, 1, 30), d(2026, 2, 28), d(2026, 3, 31)));
        assert_eq!(
            cycles[1],
            (d(2026, 2, 28), d(2026, 3, 30), d(2026, 4, 30)),
            "the colliding Mar-31 due advances to the next due-day occurrence"
        );
        assert_eq!(cycles[2], (d(2026, 3, 30), d(2026, 4, 30), d(2026, 5, 31)));
    }

    /// ADR 0039 addendum 2026-07-10 §3: pseudo-cycles for a cycle-less card close AND fall
    /// due on the due day, with contiguous due-to-due windows.
    #[test]
    fn pseudo_cycles_close_and_fall_due_on_the_due_day() {
        let cycles = derive_pseudo_cycles(d(2026, 7, 9), 17, 3);
        assert_eq!(cycles[0], (d(2026, 6, 17), d(2026, 7, 17), d(2026, 7, 17)));
        assert_eq!(cycles[1], (d(2026, 7, 17), d(2026, 8, 17), d(2026, 8, 17)));
        assert_eq!(cycles[2], (d(2026, 8, 17), d(2026, 9, 17), d(2026, 9, 17)));
    }

    /// The bug (personal-cfo-4d8.23.1): close day 3, due day 17, today 2026-07-06 — the July
    /// statement closed Jul 3 and is due Jul 17 (unpaid), so it must be cycle[0], NOT Aug 17.
    #[test]
    fn just_closed_unpaid_statement_leads() {
        let cycles = derive_card_cycles(d(2026, 7, 6), 3, 17, 3);
        assert_eq!(cycles[0], (d(2026, 6, 3), d(2026, 7, 3), d(2026, 7, 17)));
        assert_eq!(cycles[1], (d(2026, 7, 3), d(2026, 8, 3), d(2026, 8, 17)));
        assert_eq!(cycles[2], (d(2026, 8, 3), d(2026, 9, 3), d(2026, 9, 17)));
    }

    /// On the due date itself the cycle is still present (due >= today is inclusive).
    #[test]
    fn due_date_today_is_inclusive() {
        let cycles = derive_card_cycles(d(2026, 7, 17), 3, 17, 3);
        assert_eq!(cycles[0], (d(2026, 6, 3), d(2026, 7, 3), d(2026, 7, 17)));
    }

    /// One day past due, the paid statement is not resurrected: earliest due is Aug 17.
    #[test]
    fn day_after_due_drops_the_paid_statement() {
        let cycles = derive_card_cycles(d(2026, 7, 18), 3, 17, 3);
        assert_eq!(cycles[0], (d(2026, 7, 3), d(2026, 8, 3), d(2026, 8, 17)));
    }

    /// Before this month's close there is no step-back and no duplicate cycle.
    #[test]
    fn before_close_is_unchanged() {
        let cycles = derive_card_cycles(d(2026, 7, 2), 3, 17, 3);
        assert_eq!(cycles[0], (d(2026, 6, 3), d(2026, 7, 3), d(2026, 7, 17)));
        assert_eq!(cycles[1], (d(2026, 7, 3), d(2026, 8, 3), d(2026, 8, 17)));
    }

    /// Close day after due day (statement closes on the 25th, payment due the 17th of the NEXT
    /// month): triples stay strictly monotonic and non-duplicated with correct month rollovers.
    #[test]
    fn close_after_due_is_monotonic() {
        let cycles = derive_card_cycles(d(2026, 7, 10), 25, 17, 4);
        for w in cycles.windows(2) {
            assert!(w[0].1 < w[1].1, "closes strictly increase: {:?}", cycles);
            assert!(w[0].2 < w[1].2, "dues strictly increase: {:?}", cycles);
        }
        // Each due is the 17th of the month after its close (close 25 -> due next-month 17).
        for &(_open, close, due) in &cycles {
            assert!(due > close, "due after close: {close} -> {due}");
            assert_eq!(due.format("%d").to_string(), "17");
        }
    }

    /// Month-end clamping: a 31 close day in a 30-day month clamps without panicking and stays
    /// ordered.
    #[test]
    fn month_end_close_day_clamps() {
        let cycles = derive_card_cycles(d(2026, 6, 15), 31, 5, 3);
        for w in cycles.windows(2) {
            assert!(w[0].1 < w[1].1, "closes increase: {:?}", cycles);
        }
    }

    /// A due day past a short month's end must not clamp the prior statement's due date onto the
    /// CURRENT statement's due date and duplicate the payment. Close 28 / due 29, today
    /// 2026-03-01: the Feb-28 statement's "29th" clamps forward to Mar 29 — exactly where the
    /// Mar-28 statement is also due — so the step-back must NOT fire (it would emit two Mar-29
    /// cycles). The current Mar-28 statement leads, and no two cycles share a due date.
    #[test]
    fn short_month_due_clamp_does_not_duplicate_the_payment() {
        let cycles = derive_card_cycles(d(2026, 3, 1), 28, 29, 3);
        assert_eq!(cycles[0], (d(2026, 2, 28), d(2026, 3, 28), d(2026, 3, 29)));
        for w in cycles.windows(2) {
            assert!(
                w[0].2 < w[1].2,
                "due dates strictly increase (no duplicate): {cycles:?}"
            );
        }
    }
}
