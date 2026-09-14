//! Shared event collection: expands income sources, recurring bills, manual
//! entries, recurring debt payments, and transfer legs into the engine's dated
//! event stream, plus the calendar/read helpers they share (moved verbatim
//! from `forecast.rs`).

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDate};
use core_money::{Currency, Money};
use forecast_engine::{AssumptionBasis, DailyBalance, EventKind, ForecastEvent, Tz};
use pay_schedule::{Frequency, PaySchedule};
use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

use super::aggregate::{ForecastDayView, ForecastEventView};
use super::{parse_date, parse_frequency};
use crate::forecast_overrides::EntityOverride;
use crate::{currency_from_code, DbError};

/// Enrich the pure engine's daily series with each event's source-entity display
/// name (shared by the aggregate and per-account paths).
pub(super) fn to_day_views(
    series: Vec<DailyBalance>,
    names: &HashMap<Uuid, String>,
) -> Vec<ForecastDayView> {
    series
        .into_iter()
        .map(|day| ForecastDayView {
            date: day.date,
            closing: day.closing,
            events: day
                .events
                .into_iter()
                .map(|ev| ForecastEventView {
                    source_event_id: ev.source_event_id,
                    name: names.get(&ev.source_event_id).cloned().unwrap_or_default(),
                    kind: ev.kind.source_type_token().to_owned(),
                    amount: ev.amount,
                    assumption_basis: ev.assumption_basis,
                })
                .collect(),
        })
        .collect()
}

/// The IANA household timezone (defaults to `UTC` in a fresh vault).
pub(crate) fn read_household_tz(conn: &Connection) -> Result<Tz, DbError> {
    let tz_str: String = conn.query_row(
        "SELECT household_timezone FROM vault_metadata WHERE singleton = 1",
        [],
        |r| r.get(0),
    )?;
    tz_str
        .parse::<Tz>()
        .map_err(|e| DbError::InvalidCommand(format!("invalid household timezone {tz_str:?}: {e}")))
}

/// Set the household timezone (`personal-cfo-q329`), validating it is a real IANA name
/// first — `read_household_tz` and every caller downstream of it (the whole Future Cash
/// forecast surface, ADR 0021) fails loudly on a bad value, so this is the one gate that
/// keeps a typo or a stale/removed zone name out of the vault in the first place. A plain
/// `UPDATE` against `vault_metadata`, like the `schema_version` write in
/// `ensure_vault_metadata` — this is configuration on the singleton row, not a ledger
/// mutation, so it bypasses `WriteCommand`/the operation log the same way `set_setting`
/// bypasses it for app-level settings (`household_timezone` is a `vault_metadata` column,
/// not a `settings` row, so it does not go through `set_setting` itself).
pub(crate) fn write_household_tz(conn: &Connection, tz: &str) -> Result<(), DbError> {
    tz.parse::<Tz>()
        .map_err(|e| DbError::InvalidCommand(format!("invalid household timezone {tz:?}: {e}")))?;
    let updated = conn.execute(
        "UPDATE vault_metadata SET household_timezone = ?1 WHERE singleton = 1",
        [tz],
    )?;
    debug_assert_eq!(updated, 1, "vault_metadata is a singleton row");
    Ok(())
}

/// Household-local "today" (ADR 0021 §1: the household timezone is authoritative for
/// calendar boundaries such as "today" — never UTC, never the machine's local zone).
/// Reads the clock once at this impure boundary (ADR 0021 §7/§3: an impure caller reads
/// the instant, a pure function resolves it — mirrors [`forecast_engine::Horizon`]'s
/// `as_of`) and resolves it through [`household_today_at`] (`personal-cfo-5ie.11`).
pub(crate) fn household_today(conn: &Connection) -> Result<NaiveDate, DbError> {
    household_today_at(conn, chrono::Utc::now())
}

/// The household-local calendar date `as_of` an explicit instant falls on. Split out from
/// [`household_today`] so it is testable against a fixed instant rather than the real
/// clock (`personal-cfo-5ie.11` requires pinning behavior "at 23:30 local").
pub(crate) fn household_today_at(
    conn: &Connection,
    as_of: chrono::DateTime<chrono::Utc>,
) -> Result<NaiveDate, DbError> {
    Ok(as_of.with_timezone(&read_household_tz(conn)?).date_naive())
}

/// The vault's base/reporting-currency setting parsed to a [`Currency`], or
/// `None` when it is unset or unrecognized. Mirrors the IPC `reporting_currency`
/// key (personal-cfo-4d8.1); the forecast falls back to it when no liquid account
/// pins a currency, instead of hardcoding USD (personal-cfo-4n3x). Shared with the
/// cash-tier rollups for the same empty-vault currency (ADR 0028).
pub(crate) fn reporting_currency(conn: &Connection) -> Result<Option<Currency>, DbError> {
    let code: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'reporting_currency'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(code.as_deref().and_then(|c| currency_from_code(c).ok()))
}

/// The opening liquid-cash balance and the currency the forecast runs in.
///
/// The forecast is single-currency: if liquid accounts span more than one
/// currency it errors (cross-currency forecasting is a later layer). With no
/// liquid accounts it falls back to the vault's base-currency setting, and only
/// then to USD — so setting the base currency drives an empty-vault forecast.
pub(super) fn liquid_starting_balance(conn: &Connection) -> Result<(Currency, Money), DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, currency, ledger_account_id FROM accounts
          WHERE cashflow_role = 'liquid_cash' AND active = 1",
    )?;
    let accounts: Vec<(Uuid, String, Uuid)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Uuid>(2)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    let mut currency_code: Option<String> = None;
    for (_, code, _) in &accounts {
        match &currency_code {
            None => currency_code = Some(code.clone()),
            Some(existing) if existing != code => {
                return Err(DbError::InvalidCommand(
                    "Future Cash forecast does not support mixed-currency liquid accounts yet"
                        .to_owned(),
                ));
            }
            Some(_) => {}
        }
    }
    let currency = match &currency_code {
        Some(code) => currency_from_code(code)?,
        None => reporting_currency(conn)?.unwrap_or(Currency::Usd),
    };

    // Assertion-anchored per account (ADR 0027): the additive balance, not a raw
    // posting sum — so a manual balance assertion drives the forecast even with no
    // transactions recorded.
    let mut total: i64 = 0;
    for (account_id, _, ledger_account_id) in &accounts {
        let balance = crate::assertion_anchored_balance(conn, *account_id, *ledger_account_id)?;
        total = total
            .checked_add(balance)
            .ok_or_else(|| DbError::InvalidCommand("liquid balance overflow".to_owned()))?;
    }

    Ok((currency, Money::new(total, currency)))
}

/// Expand every active income source over `[start, end]` into `Income` events.
pub(super) fn collect_income_events(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    overrides: &HashMap<Uuid, EntityOverride>,
    events: &mut Vec<ForecastEvent>,
    names: &mut HashMap<Uuid, String>,
) -> Result<(), DbError> {
    // Linked income instances suppress their projection exactly like bills
    // (ADR 0026 addendum, xtz5): an early-arriving synced paycheck is in the
    // starting balance and must not project again.
    let linked = linked_instance_dates(conn, start)?;
    for source in crate::schedule_sources::active_income_schedules(conn)? {
        let crate::schedule_sources::ScheduleSource {
            id,
            name,
            amount_minor: net_minor,
            currency_code: code,
            freq_token,
            anchor,
            ..
        } = source;
        let over = overrides.get(&id);
        if over.is_some_and(EntityOverride::fully_excluded) {
            continue;
        }
        let currency = currency_from_code(&code)?;
        let frequency = parse_frequency(&freq_token, "income")?;
        let anchor_str = anchor.ok_or_else(|| {
            DbError::InvalidCommand("income source is missing its anchor date".to_owned())
        })?;
        let base_anchor = parse_date(&anchor_str)?;
        let anchor = over.map_or(base_anchor, |o| o.anchor_or(base_anchor));
        let linked_dates = linked.get(&id);
        for date in PaySchedule::new(frequency, anchor).pay_dates(start, end) {
            if linked_dates.is_some_and(|dates| dates.contains(&date)) {
                continue; // realized already — the money is in the balance
            }
            let Some(net) = over.map_or(Some(net_minor), |o| o.amount_for(date, net_minor)) else {
                continue; // excluded on this date
            };
            events.push(ForecastEvent {
                occurs_on: date,
                kind: EventKind::Income,
                amount: Money::new(net, currency), // inflow (positive)
                source_event_id: id,
                assumption_basis: AssumptionBasis::RecurringSchedule { frequency },
            });
        }
        names.insert(id, name);
    }
    Ok(())
}

/// Expand every active, in-forecast recurring obligation over `[start, end]`
/// into outflow events. The expected amount is stored positive and negated here.
pub(super) fn collect_bill_events(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    overrides: &HashMap<Uuid, EntityOverride>,
    events: &mut Vec<ForecastEvent>,
    names: &mut HashMap<Uuid, String>,
) -> Result<(), DbError> {
    // A bill charged to a credit card with a derivable billing cycle has its liquid impact
    // modeled once as the card's per-cycle PAYMENT (`collect_card_payment_events`, ADR 0039 §2 /
    // 6wk.10), so it emits no per-charge liquid outflow here — it is suppressed. Every other bill
    // (including one on a card without a cycle) keeps its charge-date outflow, so cash never
    // silently vanishes (ADR 0039 §3). Built once per forecast.
    let card_charged = card_charged_bill_ids(conn)?;
    // Occurrences the user has already confirmed paid (personal-cfo-5ie.9): the real ledger
    // transaction is baked into the starting balance, so we must not project the occurrence
    // again. Matched per event to a projected date (below), so a confirm made further ahead than
    // the instance-linking window — or a due-date edit after the confirm — still suppresses
    // exactly one occurrence (no double-count).
    let confirmed = confirmed_obligation_dates(conn)?;
    // Linked instances suppress by EXACT scheduled date, and only for
    // still-projected occurrences (ADR 0026 addendum, xtz5, tightened after
    // adversarial review): instances and the forecast expand the same
    // PaySchedule grid, so no drift tolerance is needed — and reusing the
    // ±7-day nearest-match here silently ate NEXT week's occurrence for any
    // weekly cadence with linked history. A past linked occurrence has
    // nothing to suppress: its money and its projection are both already
    // behind the starting balance.
    let linked = linked_instance_dates(conn, start)?;
    for source in crate::schedule_sources::active_obligation_schedules(conn)? {
        let crate::schedule_sources::ScheduleSource {
            id,
            name,
            amount_minor,
            currency_code: code,
            freq_token,
            anchor: anchor_opt,
            bill_type,
            ..
        } = source;
        let over = overrides.get(&id);
        if over.is_some_and(EntityOverride::fully_excluded) {
            continue;
        }
        // Suppressed: the card payment carries this bill's liquid impact.
        if card_charged.contains(&id) {
            names.insert(id, name);
            continue;
        }
        let currency = currency_from_code(&code)?;
        let frequency = parse_frequency(&freq_token, "bill")?;
        let anchor_str = anchor_opt.ok_or_else(|| {
            DbError::InvalidCommand("recurring obligation is missing its anchor date".to_owned())
        })?;
        let base_anchor = parse_date(&anchor_str)?;
        let anchor = over.map_or(base_anchor, |o| o.anchor_or(base_anchor));
        let kind = match bill_type.as_deref() {
            Some("loan_payment") => EventKind::LoanPayment,
            _ => EventKind::RecurringBill,
        };
        let charge_dates = PaySchedule::new(frequency, anchor).pay_dates(start, end);
        // Match each confirmed occurrence to its nearest projected date within tolerance,
        // consuming that date once — so a confirmed occurrence suppresses exactly one projection
        // even if a due-date edit shifted it after the confirm.
        let mut suppressed: HashSet<NaiveDate> = HashSet::new();
        let linked_dates = linked.get(&id);
        if let Some(dates) = linked_dates {
            for date in &charge_dates {
                if dates.contains(date) {
                    suppressed.insert(*date);
                }
            }
        }
        for confirmed_on in confirmed.get(&id).into_iter().flatten() {
            // The same real payment can be BOTH confirmed (5ie.9) and
            // seam-linked; if a linked date already covers this confirm
            // (within the drift tolerance), it must not consume a second
            // projected occurrence.
            if linked_dates.is_some_and(|dates| {
                dates
                    .iter()
                    .any(|d| (*d - *confirmed_on).num_days().abs() <= SUPPRESSION_TOLERANCE_DAYS)
            }) {
                continue;
            }
            if let Some(best) = charge_dates
                .iter()
                .filter(|d| !suppressed.contains(d))
                .min_by_key(|d| (**d - *confirmed_on).num_days().abs())
                .copied()
                .filter(|d| (*d - *confirmed_on).num_days().abs() <= SUPPRESSION_TOLERANCE_DAYS)
            {
                suppressed.insert(best);
            }
        }
        for charge_date in charge_dates {
            // Already paid early — its real transaction is in the balance; don't double-count.
            if suppressed.contains(&charge_date) {
                continue;
            }
            let Some(expected) = over.map_or(Some(amount_minor), |o| {
                o.amount_for(charge_date, amount_minor)
            }) else {
                continue; // excluded on this date
            };
            let outflow = expected.checked_neg().ok_or_else(|| {
                DbError::InvalidCommand("recurring bill amount overflow".to_owned())
            })?;
            events.push(ForecastEvent {
                occurs_on: charge_date,
                kind,
                amount: Money::new(outflow, currency),
                source_event_id: id,
                assumption_basis: AssumptionBasis::RecurringSchedule { frequency },
            });
        }
        names.insert(id, name);
    }
    Ok(())
}

/// The recurring bills whose liquid impact is superseded by a card **payment** — i.e. their
/// `autopay_account_id` is a `credit_facility` account that projects per-cycle payments: either
/// a derivable billing cycle (both a statement close + payment due day), or — ADR 0039 addendum
/// 2026-07-10 §3 — a cycle-less card whose due-day pseudo-cycles fold its forecast-included
/// charged bills into the payment (personal-cfo-4d8.23.10). Any repayment philosophy qualifies
/// (llx5 models revolving interest, so a partial-pay card projects a real per-cycle payment too
/// — ADR 0039 §2 / 6wk.10). These bills emit no per-charge liquid outflow (`collect_bill_events`
/// suppresses them; the card `payment` carries them). A bill on a card with neither path is NOT
/// here and keeps its charge-date outflow. Returns `recurring_events.id`s.
pub(super) fn card_charged_bill_ids(conn: &Connection) -> Result<HashSet<Uuid>, DbError> {
    let mut stmt = conn.prepare(
        // Must match the set of cards that emit a payment in `collect_card_payment_events`:
        // `read_cards_with_cycle` (close + due day) plus the pseudo-cycle cards (due day only,
        // with at least one forecast-included charged bill — `has_forecast_charged_bills`).
        // `a.active = 1` included — else an inactive card would suppress its bill with no payment
        // to replace it, and the money would vanish (6wk.10 review). For a cycle-less card the
        // per-bill forecast-included criteria are checked on the CARD (does any qualifying bill
        // exist -> the card takes the pseudo path), mirroring the collector's gate.
        "SELECT e.id
         FROM recurring_events e
         JOIN accounts a ON a.id = e.autopay_account_id
         JOIN debt_terms dt ON dt.account_id = a.id
         WHERE a.cashflow_role = 'credit_facility'
           AND a.active = 1
           AND dt.payment_due_day IS NOT NULL
           AND (dt.statement_close_day IS NOT NULL
                OR EXISTS (
                    SELECT 1 FROM recurring_events q
                     WHERE q.autopay_account_id = a.id AND q.is_active = 1
                       AND q.include_in_forecast = 1 AND q.next_expected_date IS NOT NULL))",
    )?;
    let ids = stmt
        .query_map([], |r| r.get::<_, Uuid>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(ids)
}

/// How far a confirmed occurrence's stored due date may drift from a projected occurrence and
/// still be matched to it (personal-cfo-5ie.9). Covers a bill whose due date was edited (or
/// scenario-overridden) by up to a week after it was confirmed, without ever reaching a distinct
/// occurrence of a monthly-or-less-frequent bill. Shares the single instance-linking window
/// constant (personal-cfo-4d8.24.4) so the two tolerances can never diverge.
const SUPPRESSION_TOLERANCE_DAYS: i64 = crate::recurring_instances::RECURRING_MATCH_WINDOW_DAYS;

/// The confirmed-paid occurrences per recurring event (personal-cfo-5ie.9), as their stored due
/// dates. The forecast suppresses these because the real transaction already satisfied them.
/// Grouped by event so each confirmed occurrence is matched to a projected date by nearest
/// (within [`SUPPRESSION_TOLERANCE_DAYS`]) and consumed once — correct even when a due-date edit
/// shifts the projected date after the confirm, and regardless of how far ahead it was confirmed.
/// Still-projected scheduled dates of instances the seam linked to a real
/// transaction (ADR 0026 §9), per schedule (bills AND income): exact-date
/// suppression peers of explicit confirms.
fn linked_instance_dates(
    conn: &Connection,
    start: NaiveDate,
) -> Result<HashMap<Uuid, HashSet<NaiveDate>>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT recurring_event_id, scheduled_date FROM recurring_event_instances
         WHERE linked_transaction_id IS NOT NULL AND scheduled_date >= ?1",
    )?;
    let rows = stmt.query_map([start.to_string()], |r| {
        Ok((r.get::<_, Uuid>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut map: HashMap<Uuid, HashSet<NaiveDate>> = HashMap::new();
    for row in rows {
        let (id, date) = row?;
        map.entry(id).or_default().insert(parse_date(&date)?);
    }
    Ok(map)
}

fn confirmed_obligation_dates(conn: &Connection) -> Result<HashMap<Uuid, Vec<NaiveDate>>, DbError> {
    let mut stmt =
        conn.prepare("SELECT recurring_event_id, scheduled_date FROM confirmed_obligations")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, Uuid>(0)?, r.get::<_, String>(1)?)))?;
    let mut map: HashMap<Uuid, Vec<NaiveDate>> = HashMap::new();
    for row in rows {
        let (id, date) = row?;
        map.entry(id).or_default().push(parse_date(&date)?);
    }
    Ok(map)
}

/// The last calendar day-of-month of `(year, month)`.
pub(super) fn last_day_of_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(ny, nm, 1)
        .and_then(|d| d.pred_opt())
        .map_or(28, |d| d.day())
}

/// `day` placed in `(year, month)`, month-end clamped into `1..=last_day`.
pub(super) fn clamped_day(year: i32, month: u32, day: u32) -> NaiveDate {
    let clamped = day.clamp(1, last_day_of_month(year, month));
    NaiveDate::from_ymd_opt(year, month, clamped).expect("clamped day is a valid date")
}

pub(super) fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

pub(super) fn prev_month(year: i32, month: u32) -> (i32, u32) {
    if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    }
}

/// First date `>= from` whose day-of-month is `day` (month-end clamped).
pub(super) fn day_of_month_on_or_after(from: NaiveDate, day: u32) -> NaiveDate {
    let candidate = clamped_day(from.year(), from.month(), day);
    if candidate >= from {
        candidate
    } else {
        let (y, m) = next_month(from.year(), from.month());
        clamped_day(y, m, day)
    }
}

/// First date `> after` whose day-of-month is `day` (month-end clamped).
pub(super) fn day_of_month_after(after: NaiveDate, day: u32) -> NaiveDate {
    let candidate = clamped_day(after.year(), after.month(), day);
    if candidate > after {
        candidate
    } else {
        let (y, m) = next_month(after.year(), after.month());
        clamped_day(y, m, day)
    }
}

/// Fold the active base manual future entries (personal-cfo-q6gh) whose date falls
/// in `[start, end]` into the event stream as `ManualOneOff` events.
pub(super) fn collect_manual_events(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    scenarios: &[Uuid],
    events: &mut Vec<ForecastEvent>,
    names: &mut HashMap<Uuid, String>,
) -> Result<(), DbError> {
    for entry in crate::manual_entry::active_manual_entries(conn, scenarios)? {
        if entry.occurs_on < start || entry.occurs_on > end {
            continue;
        }
        // Matched to a real transaction (ADR 0026 addendum, xtz5): the flow
        // is in the balance already — the entry no longer projects.
        if entry.matched_transaction_id.is_some() {
            continue;
        }
        events.push(ForecastEvent {
            occurs_on: entry.occurs_on,
            kind: EventKind::ManualOneOff,
            amount: entry.amount,
            source_event_id: entry.id,
            assumption_basis: AssumptionBasis::ManualOneOff,
        });
        names.insert(entry.id, entry.label);
    }
    Ok(())
}

/// Fold the active recurring extra-debt-payment overlays (personal-cfo-6wk.19) into the event
/// stream: each expands to a monthly liquid **outflow** (`−amount`, ADR 0035 §3) on its anchor's
/// day-of-month over `[start, min(end, end_date)]`. Scenario-scoped, so the base forecast (no
/// scenario) is unaffected unless a base-level overlay exists.
pub(super) fn collect_recurring_debt_payment_events(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    currency: Currency,
    scenarios: &[Uuid],
    events: &mut Vec<ForecastEvent>,
    names: &mut HashMap<Uuid, String>,
) -> Result<(), DbError> {
    for payment in crate::recurring_debt::active_recurring_debt_payments(conn, scenarios)? {
        // The forecast folds one currency; skip an overlay in another (no offline FX), mirroring
        // the loan-payment collector, so a currency-mismatched overlay can't break the projection.
        if payment.amount.currency() != currency {
            continue;
        }
        let outflow = payment.amount.checked_neg()?;
        let last = payment.end_date.map_or(end, |e| e.min(end));
        for date in PaySchedule::new(Frequency::Monthly, payment.anchor_date).pay_dates(start, last)
        {
            events.push(ForecastEvent {
                occurs_on: date,
                kind: EventKind::LoanPayment,
                amount: outflow,
                source_event_id: payment.id,
                assumption_basis: AssumptionBasis::RecurringSchedule {
                    frequency: Frequency::Monthly,
                },
            });
        }
        names.insert(payment.id, payment.label);
    }
    Ok(())
}

/// Project the SOURCE (liquid) leg of a recurring transfer whose destination is an investment
/// account — a DCA / automatic contribution (personal-cfo-9h0.1) — as a real recurring OUTFLOW in
/// the aggregate liquid forecast. Unlike a liquid↔liquid transfer (which nets to zero and is
/// omitted from the aggregate), money genuinely leaves the liquid pool for a non-liquid investment.
/// The destination (investment) leg is not projected here — it isn't liquid cash; it raises net
/// worth via the realized ledger posting. The per-account path handles both legs in
/// [`collect_transfer_legs`] (the investment leg is dropped there, since only liquid accounts form a
/// series), so the aggregate and per-account series reconcile (ADR 0035 §3 / ADR 0026 §14).
pub(super) fn collect_recurring_transfer_investment_outflows(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    currency: Currency,
    events: &mut Vec<ForecastEvent>,
    names: &mut HashMap<Uuid, String>,
) -> Result<(), DbError> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.amount_minor, t.currency, t.frequency, t.anchor_date, sa.name, da.name
         FROM recurring_transfers t
         JOIN accounts sa ON sa.id = t.source_account_id
         JOIN accounts da ON da.id = t.dest_account_id
         WHERE sa.cashflow_role = 'liquid_cash' AND da.cashflow_role = 'investment_asset'
         ORDER BY t.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (id, amount_minor, code, freq_token, anchor_str, source_name, dest_name) in rows {
        // The forecast folds one currency; skip a contribution in another (no offline FX),
        // mirroring the loan/debt collectors.
        if currency_from_code(&code)? != currency {
            continue;
        }
        let frequency = parse_frequency(&freq_token, "transfer")?;
        let anchor = parse_date(&anchor_str)?;
        let outflow = Money::new(amount_minor, currency).checked_neg()?;
        let basis = AssumptionBasis::RecurringSchedule { frequency };
        for date in PaySchedule::new(frequency, anchor).pay_dates(start, end) {
            events.push(ForecastEvent {
                occurs_on: date,
                kind: EventKind::Transfer,
                amount: outflow,
                source_event_id: id,
                assumption_basis: basis,
            });
        }
        names.insert(id, format!("Contribution: {source_name} → {dest_name}"));
    }
    Ok(())
}

/// Inject recurring-transfer legs straight into the per-account `partitions` (ADR
/// 0026 §14, personal-cfo-npoe). Each occurrence emits `−amount` to the source
/// account and `+amount` to the destination. The aggregate forecast omits
/// transfers (they net to zero), so this lives only in the per-account path; the
/// two legs cancel across the series, preserving the §12 reconciliation invariant.
pub(super) fn collect_transfer_legs(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    partitions: &mut HashMap<Option<Uuid>, Vec<ForecastEvent>>,
    names: &mut HashMap<Uuid, String>,
) -> Result<(), DbError> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.source_account_id, t.dest_account_id, t.amount_minor,
                t.currency, t.frequency, t.anchor_date, sa.name, da.name
         FROM recurring_transfers t
         JOIN accounts sa ON sa.id = t.source_account_id
         JOIN accounts da ON da.id = t.dest_account_id
         ORDER BY t.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, Uuid>(1)?,
                r.get::<_, Uuid>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (id, source, dest, amount_minor, code, freq_token, anchor_str, source_name, dest_name) in
        rows
    {
        let currency = currency_from_code(&code)?;
        let frequency = parse_frequency(&freq_token, "transfer")?;
        let anchor = parse_date(&anchor_str)?;
        let amount = Money::new(amount_minor, currency);
        let outflow = amount.checked_neg()?;
        let basis = AssumptionBasis::RecurringSchedule { frequency };
        for date in PaySchedule::new(frequency, anchor).pay_dates(start, end) {
            partitions
                .entry(Some(source))
                .or_default()
                .push(ForecastEvent {
                    occurs_on: date,
                    kind: EventKind::Transfer,
                    amount: outflow,
                    source_event_id: id,
                    assumption_basis: basis,
                });
            partitions
                .entry(Some(dest))
                .or_default()
                .push(ForecastEvent {
                    occurs_on: date,
                    kind: EventKind::Transfer,
                    amount,
                    source_event_id: id,
                    assumption_basis: basis,
                });
        }
        names.insert(id, format!("Transfer: {source_name} → {dest_name}"));
    }
    Ok(())
}
