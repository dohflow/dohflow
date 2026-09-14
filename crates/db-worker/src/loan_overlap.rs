//! Detect a loan modeled **twice** in the liquid forecast (personal-cfo-6wk.11).
//!
//! A loan can drive the Future Cash forecast two disjoint ways with no data link
//! between them:
//! - as a `loan_liability` **account** with `debt_terms(payment_due_day)`, whose
//!   payment [`crate::forecast::collect_loan_payment_events`] projects as a liquid
//!   outflow; and
//! - as an active recurring **`loan_payment` bill** ([`crate::forecast::collect_bill_events`]),
//!   whose autopay account is the paying *liquid* source (not the loan).
//!
//! Tracking the same loan both ways double-counts its payment. The bill's autopay
//! is the liquid source, not the loan, so there is no foreign key to auto-dedupe on
//! — this module surfaces a **heuristic, descriptive** warning (ADR 0018) so the
//! user removes one. It never auto-removes either side.

use rusqlite::Connection;
use uuid::Uuid;

use crate::schedule_sources::active_obligation_schedules;
use crate::DbError;

/// A suspected double-count: a loan tracked as both a `loan_liability` account with
/// payment terms and an active recurring `loan_payment` bill that looks like the same
/// loan. Heuristic — `name_match` and/or `amount_match` say *why* it was flagged so
/// the UI can explain it; at least one is always true.
pub struct LoanDoubleCount {
    /// The `loan_liability` account.
    pub loan_account_id: Uuid,
    pub loan_name: String,
    /// The `recurring_events` row for the `loan_payment` bill.
    pub bill_event_id: Uuid,
    pub bill_name: String,
    /// The normalized names look like the same loan.
    pub name_match: bool,
    /// The loan's fixed monthly payment equals the bill amount (same currency).
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
        .filter(|s| {
            s.bill_type.as_deref() == Some("loan_payment")
                && s.currency_code.as_str() == currency.code()
        })
        .collect();

    let mut out = Vec::new();
    for loan in &loans {
        for bill in &bills {
            let name_match = names_similar(&loan.name, &bill.name);
            // A fixed monthly payment is the one loan amount knowable here without re-deriving the
            // schedule; matching it to the bill (same currency, guaranteed above) is a strong
            // same-loan signal.
            let amount_match = loan.philosophy == "pay_fixed_amount"
                && loan.fixed_amount_minor > 0
                && loan.fixed_amount_minor == bill.amount_minor;
            if name_match || amount_match {
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

#[cfg(test)]
mod tests {
    use super::{names_similar, normalize};

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
}
