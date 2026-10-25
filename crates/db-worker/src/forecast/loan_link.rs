//! The bill-to-loan link in the forecast (personal-cfo-6wk.22; ADR 0035 addendum 2026-10-25).
//!
//! A recurring bill may be linked to the loan it pays. While the link is **in force** the bill
//! carries the cash at its observed amount (escrow and fees included) and the loan's own
//! due-date row is not emitted ([`loans_with_link_in_force`], used by
//! `collect_loan_payment_events`). The loan still amortizes by its own terms: nothing here touches
//! the debt views.
//!
//! A link is in force only while ALL of these hold, so cash never silently vanishes: the bill is
//! active, included in the forecast and not deleted (a link is also cleared when the bill is
//! archived or deleted, so this is belt and braces); the bill is in the loan's currency; the loan
//! projects a **recurring** payment (never a pay-in-full payoff); and the bill's cadence equals the
//! loan's (monthly). When any stops holding the loan's row returns in the same forecast with no
//! user action. [`link_status`] says which, for the user.

use std::collections::HashSet;

use rusqlite::Connection;
use uuid::Uuid;

use super::card_cycles::{read_loans_with_payment, scheduled_payment};
use crate::DbError;

/// The cadence of a loan's own payment projection (fixed and minimum payments are monthly).
const LOAN_CADENCE: &str = "monthly";

/// A linked bill's amount below this share of the loan's scheduled payment keeps the link but is
/// warned about (the amount stays the user's truth).
const LOW_AMOUNT_PERCENT: i128 = 95;

/// A bill that carries a link, as stored.
pub(crate) struct LinkedBill {
    pub amount_minor: i64,
    pub currency: String,
    pub frequency: String,
    pub include_in_forecast: bool,
    pub loan_account_id: Uuid,
}

/// Every bill that carries a link and is neither deleted nor archived.
pub(crate) fn read_linked_bills(conn: &Connection) -> Result<Vec<LinkedBill>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT amount_expected_minor, currency, frequency, include_in_forecast,
                linked_loan_account_id
           FROM recurring_events
          WHERE linked_loan_account_id IS NOT NULL AND is_active = 1 AND deleted_at IS NULL
          ORDER BY id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LinkedBill {
                amount_minor: r.get(0)?,
                currency: r.get(1)?,
                frequency: r.get(2)?,
                include_in_forecast: r.get::<_, i64>(3)? != 0,
                loan_account_id: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// How a bill's link stands, and what to tell the user about it.
pub struct LinkStatus {
    /// The loan the bill is linked to.
    pub loan_account_id: Uuid,
    /// That loan's name (empty if the account cannot be read).
    pub loan_name: String,
    /// The loan's own payment is currently carried by this bill.
    pub in_force: bool,
    /// The bill no longer repeats like the loan's payment (not monthly), so BOTH now count.
    pub cadence_changed: bool,
    /// The bill is below 95% of the loan's scheduled payment (the link stays; the user is warned).
    pub below_scheduled: bool,
    /// The payment the forecast derives for the loan, when it projects a recurring one.
    pub scheduled_payment_minor: Option<i64>,
    /// The part of the bill beyond the loan's scheduled payment (escrow, fees, extra principal),
    /// shown as the split; zero unless the link is in force and the bill is larger.
    pub other_minor: i64,
}

fn loan_name(conn: &Connection, loan: Uuid) -> Result<String, DbError> {
    Ok(conn
        .query_row("SELECT name FROM accounts WHERE id = ?1", [loan], |r| {
            r.get(0)
        })
        .unwrap_or_default())
}

/// The status of one linked bill's link.
pub(crate) fn link_status(conn: &Connection, bill: &LinkedBill) -> Result<LinkStatus, DbError> {
    let name = loan_name(conn, bill.loan_account_id)?;
    // The loan's derived recurring payment, in the bill's currency (an archived loan, or one with
    // no payment terms, is not returned by the reader).
    let recurring_payment = match read_loans_with_payment(conn)?
        .into_iter()
        .find(|l| l.account_id == bill.loan_account_id && l.currency.code() == bill.currency)
    {
        Some(terms) => match scheduled_payment(conn, &terms)? {
            Some((amount, true)) => Some(amount),
            _ => None,
        },
        None => None,
    };
    let cadence_changed = bill.frequency != LOAN_CADENCE;
    let in_force = bill.include_in_forecast && !cadence_changed && recurring_payment.is_some();
    let below_scheduled = in_force
        && recurring_payment.is_some_and(|p| {
            p > 0 && i128::from(bill.amount_minor) * 100 < i128::from(p) * LOW_AMOUNT_PERCENT
        });
    let other_minor = match recurring_payment {
        Some(p) if in_force && bill.amount_minor > p => bill.amount_minor - p,
        _ => 0,
    };
    Ok(LinkStatus {
        loan_account_id: bill.loan_account_id,
        loan_name: name,
        in_force,
        cadence_changed,
        below_scheduled,
        scheduled_payment_minor: recurring_payment,
        other_minor,
    })
}

/// The loans whose own due-date payment is carried by a linked bill right now.
pub(crate) fn loans_with_link_in_force(conn: &Connection) -> Result<HashSet<Uuid>, DbError> {
    let mut out = HashSet::new();
    for bill in read_linked_bills(conn)? {
        if link_status(conn, &bill)?.in_force {
            out.insert(bill.loan_account_id);
        }
    }
    Ok(out)
}
