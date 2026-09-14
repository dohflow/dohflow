//! Forecast Readiness (R1) scoring and the capability-unlock ladder (moved
//! verbatim from `forecast.rs`).

use std::collections::HashSet;

use chrono::{DateTime, Months, NaiveDate, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

use super::account_series::read_liquid_accounts;
use super::aggregate::{
    distinct_spend_months, distinct_variable_spend_months_all_accounts,
    LAYER2_HISTORY_WINDOW_MONTHS, LAYER2_MIN_HISTORY_MONTHS,
};
use super::events::{card_charged_bill_ids, read_household_tz};
use super::read_variable_spend_history;
use crate::DbError;

// ===== Forecast Readiness — R1 subset (ADR 0026 §13, personal-cfo-6vj9) =====

/// Calibration constants for the R1 readiness score (ADR 0026 §13). Tunable here
/// without an interface change; the dashboard shows the per-factor breakdown either
/// way. A balance older than the freshness horizon contributes nothing to freshness.
const READINESS_FRESHNESS_HORIZON_DAYS: f64 = 45.0;
// Re-pinned 2026-06-29 (nxgx, ADR 0026 §8/§18) — the full seven-factor model: the
// actuals-backed-recurrence factor (§17) plus the per-vault backtest-MAPE factor (§18) are
// both live. Weights sum to 1.0; coverage stays dominant (you need the inputs before
// anything else matters).
const READINESS_W_COVERAGE: f64 = 0.25;
const READINESS_W_FRESHNESS: f64 = 0.15;
const READINESS_W_EXPLAINED: f64 = 0.10;
const READINESS_W_CATEGORIZATION: f64 = 0.15;
const READINESS_W_SPENDING_HISTORY: f64 = 0.15;
const READINESS_W_RECURRENCE_ACTUALS: f64 = 0.10;
const READINESS_W_BACKTEST_MAPE: f64 = 0.10;
/// Trailing window the categorization factor measures (ADR 0026 §8: "% of the last 90 days").
const READINESS_CATEGORIZATION_WINDOW_DAYS: u64 = 90;
/// Realized actuals a recurring event needs before it counts as accuracy-verifiable
/// (ADR 0026 §8: "recurring events with ≥3 actuals").
const READINESS_ACTUALS_PER_EVENT_TARGET: i64 = 3;
/// Backtest MAPE (bps) at or below which the forecast-accuracy factor earns full credit
/// (≤ 10% error is "within envelope"); at or above the ceiling it earns none (≥ 30%).
const READINESS_BACKTEST_MAPE_TARGET_BPS: i64 = 1_000;
const READINESS_BACKTEST_MAPE_CEILING_BPS: i64 = 3_000;

/// One readiness factor's contribution plus the action that improves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessFactor {
    /// Stable key: `coverage` | `freshness` | `explained`.
    pub key: String,
    /// Display label.
    pub label: String,
    /// This factor's score, 0–100.
    pub score: u8,
    /// One-line explanation / the action that improves it.
    pub detail: String,
}

/// The R1 Forecast Readiness score (ADR 0026 §13): a 0–100 data-maturity indicator
/// derived on read from coverage, balance freshness, and the explained ratio.
/// Informational in R1 (the activation gate arrives with Layer-2 output, `nxgx`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForecastReadiness {
    /// Overall score, 0–100.
    pub score: u8,
    /// Per-factor breakdown, always in this order: coverage, freshness, explained,
    /// categorization, spending_history, recurrence_actuals, backtest_mape.
    pub factors: Vec<ReadinessFactor>,
}

/// Map a 0.0–1.0 ratio to a 0–100 factor score.
fn readiness_pct(ratio: f64) -> u8 {
    (ratio.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// Share of recent spend transactions that carry a category (ADR 0026 §8 "% of the last 90
/// days categorized") + a plain-language detail. Neutral (1.0) when there's no recent spend
/// to categorize, so the assert-only manual workflow (ADR 0027) is never penalized.
fn recent_categorization_ratio(
    conn: &Connection,
    today: NaiveDate,
) -> Result<(f64, String), DbError> {
    let window_start = today
        .checked_sub_days(chrono::Days::new(READINESS_CATEGORIZATION_WINDOW_DAYS))
        .unwrap_or(today);
    let (total, categorized): (i64, i64) = conn.query_row(
        "SELECT COUNT(DISTINCT lt.id), COUNT(DISTINCT tc.transaction_id)
         FROM ledger_transactions lt
         JOIN ledger_postings lp ON lp.transaction_id = lt.id AND lp.minor_units < 0
         JOIN accounts a ON a.ledger_account_id = lp.ledger_account_id
         LEFT JOIN transaction_categorizations tc ON tc.transaction_id = lt.id
         WHERE lt.voided_at IS NULL
           AND substr(lt.occurred_at, 1, 10) >= ?1",
        params![window_start.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if total == 0 {
        return Ok((1.0, "No recent spending to categorize.".to_owned()));
    }
    #[allow(clippy::cast_precision_loss)]
    let ratio = categorized as f64 / total as f64;
    let detail = if ratio >= 0.9 {
        "Your recent spending is well categorized.".to_owned()
    } else {
        format!("Categorize more of your recent spending ({categorized} of {total} done).")
    };
    Ok((ratio, detail))
}

/// Actuals-backed-recurrence maturity (ADR 0026 §8, R3): how close the user's recurring
/// events are to having enough realized history to verify forecast accuracy. Each active
/// recurring event (income + bills) earns up to [`READINESS_ACTUALS_PER_EVENT_TARGET`]
/// realized actuals (distinct realized dates with an `exact`/`matched` verdict in
/// `forecast_actuals`); the factor is the captured fraction. Neutral (1.0) when there are
/// no recurring events to verify, so a balances-only setup is never penalized. Reads
/// whatever actuals exist — actualization runs daily-on-open, not on this read.
fn recurrence_actuals_ratio(conn: &Connection) -> Result<(f64, String), DbError> {
    let mut active_ids: HashSet<Uuid> = crate::read_income_source_views(conn)?
        .iter()
        .filter(|s| s.active)
        .map(|s| s.id.as_uuid())
        .chain(
            crate::read_recurring_bill_views(conn)?
                .iter()
                .filter(|b| b.active)
                .map(|b| b.id.as_uuid()),
        )
        .collect();
    // Card-charged bills have no per-charge liquid outflow — their impact is the card payment
    // (ADR 0039 §2 / 6wk.10) — so they can't actualize against their natural
    // recurring_event_instance (dated on the charge day); exclude them from the verification
    // denominator until card-payment actualization lands (personal-cfo-6wk.9), rather than
    // silently capping this readiness factor for every household that uses one.
    let card_charged = card_charged_bill_ids(conn)?;
    active_ids.retain(|id| !card_charged.contains(id));
    let total = active_ids.len();
    if total == 0 {
        return Ok((
            1.0,
            "No recurring income or bills to verify yet.".to_owned(),
        ));
    }

    // Distinct realized dates per source event (dedups the same occurrence scored across
    // multiple persisted runs); only verdicts backed by a real transaction count.
    let mut stmt = conn.prepare(
        "SELECT fr.source_id, COUNT(DISTINCT fa.realized_date)
         FROM forecast_actuals fa
         JOIN forecast_rows fr ON fr.id = fa.forecast_row_id
         WHERE fa.match_status IN ('exact', 'matched') AND fr.source_id IS NOT NULL
         GROUP BY fr.source_id",
    )?;
    let per_event = stmt
        .query_map([], |r| Ok((r.get::<_, Uuid>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let target = READINESS_ACTUALS_PER_EVENT_TARGET;
    let mut captured = 0i64;
    let mut mature_events = 0usize;
    for (source_id, count) in per_event {
        if active_ids.contains(&source_id) {
            captured += count.min(target);
            if count >= target {
                mature_events += 1;
            }
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let ratio = (captured as f64 / (target * total as i64) as f64).clamp(0.0, 1.0);
    let detail = if mature_events == total {
        "Your recurring payments have enough recorded history to verify forecast accuracy."
            .to_owned()
    } else {
        format!(
            "{mature_events} of {total} recurring payments have enough recorded history to verify accuracy."
        )
    };
    Ok((ratio, detail))
}

/// Forecast-accuracy maturity (ADR 0026 §18, R3): how close the household's own past
/// forecasts have been, from the latest recorded backtest MAPE (`forecast_backtest_results`,
/// written daily-on-open). Lower error → higher score: ≤ 10% MAPE earns full credit, ≥ 30%
/// earns none, linear between. Neutral (1.0) until there's enough realized history to judge
/// — accuracy we can't yet measure shouldn't penalize a new vault.
fn backtest_mape_ratio(conn: &Connection) -> Result<(f64, String), DbError> {
    let Some((mape_bps, sample)) = crate::forecast_backtest::latest_mape(conn)? else {
        return Ok((
            1.0,
            "Keep recording — forecast accuracy appears once there's enough history to check."
                .to_owned(),
        ));
    };
    let ratio = if mape_bps <= READINESS_BACKTEST_MAPE_TARGET_BPS {
        1.0
    } else if mape_bps >= READINESS_BACKTEST_MAPE_CEILING_BPS {
        0.0
    } else {
        #[allow(clippy::cast_precision_loss)]
        let r = (READINESS_BACKTEST_MAPE_CEILING_BPS - mape_bps) as f64
            / (READINESS_BACKTEST_MAPE_CEILING_BPS - READINESS_BACKTEST_MAPE_TARGET_BPS) as f64;
        r
    };
    #[allow(clippy::cast_precision_loss)]
    let pct = mape_bps as f64 / 100.0;
    let detail = if ratio >= 0.99 {
        format!(
            "Your forecast has tracked reality closely (within {pct:.0}% over {sample} checks)."
        )
    } else {
        format!("Your forecast has been off by about {pct:.0}% on average ({sample} checks).")
    };
    Ok((ratio, detail))
}

/// The Layer-2 band's ACTUAL activation predicate: at least `LAYER2_MIN_HISTORY_MONTHS`
/// distinct months of LIQUID categorized variable spend — exactly what `apply_layer2_spend`
/// gates on. Distinct from the card-inclusive readiness `spending_history` indicator, which
/// diverges for card-heavy vaults until the band re-model injects card variance at the payment
/// date (ADR 0050 / ADR 0026 §13a). The `forecast_band` capability notice and the "range is
/// active" detail gate on THIS, not the indicator, so the app never announces a range it does
/// not draw (personal-cfo-4d8.27.1.2).
fn band_is_active(conn: &Connection, as_of: DateTime<Utc>) -> Result<bool, DbError> {
    let today = as_of.with_timezone(&read_household_tz(conn)?).date_naive();
    let start = today
        .checked_sub_months(Months::new(LAYER2_HISTORY_WINDOW_MONTHS))
        .unwrap_or(today);
    Ok(
        distinct_spend_months(&read_variable_spend_history(conn, start, today, None)?)
            >= LAYER2_MIN_HISTORY_MONTHS,
    )
}

/// Realized anchored history the `cash_flow_history` capability announces itself at
/// (ADR 0026 §13a cf-history addendum, personal-cfo-4d8.27.5.5).
const HISTORY_UNLOCK_MIN_DAYS: i64 = 30;

/// Whether the household has at least [`HISTORY_UNLOCK_MIN_DAYS`] of realized LIQUID
/// history — the earliest liquid-account posting (the same anchor the cf-history
/// honesty clamp folds back to) is that many days before household-local today. The
/// unlock is an announcement, not a gate: the Cash Flow history renders honestly at
/// any depth; this just marks when there is enough past to make the trend meaningful.
fn history_depth_is_sufficient(conn: &Connection, as_of: DateTime<Utc>) -> Result<bool, DbError> {
    let today = as_of.with_timezone(&read_household_tz(conn)?).date_naive();
    // LIQUID accounts only — the history view's chart is the liquid chart; a lone
    // card/loan posting must not announce it.
    let earliest: Option<String> = conn.query_row(
        "SELECT MIN(substr(lp.posting_date, 1, 10))
           FROM ledger_postings lp
           JOIN accounts a ON a.ledger_account_id = lp.ledger_account_id
          WHERE a.cashflow_role = 'liquid_cash' AND a.active = 1",
        [],
        |r| r.get(0),
    )?;
    Ok(earliest
        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .is_some_and(|d| (today - d).num_days() >= HISTORY_UNLOCK_MIN_DAYS))
}

/// Compute the R1 Forecast Readiness score as of `as_of` (ADR 0026 §13).
///
/// Derived on read from canonical state — coverage (liquid account [hard gate] +
/// income + bills), balance freshness (weakest-link assertion recency), and the
/// explained ratio (the unexplained plug, neutral with no recorded transactions so
/// the assert-only workflow is never penalized).
///
/// # Errors
/// Returns [`DbError`] on a read failure or a malformed stored schedule.
pub(crate) fn compute_forecast_readiness(
    conn: &Connection,
    as_of: DateTime<Utc>,
) -> Result<ForecastReadiness, DbError> {
    let accounts = read_liquid_accounts(conn)?;

    // Hard gate: no liquid account → no starting balance → no forecast.
    if accounts.is_empty() {
        return Ok(ForecastReadiness {
            score: 0,
            factors: vec![
                ReadinessFactor {
                    key: "coverage".to_owned(),
                    label: "Coverage".to_owned(),
                    score: 0,
                    detail: "Add a cash account so the forecast has a starting balance.".to_owned(),
                },
                ReadinessFactor {
                    key: "freshness".to_owned(),
                    label: "Balance freshness".to_owned(),
                    score: 0,
                    detail: "Set an account balance to anchor the forecast.".to_owned(),
                },
                ReadinessFactor {
                    key: "explained".to_owned(),
                    label: "Explained activity".to_owned(),
                    score: 100,
                    detail: "No recorded activity to reconcile yet.".to_owned(),
                },
                ReadinessFactor {
                    key: "categorization".to_owned(),
                    label: "Spending detail".to_owned(),
                    score: 100,
                    detail: "No spending recorded to categorize yet.".to_owned(),
                },
                ReadinessFactor {
                    key: "spending_history".to_owned(),
                    label: "Spending history".to_owned(),
                    score: 0,
                    detail: "Record and categorize spending to unlock the projected range."
                        .to_owned(),
                },
                ReadinessFactor {
                    key: "recurrence_actuals".to_owned(),
                    label: "Verified accuracy".to_owned(),
                    score: 100,
                    detail: "No recurring income or bills to verify yet.".to_owned(),
                },
                ReadinessFactor {
                    key: "backtest_mape".to_owned(),
                    label: "Forecast accuracy".to_owned(),
                    score: 100,
                    detail: "Keep recording — forecast accuracy appears once there's enough \
                             history to check."
                        .to_owned(),
                },
            ],
        });
    }

    // --- Coverage: account (present) + income + bills ---
    let has_income = crate::read_income_source_views(conn)?
        .iter()
        .any(|s| s.active);
    let has_bills = crate::read_recurring_bill_views(conn)?
        .iter()
        .any(|b| b.active);
    let coverage = (1.0 + u8::from(has_income) as f64 + u8::from(has_bills) as f64) / 3.0;
    let coverage_detail = match (has_income, has_bills) {
        (true, true) => "Accounts, income, and bills are all set.",
        (false, true) => "Add your income to project the money coming in.",
        (true, false) => "Add recurring bills to project the money going out.",
        (false, false) => "Add income and recurring bills to project your cash flow.",
    }
    .to_owned();

    // --- Freshness: the least-recently-anchored liquid account (weakest link). A
    //     balance is "anchored" by the most recent balance observation of any
    //     source (a manual assertion or an import) and, when an account has no
    //     observation, by its latest recorded posting — so the opening balance of
    //     a freshly created account reads as fresh instead of hitting a synthetic
    //     floor. This account set MUST match the accounts the forecast's starting
    //     balance sums, or readiness reports "current" over a forecast resting on a
    //     stale balance (or the reverse). Both sides now filter `active = 1`
    //     (ADR 0056) — the earlier note here said "do NOT add `active = 1` without
    //     also excluding archived accounts from the forecast", and ADR 0056 is that
    //     exclusion, so the two moved together. Keep them that way: changing one of
    //     the six liquid reads alone reintroduces exactly the failure that note was
    //     written to prevent. An account with no balance evidence at all does not
    //     drag the score (personal-cfo-4d8.27.1.1). ---
    let today = as_of.with_timezone(&read_household_tz(conn)?).date_naive();
    let mut freshness_stmt = conn.prepare(
        "SELECT a.name,
                COALESCE(
                    (SELECT MAX(substr(bo.observed_at, 1, 10)) FROM balance_observations bo
                      WHERE bo.account_id = a.id),
                    (SELECT MAX(substr(lp.posting_date, 1, 10)) FROM ledger_postings lp
                      WHERE lp.ledger_account_id = a.ledger_account_id)
                ) AS anchored_on
           FROM accounts a
          WHERE a.cashflow_role = 'liquid_cash' AND a.active = 1",
    )?;
    let anchors = freshness_stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut max_staleness_days = 0i64;
    let mut any_evidence = false;
    let mut weakest_name: Option<String> = None;
    for (name, anchored_on) in anchors {
        let Some(date) = anchored_on
            .as_deref()
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        else {
            // No observation and no postings → no balance anchor at all; a brand-new
            // empty account should not read as stale, so it does not drag the score.
            continue;
        };
        any_evidence = true;
        let staleness = (today - date).num_days().max(0);
        if staleness > max_staleness_days {
            max_staleness_days = staleness;
            weakest_name = Some(name);
        }
    }
    // No account has any balance evidence at all → prompt to set balances (0),
    // rather than reading as vacuously fresh because the staleness stayed 0.
    let freshness = if any_evidence {
        (1.0 - max_staleness_days as f64 / READINESS_FRESHNESS_HORIZON_DAYS).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let freshness_detail = if !any_evidence {
        "Set your account balances so the forecast starts from today.".to_owned()
    } else if max_staleness_days as f64 >= READINESS_FRESHNESS_HORIZON_DAYS {
        match &weakest_name {
            Some(name) => {
                format!("Update your balances — {name} hasn't been confirmed in over six weeks.")
            }
            None => "Update your balances — they're over six weeks old.".to_owned(),
        }
    } else {
        "Your balances are current.".to_owned()
    };

    // --- Explained ratio: neutral with no recorded transactions (ADR 0027 primary
    //     path is assert-only); otherwise how much of the balance recorded activity
    //     explains vs. the unexplained plug (`ueg6`). ---
    let txn_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM transaction_display_rows_read_model",
        [],
        |r| r.get(0),
    )?;
    let (explained, explained_detail) = if txn_count == 0 {
        (
            1.0,
            "No recorded transactions to reconcile — balances stand on their own.".to_owned(),
        )
    } else {
        let mut sum_plug = 0i64;
        let mut sum_balance = 0i64;
        for account in &accounts {
            if let Some(plug) =
                crate::unexplained_adjustment(conn, account.id, account.ledger_account_id)?
            {
                sum_plug = sum_plug.saturating_add(plug.abs());
            }
            sum_balance = sum_balance.saturating_add(
                crate::assertion_anchored_balance(conn, account.id, account.ledger_account_id)?
                    .abs(),
            );
        }
        if sum_balance == 0 {
            (1.0, "Recorded activity explains your balances.".to_owned())
        } else {
            let ratio = (1.0 - sum_plug as f64 / sum_balance as f64).clamp(0.0, 1.0);
            let detail = if ratio >= 0.95 {
                "Recorded activity explains your balances.".to_owned()
            } else {
                "Record transactions to explain the unexplained balance adjustment.".to_owned()
            };
            (ratio, detail)
        }
    };

    // --- Spending detail (categorization, ADR 0026 §8) + spending history — derived on
    //     demand. The spending-history INDICATOR (this factor's score) counts categorized
    //     variable spend across ALL accounts incl. credit cards (interim, ADR 0050 / §13a), so
    //     a card-based household is credited for the history it recorded instead of reading 0.
    //     But the Layer-2 band's ACTUAL activation stays liquid-only (`band_active`) until the
    //     band re-model, so the "range is active" language + the forecast_band capability gate
    //     on `band_active`, NOT the score — never announcing a range that is not drawn
    //     (personal-cfo-4d8.27.1.2). ---
    let (categorization, categorization_detail) = recent_categorization_ratio(conn, today)?;
    let spend_window_start = today
        .checked_sub_months(Months::new(LAYER2_HISTORY_WINDOW_MONTHS))
        .unwrap_or(today);
    let spend_months =
        distinct_variable_spend_months_all_accounts(conn, spend_window_start, today)?;
    let band_active = band_is_active(conn, as_of)?;
    #[allow(clippy::cast_precision_loss)]
    let spending_history = (spend_months as f64 / LAYER2_MIN_HISTORY_MONTHS as f64).clamp(0.0, 1.0);
    let spending_history_detail = if band_active {
        "Projected spending range is active.".to_owned()
    } else if spend_months >= LAYER2_MIN_HISTORY_MONTHS {
        // Enough categorized history, but it is on cards the liquid-only band does not model
        // yet — the range will include card spending once the band re-model ships (ADR 0050).
        "Your spending history is here — the projected range will soon include card spending."
            .to_owned()
    } else if spend_months > 0 {
        format!(
            "{} more month(s) of categorized spending builds your spending history.",
            LAYER2_MIN_HISTORY_MONTHS - spend_months
        )
    } else {
        "Categorize your spending to build your spending history.".to_owned()
    };

    // --- Actuals-backed recurrence (the actualization seam, §8 R3) — how much realized
    //     history backs the recurring forecast. ---
    let (recurrence_actuals, recurrence_actuals_detail) = recurrence_actuals_ratio(conn)?;

    // --- Backtest MAPE (the per-vault backtest, §18 R3) — how accurate past forecasts
    //     have actually been. ---
    let (backtest_mape, backtest_mape_detail) = backtest_mape_ratio(conn)?;

    let score = readiness_pct(
        READINESS_W_COVERAGE * coverage
            + READINESS_W_FRESHNESS * freshness
            + READINESS_W_EXPLAINED * explained
            + READINESS_W_CATEGORIZATION * categorization
            + READINESS_W_SPENDING_HISTORY * spending_history
            + READINESS_W_RECURRENCE_ACTUALS * recurrence_actuals
            + READINESS_W_BACKTEST_MAPE * backtest_mape,
    );

    Ok(ForecastReadiness {
        score,
        factors: vec![
            ReadinessFactor {
                key: "coverage".to_owned(),
                label: "Coverage".to_owned(),
                score: readiness_pct(coverage),
                detail: coverage_detail,
            },
            ReadinessFactor {
                key: "freshness".to_owned(),
                label: "Balance freshness".to_owned(),
                score: readiness_pct(freshness),
                detail: freshness_detail,
            },
            ReadinessFactor {
                key: "explained".to_owned(),
                label: "Explained activity".to_owned(),
                score: readiness_pct(explained),
                detail: explained_detail,
            },
            ReadinessFactor {
                key: "categorization".to_owned(),
                label: "Spending detail".to_owned(),
                score: readiness_pct(categorization),
                detail: categorization_detail,
            },
            ReadinessFactor {
                key: "spending_history".to_owned(),
                label: "Spending history".to_owned(),
                score: readiness_pct(spending_history),
                detail: spending_history_detail,
            },
            ReadinessFactor {
                key: "recurrence_actuals".to_owned(),
                label: "Verified accuracy".to_owned(),
                score: readiness_pct(recurrence_actuals),
                detail: recurrence_actuals_detail,
            },
            ReadinessFactor {
                key: "backtest_mape".to_owned(),
                label: "Forecast accuracy".to_owned(),
                score: readiness_pct(backtest_mape),
                detail: backtest_mape_detail,
            },
        ],
    })
}

/// A forecast capability that has self-activated and whose one-time unlock notice the user
/// has not yet acknowledged (ADR 0026 §10, personal-cfo-egon). The frontend renders one
/// dismissible notice per entry; dismissing it calls `acknowledge_capability`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityUnlock {
    /// Stable capability key (e.g. `forecast_band`) — also the acknowledgement scope.
    pub key: String,
    /// Headline shown to the user.
    pub title: String,
    /// One-line explanation of what unlocked and why.
    pub body: String,
    /// The readiness factor whose threshold unlocked this capability, so the notice can
    /// link back to it in the readiness card.
    pub factor_key: String,
}

/// A forecast capability gated by a readiness factor (ADR 0026 §8/§10). Static for now —
/// the Layer-2 band is the only probabilistic capability that exists; scenarios + insights
/// register here as they ship.
struct Capability {
    key: &'static str,
    title: &'static str,
    body: &'static str,
    /// The capability is active once this readiness factor is fully unlocked (score 100).
    factor_key: &'static str,
}

const CAPABILITIES: &[Capability] = &[
    Capability {
        key: "forecast_band",
        title: "Your forecast now shows a likely range",
        body: "You've recorded enough categorized spending history for the forecast to project a likely range, not just a single line.",
        factor_key: "spending_history",
    },
    // Announces itself via `history_depth_is_sufficient`, not a factor score; freshness
    // (balance evidence) is the closest readiness anchor for the notice's link — there
    // is no history-depth factor (personal-cfo-4d8.27.5.5).
    Capability {
        key: "cash_flow_history",
        title: "Your cash flow now shows its history",
        body: "You've recorded 30 days of real balance history, so the Cash Flow chart now shows where your money has actually been — behind where it's projected to go.",
        factor_key: "freshness",
    },
];

/// The `audit_events.event_type` recorded when a capability's unlock notice is acknowledged.
#[must_use]
pub(crate) fn capability_ack_event_type(key: &str) -> String {
    format!("capability_unlock_acknowledged:{key}")
}

/// Whether `key` names a capability the system knows about (guards acknowledgement writes).
#[must_use]
pub(crate) fn is_known_capability(key: &str) -> bool {
    CAPABILITIES.iter().any(|c| c.key == key)
}

/// Capabilities that have self-activated (their gating readiness factor is fully unlocked)
/// but whose one-time unlock notice the user has not yet acknowledged (ADR 0026 §10). The
/// "active and not acknowledged" rule means the notice fires once and stays dismissed.
pub(crate) fn pending_capability_unlocks(
    conn: &Connection,
    as_of: DateTime<Utc>,
) -> Result<Vec<CapabilityUnlock>, DbError> {
    let readiness = compute_forecast_readiness(conn, as_of)?;
    // The forecast_band capability gates on the ACTUAL liquid-only band activation, not the
    // card-inclusive spending_history indicator (ADR 0050 / §13a) — otherwise the unlock notice
    // would announce a projected range that is not drawn for a card-heavy vault
    // (personal-cfo-4d8.27.1.2).
    let band_active = band_is_active(conn, as_of)?;
    // cash_flow_history announces on realized-history depth, not a factor score
    // (an announcement, not a gate — the view renders honestly at any depth).
    let history_deep_enough = history_depth_is_sufficient(conn, as_of)?;
    let mut pending = Vec::new();
    for cap in CAPABILITIES {
        let active = match cap.key {
            "forecast_band" => band_active,
            "cash_flow_history" => history_deep_enough,
            _ => readiness
                .factors
                .iter()
                .any(|f| f.key == cap.factor_key && f.score >= 100),
        };
        if !active {
            continue;
        }
        let acked: i64 = conn.query_row(
            "SELECT COUNT(*) FROM audit_events WHERE event_type = ?1",
            params![capability_ack_event_type(cap.key)],
            |r| r.get(0),
        )?;
        if acked == 0 {
            pending.push(CapabilityUnlock {
                key: cap.key.to_owned(),
                title: cap.title.to_owned(),
                body: cap.body.to_owned(),
                factor_key: cap.factor_key.to_owned(),
            });
        }
    }
    Ok(pending)
}
