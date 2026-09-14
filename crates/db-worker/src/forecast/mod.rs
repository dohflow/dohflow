//! Future Cash forecast input adapter (plan §13.3, personal-cfo-164u).
//!
//! Bridges canonical vault state to the pure [`forecast_engine`]. It reads the
//! household timezone, the liquid-cash opening balance, the active income
//! sources, and the in-forecast recurring obligations; expands each schedule
//! into the engine's dated event stream via [`PaySchedule`]; and folds it with
//! [`forecast_layer1`]. The wall clock is read by the caller ([`crate::DbWorker::
//! future_cash_forecast`]) and passed in as `as_of`, so this module performs no
//! IO beyond the read queries and stays free of `now()`.
//!
//! **Why canonical tables, not the `commitments` projection:** the forecast does
//! signed money math, so it needs each obligation's *currency*. The commitments
//! read model intentionally drops currency (it is an obligation index, not a
//! money ledger), so the adapter reads `recurring_events` + `income_sources`
//! directly — both carry an explicit `currency`, `frequency`, and anchor. The
//! `include_in_forecast` / `active` flags select exactly the rows the forecast
//! should see.

use chrono::NaiveDate;
use pay_schedule::Frequency;

use crate::DbError;

mod account_series;
mod aggregate;
mod availability;
mod card_cycles;
mod events;
mod payoff;
mod readiness;

pub(crate) use account_series::{compute_by_account, compute_cash_flow_history};
pub use account_series::{
    AccountHistoryView, AccountSeriesView, CashFlowHistory, DayBalance, GroupSeriesView,
    HistoryDay, MultiSeriesForecast,
};
pub(crate) use aggregate::{compute, read_variable_spend_history, variable_spend_points};
pub use aggregate::{ForecastDayView, ForecastEventView, ForecastView};
pub(crate) use availability::compute_cash_availability;
pub use availability::{AccountAvailability, CashAvailability};
pub(crate) use card_cycles::{
    card_statement_forecast, card_statement_history, card_statement_walk_forward_pairs,
    loans_emitting_payments,
};
pub use card_cycles::{
    CardCycleView, CardStatementForecastView, CardStatementHistoryView, StoredStatementView,
};
pub(crate) use events::{
    household_today, read_household_tz, reporting_currency, write_household_tz,
};
// Only exercised by tests today (`household_today` is the production entry point); gated so
// a non-test build doesn't warn on it as unused.
#[cfg(test)]
pub(crate) use events::household_today_at;
pub(crate) use payoff::debt_payoff_comparison;
pub use payoff::{PayoffDebtSeries, PayoffPlanView};
pub(crate) use readiness::{
    capability_ack_event_type, compute_forecast_readiness, is_known_capability,
    pending_capability_unlocks,
};
pub use readiness::{CapabilityUnlock, ForecastReadiness, ReadinessFactor};

pub(crate) fn parse_frequency(token: &str, what: &str) -> Result<Frequency, DbError> {
    Frequency::from_token(token)
        .ok_or_else(|| DbError::InvalidCommand(format!("unknown {what} frequency token: {token}")))
}

pub(crate) fn parse_date(raw: &str) -> Result<NaiveDate, DbError> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|e| DbError::InvalidCommand(e.to_string()))
}
