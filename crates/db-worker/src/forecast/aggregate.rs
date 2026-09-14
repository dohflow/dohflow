//! Aggregate Future Cash forecast: the engine adapter that expands vault
//! schedules into events, folds them with [`forecast_layer1`], and applies the
//! Layer-2 variable-spend widening (moved verbatim from `forecast.rs`).

use std::collections::{HashMap, HashSet};

use categorization::spend_classifier::{classify, CategoryProfile, SpendClass};
use chrono::{DateTime, Datelike, Months, NaiveDate, Utc};
use core_money::{Currency, Money};
use forecast_engine::layer2::{
    widen_with_spend_adjusted, SpendAdjustment, SpendModel, SpendObservation,
};
use forecast_engine::{forecast_layer1, AssumptionBasis, Band, DailyBalance, Horizon};
use rusqlite::{params, Connection};
use uuid::Uuid;

use super::card_cycles::{collect_card_payment_events, collect_loan_payment_events};
use super::events::{
    collect_bill_events, collect_income_events, collect_manual_events,
    collect_recurring_debt_payment_events, collect_recurring_transfer_investment_outflows,
    liquid_starting_balance, read_household_tz, to_day_views,
};
use crate::forecast_overrides::entity_overrides;
use crate::DbError;

/// One projected cash event on a forecast day, enriched with the source
/// entity's display name (which the pure engine does not carry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForecastEventView {
    /// The source entity id (income source / recurring event) this came from.
    pub source_event_id: Uuid,
    /// The source entity's display name (e.g. `"Rent"`, `"Acme Corp"`).
    pub name: String,
    /// The `forecast_rows.source_type` token (`income` / `recurring_bill` /
    /// `loan_payment` / `transfer` / `manual_entry`).
    pub kind: String,
    /// Signed amount applied to the running balance (inflow +, outflow −).
    pub amount: Money,
    /// Why this event is assumed (provenance for row explanation, ADR 0026 §1).
    pub assumption_basis: AssumptionBasis,
}

/// One day of the projected series: the closing balance and the events that
/// moved it. Emitted for every day in the horizon (empty `events` on quiet days).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForecastDayView {
    /// The household-local calendar date.
    pub date: NaiveDate,
    /// Projected liquid-cash balance at end of day, as a P10/P50/P90 band
    /// (collapsed for the deterministic Layer-1 engine).
    pub closing: Band,
    /// The events applied on this day, in canonical same-day order.
    pub events: Vec<ForecastEventView>,
}

/// The Future Cash forecast: the opening liquid balance plus the per-day series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForecastView {
    /// The single currency the forecast is computed in (from the liquid accounts).
    pub currency: Currency,
    /// Liquid-cash balance at the start of the horizon ("today").
    pub starting_balance: Money,
    /// The first projected day (household-local "today").
    pub start_date: NaiveDate,
    /// Number of days projected (the requested horizon).
    pub horizon_days: u32,
    /// One row per calendar day in the horizon, ascending.
    pub days: Vec<ForecastDayView>,
}

/// Compute the Future Cash forecast as of `as_of` over `horizon_days`.
///
/// `as_of` is supplied by the caller (the clock read happens there) so this
/// function is a deterministic pure-ish reader: same vault state + same `as_of`
/// → same `ForecastView`.
///
/// # Errors
/// Returns [`DbError`] on a read failure, a malformed stored schedule, mixed
/// liquid-account currencies (unsupported in Layer 1), or a forecast arithmetic
/// failure (currency mismatch / overflow) surfaced by the engine.
pub(crate) fn compute(
    conn: &Connection,
    as_of: DateTime<Utc>,
    horizon_days: u32,
    scenarios: &[Uuid],
) -> Result<ForecastView, DbError> {
    let tz = read_household_tz(conn)?;
    let (currency, starting_balance) = liquid_starting_balance(conn)?;
    let horizon = Horizon::new(as_of, horizon_days);
    let start_date = as_of.with_timezone(&tz).date_naive();

    let Some((start, end)) = horizon.window(tz) else {
        // Zero-day horizon: nothing to project.
        return Ok(ForecastView {
            currency,
            starting_balance,
            start_date,
            horizon_days,
            days: Vec::new(),
        });
    };

    // An archived, expired, or deleted scenario must not reach the run (ADR 0051).
    // Expired/archived selections drop out but the survivors keep their order — order
    // is the precedence (ADR 0059 §1).
    let scenarios = &crate::scenarios::effective_scenarios(conn, scenarios, start_date)?[..];
    let overrides = entity_overrides(conn, scenarios)?;
    let mut events = Vec::new();
    let mut names: HashMap<Uuid, String> = HashMap::new();
    collect_income_events(conn, start, end, &overrides, &mut events, &mut names)?;
    collect_bill_events(conn, start, end, &overrides, &mut events, &mut names)?;
    collect_loan_payment_events(conn, start, end, currency, &mut events, &mut names)?;
    collect_card_payment_events(
        conn,
        start,
        end,
        currency,
        &overrides,
        &mut events,
        &mut names,
    )?;
    collect_manual_events(conn, start, end, scenarios, &mut events, &mut names)?;
    collect_recurring_debt_payment_events(
        conn,
        start,
        end,
        currency,
        scenarios,
        &mut events,
        &mut names,
    )?;
    // Asymmetric transfers to investments (DCA, 9h0.1): the liquid source leg is a real outflow in
    // the aggregate. (Liquid↔liquid transfers net to zero and stay omitted; the per-account path
    // carries both legs of all transfers.)
    collect_recurring_transfer_investment_outflows(
        conn,
        start,
        end,
        currency,
        &mut events,
        &mut names,
    )?;

    let series = forecast_layer1(&events, starting_balance, horizon, tz)
        .map_err(|e| DbError::InvalidCommand(format!("future cash forecast failed: {e}")))?;

    // Layer 2 (ADR 0026 §7-8, personal-cfo-9h1s): once the household has enough variable-
    // spend history, widen the deterministic line into a band learned from that spend.
    // Planned per-category spend changes (base + the selected scenario) shift the
    // modelled draw (personal-cfo-4d8.27.6.2).
    let adjustments = crate::forecast_overrides::category_spend_adjustments(conn, scenarios)?;
    let series = apply_layer2_spend(conn, series, start, &adjustments)?;

    Ok(ForecastView {
        currency,
        starting_balance,
        start_date,
        horizon_days,
        days: to_day_views(series, &names),
    })
}

/// How many distinct months of variable-spend history a household must have before the
/// Layer-2 band activates — enough to fit a meaningful seasonal model. Below this the
/// deterministic line is all that shows (ADR 0026 §8; the full readiness-score gate that
/// also weighs categorization % + backtest error is personal-cfo-nxgx).
pub(super) const LAYER2_MIN_HISTORY_MONTHS: usize = 6;
/// How far back to learn variable spend from.
pub(super) const LAYER2_HISTORY_WINDOW_MONTHS: u32 = 24;
/// A category needs at least this many postings in the window before the ordinary/
/// extraordinary classifier judges its outliers — below it, thin evidence is kept whole
/// rather than risk excluding ordinary spend (ADR 0038 §2, personal-cfo-pezm.1).
const MIN_SAMPLES_FOR_CLASSIFICATION: usize = 4;

/// Widen a Layer-1 series with learned variable spend, gated on history sufficiency. Below
/// the gate the series passes through unchanged (the trustworthy deterministic line).
fn apply_layer2_spend(
    conn: &Connection,
    series: Vec<DailyBalance>,
    start: NaiveDate,
    adjustments: &[SpendAdjustment],
) -> Result<Vec<DailyBalance>, DbError> {
    let window_start = start
        .checked_sub_months(Months::new(LAYER2_HISTORY_WINDOW_MONTHS))
        .unwrap_or(start);
    let history = read_variable_spend_history(conn, window_start, start, None)?;

    if distinct_spend_months(&history) < LAYER2_MIN_HISTORY_MONTHS {
        return Ok(series); // not enough data earned → keep the deterministic line
    }

    let model = SpendModel::fit(&history);
    Ok(widen_with_spend_adjusted(&series, &model, adjustments))
}

/// Distinct calendar months present in a variable-spend history — the maturity signal the
/// Layer-2 band gates on ([`apply_layer2_spend`], liquid-only). The readiness "spending
/// history" indicator no longer shares this: it counts card spend too via
/// [`distinct_variable_spend_months_all_accounts`], so the band gate and the readiness
/// indicator diverge for card-heavy vaults until the band re-model consumes card spend
/// (ADR 0050 / personal-cfo-4d8.27.1.2).
pub(super) fn distinct_spend_months(history: &[SpendObservation]) -> usize {
    history
        .iter()
        .map(|obs| (obs.date.year(), obs.date.month()))
        .collect::<HashSet<(i32, u32)>>()
        .len()
}

/// Distinct calendar months of categorized **variable** spend across **all** accounts
/// (liquid + credit) over `[start, end)`. The readiness "spending history" indicator uses
/// this so a card-based household — whose discretionary spend sits on credit cards — is
/// credited for the history it has recorded (personal-cfo-4d8.27.1.2), instead of reading 0
/// because [`read_variable_spend_history`]'s aggregate path is deliberately liquid-only. This
/// is the interim indicator half of ADR 0050: until the band re-model injects card variance
/// at the card payment date, the indicator counts card spend while the Layer-2 band
/// aggregation stays liquid-only.
pub(super) fn distinct_variable_spend_months_all_accounts(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<usize, DbError> {
    let months: i64 = conn.query_row(
        // Join through to `accounts` (like the sibling spend queries) so only postings on real
        // user accounts count — a categorized money-in transaction (e.g. a refund) posts its
        // negative leg on a SYSTEM counter-account with no `accounts` row, which this join drops.
        // The only difference from `read_variable_spend_history` is the absence of the
        // `cashflow_role = 'liquid_cash'` filter, so credit-card spend is included.
        "SELECT COUNT(DISTINCT substr(lt.occurred_at, 1, 7))
         FROM ledger_postings lp
         JOIN ledger_transactions lt ON lt.id = lp.transaction_id
         JOIN ledger_accounts la ON la.id = lp.ledger_account_id
         JOIN accounts a ON a.ledger_account_id = la.id
         JOIN transaction_categorizations tc ON tc.transaction_id = lt.id
         JOIN categories c ON c.id = tc.category_id
         WHERE lt.voided_at IS NULL
           AND lp.minor_units < 0
           AND c.forecast_behavior IN ('variable_regular', 'variable_lumpy')
           AND substr(lt.occurred_at, 1, 10) >= ?1
           AND substr(lt.occurred_at, 1, 10) < ?2",
        params![start.to_string(), end.to_string()],
        |r| r.get(0),
    )?;
    Ok(usize::try_from(months).unwrap_or(0))
}

/// Read the household's categorized **variable** spending over `[start, end)` as Layer-2
/// observations, with **extraordinary** (one-off) postings excluded so the band models
/// ordinary behaviour rather than a vacation that will not repeat (ADR 0038,
/// personal-cfo-pezm.1). Only `variable_*` categories — `deterministic` (fixed recurring)
/// spend is already folded by Layer-1, so excluding it avoids double-counting
/// (personal-cfo-9h1s).
pub(crate) fn read_variable_spend_history(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    account: Option<Uuid>,
) -> Result<Vec<SpendObservation>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT lt.occurred_at, tc.category_id, lp.minor_units, c.forecast_behavior
         FROM ledger_postings lp
         JOIN ledger_transactions lt ON lt.id = lp.transaction_id
         JOIN ledger_accounts la ON la.id = lp.ledger_account_id
         JOIN accounts a ON a.ledger_account_id = la.id
         JOIN transaction_categorizations tc ON tc.transaction_id = lt.id
         JOIN categories c ON c.id = tc.category_id
         WHERE lt.voided_at IS NULL
           AND lp.minor_units < 0
           AND c.forecast_behavior IN ('variable_regular', 'variable_lumpy')
           AND substr(lt.occurred_at, 1, 10) >= ?1
           AND substr(lt.occurred_at, 1, 10) < ?2
           AND (?3 IS NULL OR a.id = ?3)
           -- The aggregate band (account=NULL) is LIQUID variable spend only: card variable
           -- spend flows via the card payment (ADR 0039 §2 / 6wk.10), so it must not also widen
           -- the band. The per-account call (account set, e.g. a card for llx5) is unrestricted.
           AND (?3 IS NOT NULL OR a.cashflow_role = 'liquid_cash')",
    )?;
    let rows = stmt.query_map(params![start.to_string(), end.to_string(), account], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Uuid>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    // One row per posting, tagged whether its category is a *regular* (steady) variable
    // category. Only regular categories are classified; `variable_lumpy` is the taxonomy's
    // explicit irregular-spend bucket (property tax, maintenance, …), so its large charges
    // are its signal and are kept whole (ADR 0038 §2).
    struct Posting {
        obs: SpendObservation,
        is_regular: bool,
    }
    let mut postings: Vec<Posting> = Vec::new();
    for row in rows {
        let (occurred_at, category_id, minor_units, behavior) = row?;
        let date = DateTime::parse_from_rfc3339(&occurred_at)
            .map(|dt| dt.with_timezone(&Utc).date_naive())
            .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
        postings.push(Posting {
            obs: SpendObservation {
                date,
                category: category_id.to_string(),
                amount_cents: -minor_units, // ledger debit → positive spend magnitude
            },
            is_regular: behavior == "variable_regular",
        });
    }

    // Build a robust per-category profile from the regular categories, then drop their
    // extraordinary postings so a one-off (a vacation, a big purchase) does not inflate the
    // ordinary baseline (ADR 0038 §3). The committed-ledger merchant key is not populated
    // yet, so the recurring-merchant guard gets `0` occurrences here — wiring it is
    // personal-cfo-pezm.2.
    let mut amounts_by_category: HashMap<String, Vec<i64>> = HashMap::new();
    for p in postings.iter().filter(|p| p.is_regular) {
        amounts_by_category
            .entry(p.obs.category.clone())
            .or_default()
            .push(p.obs.amount_cents);
    }
    let profiles: HashMap<String, CategoryProfile> = amounts_by_category
        .into_iter()
        .filter(|(_, amounts)| amounts.len() >= MIN_SAMPLES_FOR_CLASSIFICATION)
        .map(|(category, amounts)| (category, CategoryProfile::from_amounts(&amounts)))
        .collect();

    let history = postings
        .into_iter()
        .filter(|p| {
            // Lumpy categories are kept whole; a regular category with too few samples is
            // too thin to judge, so keep it rather than risk excluding ordinary spend.
            if !p.is_regular {
                return true;
            }
            match profiles.get(p.obs.category.as_str()) {
                None => true,
                Some(profile) => {
                    classify(p.obs.amount_cents, 0, profile).class == SpendClass::Ordinary
                }
            }
        })
        .map(|p| p.obs)
        .collect();
    Ok(history)
}

/// The categorized ordinary variable spend over `[start, end)` as `(date, category_id, amount)`
/// tuples in minor units — the input to the band-drift engine (personal-cfo-5ie.8). Liquid-only
/// (account = `None`), matching the aggregate spend band. Extraordinary/lumpy spend is already
/// dropped by [`read_variable_spend_history`] (ADR 0038).
pub(crate) fn variable_spend_points(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<(NaiveDate, String, i64)>, DbError> {
    Ok(read_variable_spend_history(conn, start, end, None)?
        .into_iter()
        .map(|o| (o.date, o.category, o.amount_cents))
        .collect())
}
