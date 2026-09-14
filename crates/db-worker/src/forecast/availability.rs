//! Cash availability model: available / committed / headroom per liquid
//! account plus the household floor (moved verbatim from `forecast.rs`).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use core_money::{Currency, Money};
use rusqlite::Connection;
use uuid::Uuid;

use super::account_series::{compute_by_account, liquid_currency, read_liquid_accounts};
use super::aggregate::ForecastDayView;
use crate::DbError;

// ===== Cash availability model (ADR 0029, personal-cfo-fqbm) =====

/// The window over which "committed" outflows are summed (ADR 0029 §4).
const COMMITTED_WINDOW_DAYS: u32 = 30;

/// The five derived cash numbers for one liquid account (ADR 0029 §1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountAvailability {
    /// The liquid account id.
    pub account_id: Uuid,
    /// Display name.
    pub name: String,
    /// Canonical balance — the assertion-anchored balance (ADR 0027).
    pub ledger: Money,
    /// Uncleared holds (≥ 0). Zero in manual mode; connectors populate it later.
    pub pending: Money,
    /// `ledger − pending` — accessible right now.
    pub available: Money,
    /// Projected outflows over the next 30 days attributed to this account.
    pub committed: Money,
    /// `available − committed` — free after the next 30 days of bills.
    pub headroom: Money,
}

/// The household cash-availability snapshot (ADR 0029): the per-account numbers plus
/// the net rollup and the minimum-cash-floor status. Single-currency, derived on
/// read; only the floor is a persisted setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashAvailability {
    /// The single currency the numbers are in.
    pub currency: Currency,
    /// One entry per liquid account.
    pub accounts: Vec<AccountAvailability>,
    /// `Σ available` across liquid accounts.
    pub net_available: Money,
    /// `Σ committed` — every account's outflows plus un-attributable (Unallocated)
    /// outflows over the next 30 days.
    pub net_committed: Money,
    /// `net_available − net_committed` — the household safe-to-spend.
    pub net_headroom: Money,
    /// The household minimum-cash-floor setting (default 0).
    pub floor: Money,
    /// Whether net headroom is below the floor (the forward-looking alert).
    pub below_floor: bool,
}

/// Sum the outflow magnitudes (negative event amounts) across a per-account series.
fn committed_outflow(days: &[ForecastDayView]) -> Result<i64, DbError> {
    let mut total: i64 = 0;
    for day in days {
        for event in &day.events {
            let minor = event.amount.minor_units();
            if minor < 0 {
                total = total
                    .checked_sub(minor)
                    .ok_or_else(|| DbError::InvalidCommand("committed overflow".to_owned()))?;
            }
        }
    }
    Ok(total)
}

/// Compute the household cash-availability snapshot as of `as_of` (ADR 0029).
///
/// Reuses [`compute_by_account`] (a 30-day horizon) for the per-account committed
/// outflows and [`crate::assertion_anchored_balance`] for the ledger. `floor_minor`
/// is the household minimum-cash-floor setting.
///
/// # Errors
/// Returns [`DbError`] on a read failure, mixed-currency liquid accounts, or a
/// forecast arithmetic failure.
pub(crate) fn compute_cash_availability(
    conn: &Connection,
    as_of: DateTime<Utc>,
    floor_minor: i64,
) -> Result<CashAvailability, DbError> {
    let accounts = read_liquid_accounts(conn)?;
    let currency = liquid_currency(conn, &accounts)?;

    // Canonical ledger per account (assertion-anchored, ADR 0027).
    let mut ledger_by_id: HashMap<Uuid, i64> = HashMap::new();
    for account in &accounts {
        ledger_by_id.insert(
            account.id,
            crate::assertion_anchored_balance(conn, account.id, account.ledger_account_id)?,
        );
    }

    // Committed outflows over the next 30 days, attributed per account (ADR 0026 §12).
    let projection = compute_by_account(conn, as_of, COMMITTED_WINDOW_DAYS, &[])?;
    let overflow = || DbError::InvalidCommand("cash availability overflow".to_owned());

    let mut account_views = Vec::new();
    let mut net_available: i64 = 0;
    let mut net_committed: i64 = 0;
    for series in &projection.accounts {
        let committed = committed_outflow(&series.days)?;
        net_committed = net_committed.checked_add(committed).ok_or_else(overflow)?;
        // The Unallocated series (account_id None) feeds only net committed.
        let Some(account_id) = series.account_id else {
            continue;
        };
        let ledger = ledger_by_id.get(&account_id).copied().unwrap_or(0);
        let pending = 0; // manual mode: no holds (ADR 0029 §2)
        let available = ledger - pending;
        let headroom = available.checked_sub(committed).ok_or_else(overflow)?;
        net_available = net_available.checked_add(available).ok_or_else(overflow)?;
        account_views.push(AccountAvailability {
            account_id,
            name: series.name.clone(),
            ledger: Money::new(ledger, currency),
            pending: Money::new(pending, currency),
            available: Money::new(available, currency),
            committed: Money::new(committed, currency),
            headroom: Money::new(headroom, currency),
        });
    }

    let net_headroom = net_available
        .checked_sub(net_committed)
        .ok_or_else(overflow)?;
    Ok(CashAvailability {
        currency,
        accounts: account_views,
        net_available: Money::new(net_available, currency),
        net_committed: Money::new(net_committed, currency),
        net_headroom: Money::new(net_headroom, currency),
        floor: Money::new(floor_minor, currency),
        below_floor: net_headroom < floor_minor,
    })
}
