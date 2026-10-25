//! Detect a loan modeled **twice** in the liquid forecast (personal-cfo-6wk.11, widened by
//! personal-cfo-6wk.20).
//!
//! A loan can drive the Future Cash forecast two disjoint ways with no data link
//! between them:
//! - as a `loan_liability` **account** with `debt_terms(payment_due_day)`, whose
//!   payment [`crate::forecast::collect_loan_payment_events`] projects as a liquid
//!   outflow; and
//! - as an active recurring **bill** of ANY type ([`crate::forecast::collect_bill_events`]),
//!   whose autopay account is the paying *liquid* source (not the loan).
//!
//! Tracking the same loan both ways double-counts its payment. The bill's autopay is the liquid
//! source, not the loan, so there is no foreign key to auto-dedupe on — this module surfaces a
//! **heuristic, descriptive** warning (ADR 0018) so the user removes one. It never auto-removes
//! either side.
//!
//! **Which bills are suspected.** Every active bill, not only `loan_payment`-typed ones: every
//! creation path (Add bill, Make recurring, promoting a suggestion) defaults the type to
//! `subscription`, so a type test alone hid the double count it was meant to catch (6wk.20).
//! A `loan_payment` or `rent_mortgage` bill is suspected on the name or the amount alone, as
//! before. Any other bill needs corroboration from its SCHEDULE as well: it must be monthly and
//! either fall within a few days of the loan's due day or be paid from the loan's paying source,
//! and match the loan by name or by the loan's derived payment amount. That keeps an unrelated
//! subscription that happens to share a word with a loan's name from being flagged.
//!
//! **Which amount.** The loan's payment is the one the forecast projects for it, for ANY
//! repayment philosophy (a minimum-payment loan's derived minimum is comparable to a bill too);
//! a pay-in-full loan projects a single payoff, not a monthly amount, so it has no amount match.

use rusqlite::Connection;
use uuid::Uuid;

use crate::schedule_sources::active_obligation_schedules;
use crate::DbError;

/// A suspected double-count: a loan tracked as both a `loan_liability` account with
/// payment terms and an active recurring bill (of any type, 6wk.20) that looks like the same
/// loan. Heuristic — `name_match` and/or `amount_match` say *why* it was flagged so
/// the UI can explain it; at least one is always true.
pub struct LoanDoubleCount {
    /// The `loan_liability` account.
    pub loan_account_id: Uuid,
    pub loan_name: String,
    /// The `recurring_events` row for the bill.
    pub bill_event_id: Uuid,
    pub bill_name: String,
    /// The normalized names look like the same loan.
    pub name_match: bool,
    /// The monthly payment the forecast derives for the loan equals the bill amount (same currency).
    pub amount_match: bool,
}

/// Generic loan/bill words that are *not* distinctive enough to call two entities the same
/// loan on their own — so "Loan" (an account) does not match every `…loan…` bill.
const GENERIC_NAME_TOKENS: &[&str] = &[
    "loan", "loans", "payment", "payments", "auto", "car", "card", "bill", "debt", "monthly",
];

/// A name reduced to its lowercase alphanumeric core, for fuzzy comparison
/// ("Car Loan" and "Car-Loan Payment" both contain "carloan").
fn normalize(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

/// Two names look like the same loan: one normalized name contains the other, the shorter is at
/// least 3 chars (so a trivial 1–2 char overlap does not match), and the shorter is not a bare
/// generic term (so a loan account literally named "Loan" does not match every loan bill).
fn names_similar(a: &str, b: &str) -> bool {
    let (na, nb) = (normalize(a), normalize(b));
    let (short, long) = if na.len() <= nb.len() {
        (&na, &nb)
    } else {
        (&nb, &na)
    };
    short.len() >= 3
        && !GENERIC_NAME_TOKENS.contains(&short.as_str())
        && long.contains(short.as_str())
}

/// Every suspected loan double-count (see the module docs). Deterministic order:
/// loans by name, then bills by name (both readers already order that way).
///
/// # Errors
/// Returns [`DbError`] if a read fails.
pub(crate) fn detect_loan_double_counts(
    conn: &Connection,
) -> Result<Vec<LoanDoubleCount>, DbError> {
    // Only loans that actually emit a forecast payment (in the forecast's fold currency, owed > 0)
    // can be double-counted — a paid-off or foreign-currency loan contributes nothing, so warning
    // about it would be false. `currency` is that fold currency; align the bills to it too.
    let (currency, loans) = crate::forecast::loans_emitting_payments(conn)?;
    if loans.is_empty() {
        return Ok(Vec::new());
    }
    let bills: Vec<_> = active_obligation_schedules(conn)?
        .into_iter()
        .filter(|s| s.currency_code.as_str() == currency.code())
        .collect();

    let mut out = Vec::new();
    for emitting in &loans {
        let loan = &emitting.terms;
        for bill in &bills {
            let name_match = names_similar(&loan.name, &bill.name);
            // The forecast's own payment for this loan, compared to the bill (same currency,
            // guaranteed above). A recurring monthly amount is the one loan amount that is
            // comparable to a bill; a pay-in-full payoff is a single event.
            let amount_match = emitting.recurring && emitting.payment_minor == bill.amount_minor;
            if !(name_match || amount_match) {
                continue;
            }
            // `loan_payment` and `rent_mortgage` bills are loan-shaped by their own type, so the
            // name or amount alone suffices. Any other type needs its schedule to line up too.
            let loan_shaped = matches!(
                bill.bill_type.as_deref(),
                Some("loan_payment" | "rent_mortgage")
            );
            if loan_shaped || schedule_matches(loan, bill) {
                out.push(LoanDoubleCount {
                    loan_account_id: loan.account_id,
                    loan_name: loan.name.clone(),
                    bill_event_id: bill.id,
                    bill_name: bill.name.clone(),
                    name_match,
                    amount_match,
                });
            }
        }
    }
    Ok(out)
}

/// How many days either side of the loan's due day a bill's day-of-month may fall and still count as
/// "the same payment".
const DUE_DAY_TOLERANCE: u32 = 3;

/// The bill pays on the loan's schedule: monthly, and either on (about) the loan's due day or from
/// the account the loan is paid from.
fn schedule_matches(
    loan: &crate::forecast::LoanPaymentTerms,
    bill: &crate::schedule_sources::ScheduleSource,
) -> bool {
    if bill.freq_token != "monthly" {
        return false;
    }
    let near_due_day = bill
        .anchor
        .as_deref()
        .and_then(day_of_month)
        .is_some_and(|day| days_apart(day, loan.payment_due_day) <= DUE_DAY_TOLERANCE);
    let same_source = loan.paying_source.is_some() && loan.paying_source == bill.autopay_account_id;
    near_due_day || same_source
}

/// The day of the month in a `YYYY-MM-DD` date.
fn day_of_month(date: &str) -> Option<u32> {
    date.get(8..10)?.parse().ok()
}

/// Distance between two days of the month, wrapping across the month end (the 31st is two days
/// from the 2nd).
fn days_apart(a: u32, b: u32) -> u32 {
    let direct = a.abs_diff(b);
    direct.min(31 - direct)
}

#[cfg(test)]
mod tests {
    use super::{day_of_month, days_apart, names_similar, normalize};

    #[test]
    fn normalize_strips_case_and_punctuation() {
        assert_eq!(normalize("Car-Loan Payment"), "carloanpayment");
        assert_eq!(normalize("  Toyota  "), "toyota");
    }

    #[test]
    fn similar_names_match_when_one_contains_the_other() {
        assert!(names_similar("Car Loan", "Car-Loan Payment"));
        assert!(names_similar("Toyota", "Toyota Corolla"));
        // Same core, different formatting.
        assert!(names_similar("student loan", "Student Loan"));
    }

    #[test]
    fn unrelated_or_trivial_names_do_not_match() {
        assert!(!names_similar("Car Loan", "Mortgage"));
        // A 1–2 char overlap is not enough to call it the same loan.
        assert!(!names_similar("A1", "A1 something else"));
        assert!(!names_similar("", "anything"));
    }

    #[test]
    fn a_bare_generic_name_does_not_match_every_loan_bill() {
        // A loan account literally named "Loan" must not name-match every `…loan…` bill.
        assert!(!names_similar("Loan", "Student Loan Autopay"));
        assert!(!names_similar("Loan", "Car Loan Payment"));
        assert!(!names_similar("Payment", "Car Loan Payment"));
        // But a distinctive shared core still matches.
        assert!(names_similar("Student Loan", "Student Loan Autopay"));
    }

    #[test]
    fn days_apart_wraps_across_the_month_end() {
        assert_eq!(days_apart(15, 15), 0);
        assert_eq!(days_apart(15, 18), 3);
        assert_eq!(days_apart(1, 15), 14);
        assert_eq!(days_apart(31, 2), 2);
        assert_eq!(days_apart(30, 1), 2);
    }

    #[test]
    fn the_day_of_month_comes_from_an_iso_date() {
        assert_eq!(day_of_month("2026-07-15"), Some(15));
        assert_eq!(day_of_month("2026-07-01"), Some(1));
        assert_eq!(day_of_month("2026-07"), None);
        assert_eq!(day_of_month("bogus"), None);
    }
}
