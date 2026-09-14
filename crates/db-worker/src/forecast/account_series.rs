//! Per-account / per-group projection: one forecast series per liquid account
//! plus the synthetic Unallocated bucket (moved verbatim from `forecast.rs`).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Months, NaiveDate, Utc};
use core_ledger::{AccountSubtype, CashTier};
use core_money::{Currency, Money};
use forecast_engine::layer2::{
    widen_with_lumps, widen_with_spend_adjusted, LumpInjection, SpendAdjustment, SpendModel,
};
use forecast_engine::{forecast_layer1, Band, ForecastEvent, Horizon, Tz};
use rusqlite::{params, Connection};
use uuid::Uuid;

use super::aggregate::{
    distinct_spend_months, ForecastDayView, LAYER2_HISTORY_WINDOW_MONTHS, LAYER2_MIN_HISTORY_MONTHS,
};
use super::card_cycles::{
    collect_card_lump_injections, collect_card_payment_events, collect_loan_payment_events,
    read_cards_with_cycle, read_cards_without_cycle, read_loans_with_payment,
};
use super::events::{
    collect_bill_events, collect_income_events, collect_manual_events,
    collect_recurring_debt_payment_events, collect_transfer_legs, read_household_tz,
    reporting_currency, to_day_views,
};
use super::read_variable_spend_history;
use crate::forecast_overrides::entity_overrides;
use crate::{currency_from_code, DbError};

// ===== Per-account / per-group projection (ADR 0026 §12, personal-cfo-l8oh) =====

/// One projected series: a single liquid account, or the synthetic "Unallocated
/// cash" bucket (`account_id == None`). Carries the same per-day shape as the
/// aggregate forecast, including the events attributed to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSeriesView {
    /// The liquid account id, or `None` for the Unallocated series.
    pub account_id: Option<Uuid>,
    /// Display name (account name, or `"Unallocated cash"`).
    pub name: String,
    /// The account's subtype storage token, if any (ADR 0028).
    pub subtype: Option<String>,
    /// The cash tier this series rolls into: `spendable` / `reserve` /
    /// `unallocated`.
    pub tier: String,
    /// Per-day projected balance + the events attributed to this series.
    pub days: Vec<ForecastDayView>,
}

/// A single day's closing balance — a group-series row (lighter than
/// [`ForecastDayView`]: no per-event detail at the group level).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayBalance {
    /// The household-local calendar date.
    pub date: NaiveDate,
    /// Projected closing balance band for the group on that day.
    pub closing: Band,
}

/// A per-tier group series: the summed running balance of its member accounts
/// (`spendable` / `reserve` / `unallocated` / `net`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupSeriesView {
    /// `spendable` / `reserve` / `unallocated` / `net`.
    pub tier: String,
    /// The per-day closing balance for the group.
    pub closings: Vec<DayBalance>,
}

/// The multi-series Future Cash projection (ADR 0026 §12): one series per liquid
/// account (+ Unallocated for un-attributable flows), plus the per-tier group
/// rollups. Every per-account closing sums, day by day, to the aggregate (`net`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiSeriesForecast {
    /// The single currency the projection is computed in.
    pub currency: Currency,
    /// The first projected day (household-local "today").
    pub start_date: NaiveDate,
    /// Number of days projected.
    pub horizon_days: u32,
    /// One series per liquid account, plus Unallocated when it has flows.
    pub accounts: Vec<AccountSeriesView>,
    /// The tier rollups: spendable, reserve, (unallocated if any), net.
    pub groups: Vec<GroupSeriesView>,
}

/// A single realized historical closing balance (cf-history, ADR 0050 / ADR 0026 §13a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryDay {
    /// The household-local calendar date.
    pub date: NaiveDate,
    /// The account's actual closing balance that day, in minor units.
    pub closing_minor: i64,
}

/// One liquid account's REALIZED historical balance series — the backward-looking companion
/// to the forward [`AccountSeriesView`] cone. Folded backward from today's assertion-anchored
/// balance over the ledger postings (`bal(D) = bal_today − Σ postings after D`), and clamped to
/// the account's earliest real data so nothing before it is fabricated (the honesty rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountHistoryView {
    /// The liquid account id.
    pub account_id: Uuid,
    /// Display name.
    pub name: String,
    /// The account's subtype storage token, if any (ADR 0028).
    pub subtype: Option<String>,
    /// The cash tier this series rolls into (`spendable` / `reserve`), or `card` for a
    /// credit card's owed-balance history (stored signed, negative when owed).
    pub tier: String,
    /// The realized closing balance each day, oldest first.
    pub days: Vec<HistoryDay>,
}

/// The multi-series realized cash-flow HISTORY (cf-history): one realized daily-closing series
/// per liquid account, each honest back only as far as its own real data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashFlowHistory {
    /// The single currency the history is computed in.
    pub currency: Currency,
    /// The earliest day any account has data for (clamped to real data — may be later than the
    /// requested lookback when a household is young).
    pub start_date: NaiveDate,
    /// The last realized day (household-local "today").
    pub end_date: NaiveDate,
    /// One realized series per liquid account.
    pub accounts: Vec<AccountHistoryView>,
}

/// A liquid-cash account's projection inputs.
pub(super) struct LiquidAccount {
    pub(super) id: Uuid,
    pub(super) ledger_account_id: Uuid,
    pub(super) name: String,
    pub(super) currency: String,
    pub(super) subtype: Option<String>,
}

/// Compute the per-account and per-group Future Cash projection (ADR 0026 §12).
///
/// Runs the unchanged Layer-1 engine once per series — each liquid account, plus
/// one "Unallocated cash" series for flows not tied to a liquid account (manual
/// entries, or income/bills with an unset or non-liquid deposit/autopay account).
/// Because the fold is linear, the per-account closings sum to the aggregate
/// [`compute`] day by day (the reconciliation invariant).
///
/// # Errors
/// Returns [`DbError`] on a read failure, a malformed schedule, mixed-currency
/// liquid accounts, or a forecast arithmetic failure.
pub(crate) fn compute_by_account(
    conn: &Connection,
    as_of: DateTime<Utc>,
    horizon_days: u32,
    scenarios: &[Uuid],
) -> Result<MultiSeriesForecast, DbError> {
    let tz = read_household_tz(conn)?;
    let accounts = read_liquid_accounts(conn)?;
    let currency = liquid_currency(conn, &accounts)?;
    let horizon = Horizon::new(as_of, horizon_days);
    let start_date = as_of.with_timezone(&tz).date_naive();

    let Some((start, end)) = horizon.window(tz) else {
        return Ok(MultiSeriesForecast {
            currency,
            start_date,
            horizon_days,
            accounts: Vec::new(),
            groups: Vec::new(),
        });
    };

    // The full event stream — identical to the aggregate's — plus display names.
    // An archived, expired, or deleted scenario must not reach the run (ADR 0051).
    // Expired/archived selections drop out but the survivors keep their order — order
    // is the precedence (ADR 0059 §1).
    let scenarios = &crate::scenarios::effective_scenarios(conn, scenarios, start_date)?[..];
    let overrides = entity_overrides(conn, scenarios)?;
    // Planned per-category spend changes shift each account's modelled draw
    // (personal-cfo-4d8.27.6.2); loaded once for the whole run.
    let adjustments = crate::forecast_overrides::category_spend_adjustments(conn, scenarios)?;
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

    // Attribute each event to a liquid account (or leave it for Unallocated).
    let liquid_ids: HashSet<Uuid> = accounts.iter().map(|a| a.id).collect();
    let attribution = build_attribution(conn, &liquid_ids)?;
    let mut partitions: HashMap<Option<Uuid>, Vec<ForecastEvent>> = HashMap::new();
    for event in events {
        let account = attribution.get(&event.source_event_id).copied();
        partitions.entry(account).or_default().push(event);
    }
    // Recurring transfers attribute both legs directly (ADR 0026 §14): −amount to
    // the source account, +amount to the destination. The aggregate omitted them
    // (net zero), so the per-account series still reconcile to it.
    collect_transfer_legs(conn, start, end, &mut partitions, &mut names)?;

    // The window each account learns its own variable spend from (mirrors the aggregate's
    // apply_layer2_spend). Two uncertainty sources land on a liquid account's cone: its own
    // CONTINUOUS discretionary spend (the per-account cash cone), and a LUMP at each future
    // full-payer card payment date it funds (ADR 0050; revolvers are the MC follow-up).
    let spend_window_start = start
        .checked_sub_months(Months::new(LAYER2_HISTORY_WINDOW_MONTHS))
        .unwrap_or(start);
    let card_mape_bps =
        crate::forecast_backtest::latest_card_statement_mape(conn)?.map_or(0, |(bps, _sample)| bps);
    let card_lumps = if card_mape_bps > 0 {
        collect_card_lump_injections(conn, start, end, currency, &overrides, card_mape_bps)?
    } else {
        HashMap::new()
    };

    // Fit every account's spend model up front: a planned category change is a
    // HOUSEHOLD-level figure ("$200/month less on dining"), but each account carries its
    // own model, so handing the same delta to every account would cut the plan once per
    // account — the per-account chart would disagree with the aggregate about what the
    // very same plan does. Apportioning by each account's share of that category's
    // modelled spend keeps both views telling the same story.
    let spend_models: Vec<Option<SpendModel>> = accounts
        .iter()
        .map(|account| fit_account_spend_model(conn, account.id, spend_window_start, start))
        .collect::<Result<_, DbError>>()?;
    let per_account_adjustments = apportion_adjustments(&adjustments, &spend_models);

    // One series per liquid account (every account, even with no flows).
    let mut account_views = Vec::with_capacity(accounts.len() + 1);
    for (index, account) in accounts.iter().enumerate() {
        let starting = Money::new(
            crate::assertion_anchored_balance(conn, account.id, account.ledger_account_id)?,
            currency,
        );
        let events = partitions.remove(&Some(account.id)).unwrap_or_default();
        let spend_model = spend_models[index].as_ref();
        let lumps = card_lumps.get(&account.id).map_or(&[][..], Vec::as_slice);
        account_views.push(AccountSeriesView {
            account_id: Some(account.id),
            name: account.name.clone(),
            subtype: account.subtype.clone(),
            tier: tier_token(account.subtype.as_deref()).to_owned(),
            days: run_series(
                &events,
                starting,
                horizon,
                tz,
                &names,
                Overlays {
                    spend_model,
                    lumps,
                    adjustments: &per_account_adjustments[index],
                },
            )?,
        });
    }
    // The Unallocated series — only when there are un-attributable flows. It is not a real
    // spending account, so it carries no cone (the deterministic line passes through).
    if let Some(events) = partitions.remove(&None) {
        if !events.is_empty() {
            account_views.push(AccountSeriesView {
                account_id: None,
                name: "Unallocated cash".to_owned(),
                subtype: None,
                tier: "unallocated".to_owned(),
                days: run_series(
                    &events,
                    Money::zero(currency),
                    horizon,
                    tz,
                    &names,
                    Overlays {
                        spend_model: None,
                        lumps: &[],
                        adjustments: &[],
                    },
                )?,
            });
        }
    }

    let groups = roll_up_groups(&account_views, currency)?;
    Ok(MultiSeriesForecast {
        currency,
        start_date,
        horizon_days,
        accounts: account_views,
        groups,
    })
}

/// Compute the realized cash-flow HISTORY over the trailing `lookback_days` (cf-history,
/// personal-cfo-4d8.27.5.2). Per spending account — every liquid account plus each same-currency
/// credit card (tier `card`, the Account Detail owed-balance history) — folds its actual closing
/// balance backward from today's assertion-anchored balance over the ledger postings, clamped to
/// the account's earliest real data (the honesty rule, ADR 0026 §13a: never fabricate a balance
/// before there was data). Card balances stay in STORED signed form (negative owed) like every
/// other read; the frontend renders them as positive amount-owed via its sign convention.
///
/// # Errors
/// Returns [`DbError`] on a read failure or mixed-currency liquid accounts.
pub(crate) fn compute_cash_flow_history(
    conn: &Connection,
    as_of: DateTime<Utc>,
    lookback_days: u32,
) -> Result<CashFlowHistory, DbError> {
    let tz = read_household_tz(conn)?;
    let accounts = read_liquid_accounts(conn)?;
    let currency = liquid_currency(conn, &accounts)?;
    let today = as_of.with_timezone(&tz).date_naive();
    // An absurd lookback that overflows the date range means "as far back as there is data",
    // so fall back to the minimum date (the per-account honest clamp then bounds it), not today.
    let requested_start = today
        .checked_sub_days(chrono::Days::new(u64::from(lookback_days)))
        .unwrap_or(NaiveDate::MIN);

    let cards = read_card_accounts(conn)?;
    let mut account_views = Vec::with_capacity(accounts.len() + cards.len());
    let mut earliest = today;
    // A foreign-currency card is skipped, exactly like the payment-event collectors.
    let card_rows = cards.iter().filter(|c| c.currency == currency.code());
    for (account, tier) in accounts
        .iter()
        .map(|a| (a, tier_token(a.subtype.as_deref())))
        .chain(card_rows.map(|c| (c, "card")))
    {
        let days = account_history_series(conn, account, today, requested_start)?;
        if let Some(first) = days.first() {
            earliest = earliest.min(first.date);
        }
        account_views.push(AccountHistoryView {
            account_id: account.id,
            name: account.name.clone(),
            subtype: account.subtype.clone(),
            tier: tier.to_owned(),
            days,
        });
    }
    Ok(CashFlowHistory {
        currency,
        start_date: earliest,
        end_date: today,
        accounts: account_views,
    })
}

/// Every credit-card account with the projection-input shape (for the history fold — the
/// forecast's card reads live in `card_cycles`, which need debt terms; history needs none).
fn read_card_accounts(conn: &Connection) -> Result<Vec<LiquidAccount>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, ledger_account_id, name, currency, subtype
         FROM accounts
         WHERE cashflow_role = 'credit_facility'
         ORDER BY name COLLATE NOCASE, id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LiquidAccount {
                id: r.get(0)?,
                ledger_account_id: r.get(1)?,
                name: r.get(2)?,
                currency: r.get(3)?,
                subtype: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One account's realized daily closing balances over `[honest_start, today]`, oldest first,
/// clamped to the account's earliest real posting (never before it existed).
fn account_history_series(
    conn: &Connection,
    account: &LiquidAccount,
    today: NaiveDate,
    requested_start: NaiveDate,
) -> Result<Vec<HistoryDay>, DbError> {
    let bal_today = crate::assertion_anchored_balance(conn, account.id, account.ledger_account_id)?;
    // The honest start: the later of the requested window and the account's first real activity.
    let honest_start = match earliest_posting_date(conn, account.ledger_account_id)? {
        Some(earliest) => requested_start.max(earliest),
        // No postings at all → only today has a (possibly asserted) balance.
        None => today,
    };
    let deltas = daily_posting_deltas(conn, account.ledger_account_id, honest_start, today)?;

    // Walk backward: bal(D) = bal_today − Σ(postings dated after D). `running` starts at the sum
    // of postings dated AFTER today — `bal_today` (assertion-anchored) counts every posting with
    // no upper date bound, so a future-dated or evening (UTC-rolled-over) posting would otherwise
    // never be subtracted and would shift the whole history; seeding `running` with it makes each
    // day exactly `Σ(postings ≤ D)`. Then `running` accumulates postings after the current day as
    // we step earlier.
    let after_today: i64 = conn.query_row(
        "SELECT COALESCE(SUM(minor_units), 0) FROM ledger_postings
         WHERE ledger_account_id = ?1 AND posting_date > ?2",
        params![account.ledger_account_id, today.to_string()],
        |r| r.get(0),
    )?;
    let mut out = Vec::new();
    let mut running = after_today;
    let mut day = today;
    loop {
        out.push(HistoryDay {
            date: day,
            closing_minor: bal_today.saturating_sub(running),
        });
        if day <= honest_start {
            break;
        }
        running = running.saturating_add(deltas.get(&day).copied().unwrap_or(0));
        match day.pred_opt() {
            Some(prev) => day = prev,
            None => break,
        }
    }
    out.reverse();
    Ok(out)
}

/// Σ of `ledger_postings.minor_units` per posting date for one ledger account over
/// `(start, today]` (a posting on `start` itself is excluded — it anchors `bal(start)`).
fn daily_posting_deltas(
    conn: &Connection,
    ledger_account_id: Uuid,
    start: NaiveDate,
    today: NaiveDate,
) -> Result<HashMap<NaiveDate, i64>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT substr(posting_date, 1, 10) AS d, COALESCE(SUM(minor_units), 0)
           FROM ledger_postings
          WHERE ledger_account_id = ?1 AND posting_date > ?2 AND posting_date <= ?3
          GROUP BY d",
    )?;
    let rows = stmt.query_map(
        params![ledger_account_id, start.to_string(), today.to_string()],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    )?;
    let mut out = HashMap::new();
    for row in rows {
        let (date_str, sum) = row?;
        if let Ok(date) = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
            out.insert(date, sum);
        }
    }
    Ok(out)
}

/// The earliest posting date for a ledger account (its first real activity — typically the
/// opening-balance posting), or `None` if it has no postings.
fn earliest_posting_date(
    conn: &Connection,
    ledger_account_id: Uuid,
) -> Result<Option<NaiveDate>, DbError> {
    let raw: Option<String> = conn.query_row(
        "SELECT MIN(substr(posting_date, 1, 10)) FROM ledger_postings WHERE ledger_account_id = ?1",
        [ledger_account_id],
        |r| r.get(0),
    )?;
    Ok(raw.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()))
}

/// Run the Layer-1 engine for one series, optionally widen it into this account's own
/// discretionary-spend cone, and enrich it with event names.
fn run_series(
    events: &[ForecastEvent],
    starting: Money,
    horizon: Horizon,
    tz: Tz,
    names: &HashMap<Uuid, String>,
    overlays: Overlays<'_>,
) -> Result<Vec<ForecastDayView>, DbError> {
    let series = forecast_layer1(events, starting, horizon, tz)
        .map_err(|e| DbError::InvalidCommand(format!("per-account forecast failed: {e}")))?;
    // Widen the deterministic line into a monotonically fanning cone from this account's own
    // continuous discretionary spend (ADR 0026 §7 / ADR 0050). `None` below the history gate.
    let series = match overlays.spend_model {
        Some(model) => widen_with_spend_adjusted(&series, model, overlays.adjustments),
        None => series,
    };
    // …then add a lump at each future full-payer card payment this account funds (ADR 0050).
    let series = if overlays.lumps.is_empty() {
        series
    } else {
        widen_with_lumps(&series, overlays.lumps)
    };
    Ok(to_day_views(series, names))
}

/// The statistical overlays applied on top of one account's deterministic series:
/// its fitted spend cone, its card-payment lumps, and any planned category changes
/// apportioned to it. Grouped so `run_series` keeps a readable signature.
#[derive(Clone, Copy)]
pub(super) struct Overlays<'a> {
    pub spend_model: Option<&'a SpendModel>,
    pub lumps: &'a [LumpInjection],
    pub adjustments: &'a [SpendAdjustment],
}

/// Split each household-level [`SpendAdjustment`] across the accounts in proportion to
/// how much of that category each one actually spends (personal-cfo-4d8.27.6.2).
///
/// The shares are taken from each account's own modelled monthly spend for the category,
/// so an account that never buys dinner absorbs none of a dining cut. The largest share
/// takes the integer remainder, so the parts sum to exactly the household figure and the
/// per-account views cannot drift from the aggregate by rounding.
///
/// An adjustment for a category no account models is dropped: there is nothing to reduce.
fn apportion_adjustments(
    adjustments: &[SpendAdjustment],
    models: &[Option<SpendModel>],
) -> Vec<Vec<SpendAdjustment>> {
    let mut out = vec![Vec::new(); models.len()];
    for adjustment in adjustments {
        let weights: Vec<i64> = models
            .iter()
            .map(|m| {
                m.as_ref()
                    .and_then(|m| m.expected_monthly(&adjustment.category))
                    .unwrap_or(0)
                    .max(0)
            })
            .collect();
        let total: i64 = weights.iter().sum();
        if total <= 0 {
            continue;
        }
        // Integer split, then hand the remainder to the heaviest share.
        // i128 for the intermediate product: both factors are money figures, and the
        // multiply is the only place two of them meet.
        let mut parts: Vec<i64> = weights
            .iter()
            .map(|w| {
                i64::try_from(
                    i128::from(adjustment.delta_cents_per_month) * i128::from(*w)
                        / i128::from(total),
                )
                .unwrap_or(0)
            })
            .collect();
        let assigned: i64 = parts.iter().sum();
        if let Some(heaviest) = weights
            .iter()
            .enumerate()
            .max_by_key(|(i, w)| (**w, std::cmp::Reverse(*i)))
            .map(|(i, _)| i)
        {
            parts[heaviest] += adjustment.delta_cents_per_month - assigned;
        }
        for (index, part) in parts.into_iter().enumerate() {
            if part != 0 {
                out[index].push(SpendAdjustment {
                    category: adjustment.category.clone(),
                    delta_cents_per_month: part,
                    start: adjustment.start,
                    end: adjustment.end,
                });
            }
        }
    }
    out
}

/// Fit a per-account Layer-2 spend model from the account's OWN categorized variable spend over
/// `[window_start, today)`, gated on at least [`LAYER2_MIN_HISTORY_MONTHS`] distinct months of
/// its own history. Returns `None` below the gate, so a thin-history account keeps the
/// trustworthy deterministic line rather than a fabricated cone.
fn fit_account_spend_model(
    conn: &Connection,
    account_id: Uuid,
    window_start: NaiveDate,
    today: NaiveDate,
) -> Result<Option<SpendModel>, DbError> {
    let history = read_variable_spend_history(conn, window_start, today, Some(account_id))?;
    if distinct_spend_months(&history) < LAYER2_MIN_HISTORY_MONTHS {
        return Ok(None);
    }
    Ok(Some(SpendModel::fit(&history)))
}

/// Every liquid-cash account with its projection inputs. Matches the account set
/// the aggregate [`liquid_starting_balance`] sums, so the series reconcile.
pub(super) fn read_liquid_accounts(conn: &Connection) -> Result<Vec<LiquidAccount>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, ledger_account_id, name, currency, subtype
         FROM accounts
         WHERE cashflow_role = 'liquid_cash' AND active = 1
         ORDER BY name COLLATE NOCASE, id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LiquidAccount {
                id: r.get(0)?,
                ledger_account_id: r.get(1)?,
                name: r.get(2)?,
                currency: r.get(3)?,
                subtype: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The single currency the projection runs in — the liquid accounts' shared
/// currency, or the base-currency setting (then USD) when there are none. Rejects
/// mixed-currency liquid accounts, exactly as the aggregate does.
pub(super) fn liquid_currency(
    conn: &Connection,
    accounts: &[LiquidAccount],
) -> Result<Currency, DbError> {
    let mut code: Option<&str> = None;
    for account in accounts {
        match code {
            None => code = Some(&account.currency),
            Some(existing) if existing != account.currency => {
                return Err(DbError::InvalidCommand(
                    "Future Cash forecast does not support mixed-currency liquid accounts yet"
                        .to_owned(),
                ));
            }
            Some(_) => {}
        }
    }
    match code {
        Some(c) => currency_from_code(c),
        None => Ok(reporting_currency(conn)?.unwrap_or(Currency::Usd)),
    }
}

/// Map each source entity to the liquid account its flows are attributed to:
/// income → its deposit account, recurring obligations → their autopay account,
/// keeping only attributions to a liquid account (others fall through to
/// Unallocated). ADR 0026 §12.
fn build_attribution(
    conn: &Connection,
    liquid_ids: &HashSet<Uuid>,
) -> Result<HashMap<Uuid, Uuid>, DbError> {
    let mut map = HashMap::new();
    for (table, account_col) in [
        ("income_sources", "deposit_account_id"),
        ("recurring_events", "autopay_account_id"),
    ] {
        let sql = format!("SELECT id, {account_col} FROM {table} WHERE {account_col} IS NOT NULL");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Uuid>(0)?, r.get::<_, Uuid>(1)?)))?;
        for row in rows {
            let (source_id, account_id) = row?;
            if liquid_ids.contains(&account_id) {
                map.insert(source_id, account_id);
            }
        }
    }
    // A card payment (personal-cfo-6wk.10, keyed by the card account id) leaves the card's
    // paying source (a liquid account), not the card; attribute it there so the per-account
    // series match the aggregate. A missing/non-liquid paying source leaves it Unallocated (the
    // aggregate still carries the outflow).
    for card in read_cards_with_cycle(conn)? {
        if let Some(source) = card.paying_source {
            if liquid_ids.contains(&source) {
                map.insert(card.account_id, source);
            }
        }
    }
    // A loan payment (personal-cfo-6wk.4) leaves the loan's paying source, not the loan;
    // attribute the loan's payment event (keyed by the loan account id) there.
    for loan in read_loans_with_payment(conn)? {
        if let Some(source) = loan.paying_source {
            if liquid_ids.contains(&source) {
                map.insert(loan.account_id, source);
            }
        }
    }
    // A naive (cycle-less) card payment (personal-cfo-4d8.23.2) is keyed by the card account id
    // like the cycle-card payment above and likewise leaves the card's paying source — attribute
    // it there so the per-account series + cash-tier rollups match the aggregate (else it falls
    // to Unallocated and the paying account reads too high).
    for card in read_cards_without_cycle(conn)? {
        if let Some(source) = card.paying_source {
            if liquid_ids.contains(&source) {
                map.insert(card.account_id, source);
            }
        }
    }
    // A manual future entry (personal-cfo-4d8.24.3) is keyed by its assumption-event id in the
    // stream (`collect_manual_events` sets `source_event_id = entry.id`). When the user attributed
    // it to a liquid account, route the projected flow there; otherwise it stays Unallocated (the
    // aggregate carries it either way, preserving the reconciliation invariant, ADR 0026 §12).
    // Base + scenario entries are both included (the map is keyed by unique entry id).
    let mut stmt = conn.prepare(
        "SELECT id, json_extract(params_json, '$.account_id')
           FROM forecast_assumption_events
          WHERE status = 'active' AND kind = 'one_time_event'
            AND json_extract(params_json, '$.account_id') IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, Uuid>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (entry_id, account_str) = row?;
        if let Ok(account_id) = Uuid::parse_str(&account_str) {
            if liquid_ids.contains(&account_id) {
                map.insert(entry_id, account_id);
            }
        }
    }
    Ok(map)
}

/// The cash tier a liquid account's subtype rolls into (ADR 0028); an unclassified
/// liquid account counts as spendable.
fn tier_token(subtype: Option<&str>) -> &'static str {
    match subtype
        .and_then(AccountSubtype::from_token)
        .and_then(AccountSubtype::cash_tier)
    {
        Some(CashTier::Reserve) => "reserve",
        Some(CashTier::Spendable) | None => "spendable",
    }
}

/// Sum the per-account series into the tier group series (spendable / reserve /
/// unallocated) plus `net` (every series), day by day. All series share the
/// horizon's dates, so the rollup is index-aligned.
fn roll_up_groups(
    accounts: &[AccountSeriesView],
    currency: Currency,
) -> Result<Vec<GroupSeriesView>, DbError> {
    let Some(first) = accounts.first() else {
        return Ok(Vec::new());
    };
    let dates: Vec<NaiveDate> = first.days.iter().map(|d| d.date).collect();
    let zero = Money::zero(currency);
    let flat = || {
        vec![
            Band {
                p10: zero,
                p50: zero,
                p90: zero
            };
            dates.len()
        ]
    };

    let (mut spendable, mut reserve, mut unallocated, mut net) = (flat(), flat(), flat(), flat());
    let mut has_unallocated = false;
    for account in accounts {
        let target = match account.tier.as_str() {
            "reserve" => &mut reserve,
            "unallocated" => {
                has_unallocated = true;
                &mut unallocated
            }
            _ => &mut spendable,
        };
        for (i, day) in account.days.iter().enumerate() {
            target[i] = add_band(target[i], day.closing)?;
            net[i] = add_band(net[i], day.closing)?;
        }
    }

    let group = |tier: &str, bands: &[Band]| GroupSeriesView {
        tier: tier.to_owned(),
        closings: dates
            .iter()
            .zip(bands)
            .map(|(&date, &closing)| DayBalance { date, closing })
            .collect(),
    };
    let mut groups = vec![group("spendable", &spendable), group("reserve", &reserve)];
    if has_unallocated {
        groups.push(group("unallocated", &unallocated));
    }
    groups.push(group("net", &net));
    Ok(groups)
}

/// Add two percentile bands (checked, per percentile).
fn add_band(a: Band, b: Band) -> Result<Band, DbError> {
    let add = |x: Money, y: Money| {
        x.checked_add(y)
            .map_err(|e| DbError::InvalidCommand(format!("group rollup overflow: {e}")))
    };
    Ok(Band {
        p10: add(a.p10, b.p10)?,
        p50: add(a.p50, b.p50)?,
        p90: add(a.p90, b.p90)?,
    })
}
