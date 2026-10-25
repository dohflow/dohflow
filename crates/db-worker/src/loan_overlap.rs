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
//! **Which bills are suspected** (planning's revised ruling on 6wk.20; the first draft, which let a
//! large unrelated bill near the due day count as "the same payment", was too loose, and exact amount
//! equality before it flickered for a minimum-payment loan and missed escrow). Every active bill is
//! considered, whatever its type: every creation path defaults the type to `subscription`, so a type
//! test alone hid the double count. A bill is a candidate only if it is MONTHLY and falls within
//! [`DUE_DAY_TOLERANCE`] days of the loan's due day. Two strengths, and only the first claims the bill
//! is "counted twice" or offers to link it:
//!
//! - **Strong** ([`MatchStrength::Strong`]) needs real evidence, either
//!   (i) the bill's payee is the payee of the loan's already-paired payments, with an amount of at
//!   least 95% of the scheduled payment and NO ceiling (escrow, extra principal), or
//!   (ii) the bill is typed `loan_payment` or `rent_mortgage`, paid from the loan's funding account
//!   (its terms' paying source, or the source of the payments already paired into the loan), with an
//!   amount from 95% to 200% of the scheduled payment.
//! - **Possible** ([`MatchStrength::Possible`]) is a soft "might be the same payment" note, never a
//!   link offer: paid from the funding account with an amount from 95% to 150%, or an amount within
//!   10% either way. When the funding account is the loan's DECLARED paying account the match
//!   carries [`MatchedBy::DeclaredTerms`]: still possible (never "counted twice"), but at the two
//!   interactive moments (a suggestion, a bill being created) it is enough to ASK the user whether
//!   it is the loan's payment.
//!
//! Anything else is no match. A name resemblance alone is not evidence.
//!
//! **Which amount.** The loan's payment is the one the forecast projects for it, for ANY repayment
//! philosophy; a pay-in-full loan projects a single payoff, not a monthly amount, so it has no
//! amount match.

use std::collections::{HashMap, HashSet};

use categorization::normalize_merchant;
use rusqlite::Connection;
use uuid::Uuid;

use crate::forecast::EmittingLoan;
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
    /// The bill amount fits the loan's derived monthly payment (same currency): at least 95% of it
    /// (no upper bound) or within 10% either way.
    pub amount_match: bool,
    /// How strongly the bill looks like the loan's payment.
    pub strength: MatchStrength,
    /// Which evidence made a STRONG match (`None` for a possible one), so the UI can say why.
    pub matched_by: Option<MatchedBy>,
    /// The bill's type token (`loan_payment`, `rent_mortgage`, ...), when it has a contract.
    pub bill_type: Option<String>,
    /// The name of the account the bill is paid from (the loan's funding account on the loan-typed
    /// route), when it has one.
    pub paid_from_account_name: Option<String>,
}

/// The evidence behind a match (planning's rulings on 6wk.20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchedBy {
    /// The bill carries the payee of the payments already paired into the loan.
    Payee,
    /// The bill is typed `loan_payment` or `rent_mortgage` and paid from the loan's funding account.
    LoanTyped,
    /// A POSSIBLE match only: paid from the loan's DECLARED paying account, monthly, within the due-day
    /// tolerance of its declared due day, at 95% to 150% of the payment. The user's own loan terms
    /// are evidence enough to ASK ("Is this the loan's payment?") where the user is interacting (a
    /// suggestion, a bill being created), never to ASSERT it is counted twice: a similar-size monthly
    /// bill near the due day, such as childcare, would be falsely called a duplicate. For an
    /// existing bill this stays the soft possible note.
    DeclaredTerms,
}

/// How strongly a bill looks like a loan's payment (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStrength {
    /// Real evidence (the payee of the loan's paired payments, or a loan-typed bill from its funding
    /// account) on the loan's cadence: the bill IS the loan's payment. The only tier that may say
    /// "counted twice" or offer a link.
    Strong,
    /// Some signals fit: a soft "might be the same payment" note, never a link offer.
    Possible,
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

/// A bill-shaped thing to compare with the loans' payments: an existing bill, a suggested recurring
/// candidate, or a bill the user is about to create (personal-cfo-6wk.20). One shape, one rule.
pub(crate) struct BillLike {
    pub name: String,
    /// The normalized merchant key (a promoted bill's persisted key, or a candidate's), if any.
    pub merchant_key: Option<String>,
    pub bill_type: Option<String>,
    pub amount_minor: i64,
    pub currency_code: String,
    pub freq_token: String,
    /// The day of the month it falls on, when it has a monthly cadence anchor.
    pub day_of_month: Option<u32>,
    pub paying_account: Option<Uuid>,
}

/// How a bill-shaped thing relates to one loan's payment.
struct Classified {
    strength: MatchStrength,
    matched_by: Option<MatchedBy>,
    name_match: bool,
    amount_match: bool,
}

/// The loans that emit a forecast payment, each with the payments already paired into it, loaded
/// once so many bill-shaped things can be compared without re-reading the ledger.
pub(crate) struct LoanContext {
    currency_code: String,
    loans: Vec<(EmittingLoan, PairedPayments)>,
}

pub(crate) fn load_loan_context(conn: &Connection) -> Result<LoanContext, DbError> {
    // Only loans that actually emit a forecast payment (in the forecast's fold currency, owed > 0)
    // can be double-counted — a paid-off or foreign-currency loan contributes nothing, so warning
    // about it would be false. `currency` is that fold currency; align the bills to it too.
    let (currency, loans) = crate::forecast::loans_emitting_payments(conn)?;
    let mut with_paired = Vec::with_capacity(loans.len());
    for loan in loans {
        let paired = paired_payments(conn, loan.terms.ledger_account_id)?;
        with_paired.push((loan, paired));
    }
    Ok(LoanContext {
        currency_code: currency.code().to_owned(),
        loans: with_paired,
    })
}

/// THE rule (see the module docs): how strongly `bill` looks like `emitting`'s payment, or `None`.
fn classify(
    emitting: &EmittingLoan,
    paired: &PairedPayments,
    bill: &BillLike,
) -> Option<Classified> {
    let loan = &emitting.terms;
    let payment = emitting.payment_minor;
    let name_match = names_similar(&loan.name, &bill.name);
    let loan_shaped = matches!(
        bill.bill_type.as_deref(),
        Some("loan_payment" | "rent_mortgage")
    );
    // A pay-in-full payoff is a single event: there is no monthly amount to compare.
    let comparable = emitting.recurring && payment > 0;
    let at_least_95 = comparable && at_least_percent(bill.amount_minor, payment, 95);
    let up_to_150 = at_least_95 && at_most_percent(bill.amount_minor, payment, 150);
    let up_to_200 = at_least_95 && at_most_percent(bill.amount_minor, payment, 200);
    let within_10 = comparable && within_percent(bill.amount_minor, payment, 10);
    let amount_match = at_least_95 || within_10;
    let date_ok = bill.freq_token == "monthly"
        && bill
            .day_of_month
            .is_some_and(|day| days_apart(day, loan.payment_due_day) <= DUE_DAY_TOLERANCE);
    let funded_by_the_loans_account = bill.paying_account.is_some_and(|account| {
        loan.paying_source == Some(account) || paired.sources.contains(&account)
    });
    let funded_by_the_declared_account = bill
        .paying_account
        .is_some_and(|account| loan.paying_source == Some(account));
    let payee_matches = paired_payee_matches(bill, paired);

    // The payee is checked first: it is the stronger evidence when both routes hold.
    let (strength, matched_by) = if !date_ok {
        return None;
    } else if payee_matches && at_least_95 {
        (MatchStrength::Strong, Some(MatchedBy::Payee))
    } else if loan_shaped && funded_by_the_loans_account && up_to_200 {
        (MatchStrength::Strong, Some(MatchedBy::LoanTyped))
    } else if funded_by_the_declared_account && up_to_150 {
        (MatchStrength::Possible, Some(MatchedBy::DeclaredTerms))
    } else if (funded_by_the_loans_account && up_to_150) || within_10 {
        (MatchStrength::Possible, None)
    } else {
        return None;
    };
    Some(Classified {
        strength,
        matched_by,
        name_match,
        amount_match,
    })
}

/// The loan a bill-shaped thing most looks like the payment of: the strongest match, the first loan
/// by name when tied. `None` when it matches no loan (or is in another currency than the forecast).
pub(crate) fn best_loan_match(context: &LoanContext, bill: &BillLike) -> Option<LoanPaymentMatch> {
    if bill.currency_code != context.currency_code {
        return None;
    }
    let mut best: Option<LoanPaymentMatch> = None;
    for (emitting, paired) in &context.loans {
        let Some(found) = classify(emitting, paired, bill) else {
            continue;
        };
        let better = best.as_ref().is_none_or(|b| {
            b.strength == MatchStrength::Possible && found.strength == MatchStrength::Strong
        });
        if better {
            best = Some(LoanPaymentMatch {
                loan_account_id: emitting.terms.account_id,
                loan_name: emitting.terms.name.clone(),
                strength: found.strength,
                matched_by: found.matched_by,
            });
        }
    }
    best
}

/// A loan a bill-shaped thing looks like the payment of, for the suggestion list and the
/// creation-time check (personal-cfo-6wk.20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPaymentMatch {
    pub loan_account_id: Uuid,
    pub loan_name: String,
    pub strength: MatchStrength,
    pub matched_by: Option<MatchedBy>,
}

/// A bill the user is about to create, for the creation-time check.
pub struct LoanOverlapDraft {
    pub name: String,
    pub merchant_key: Option<String>,
    pub bill_type: String,
    pub amount: core_money::Money,
    pub frequency: pay_schedule::Frequency,
    pub anchor: chrono::NaiveDate,
    pub autopay_account_id: Option<Uuid>,
}

/// Does the bill the user is about to create look like a loan's payment? Read-only: nothing is
/// created, changed or linked (ADR 0018).
///
/// # Errors
/// Returns [`DbError`] if a read fails.
pub(crate) fn check_draft(
    conn: &Connection,
    draft: &LoanOverlapDraft,
) -> Result<Option<LoanPaymentMatch>, DbError> {
    use chrono::Datelike;
    let context = load_loan_context(conn)?;
    let bill = BillLike {
        name: draft.name.clone(),
        merchant_key: draft.merchant_key.clone(),
        bill_type: Some(draft.bill_type.clone()),
        amount_minor: draft.amount.minor_units(),
        currency_code: draft.amount.currency().code().to_owned(),
        freq_token: draft.frequency.token(),
        day_of_month: Some(draft.anchor.day()),
        paying_account: draft.autopay_account_id,
    };
    Ok(best_loan_match(&context, &bill))
}

/// Every suspected loan double-count (see the module docs). Deterministic order:
/// loans by name, then bills by name (both readers already order that way).
///
/// # Errors
/// Returns [`DbError`] if a read fails.
pub(crate) fn detect_loan_double_counts(
    conn: &Connection,
) -> Result<Vec<LoanDoubleCount>, DbError> {
    let context = load_loan_context(conn)?;
    if context.loans.is_empty() {
        return Ok(Vec::new());
    }
    let merchant_keys = bill_merchant_keys(conn)?;
    let bills = active_obligation_schedules(conn)?;

    let mut out = Vec::new();
    for (emitting, paired) in &context.loans {
        let loan = &emitting.terms;
        for bill in &bills {
            // Resolved or answered (personal-cfo-6wk.22): a bill that is LINKED to a loan is that
            // loan's payment and the forecast already counts it once, so it is no overlap with any
            // loan; and a bill the user said is NOT this loan's payment is skipped for this loan.
            if bill.linked_loan_account_id.is_some()
                || bill.not_payment_of_loan_account_id == Some(loan.account_id)
            {
                continue;
            }
            let like = BillLike {
                name: bill.name.clone(),
                merchant_key: merchant_keys.get(&bill.id).cloned(),
                bill_type: bill.bill_type.clone(),
                amount_minor: bill.amount_minor,
                currency_code: bill.currency_code.clone(),
                freq_token: bill.freq_token.clone(),
                day_of_month: bill.anchor.as_deref().and_then(day_of_month),
                paying_account: bill.autopay_account_id,
            };
            if like.currency_code != context.currency_code {
                continue;
            }
            let Some(found) = classify(emitting, paired, &like) else {
                continue;
            };
            let paid_from_account_name = match bill.autopay_account_id {
                Some(account) => account_name(conn, account)?,
                None => None,
            };
            out.push(LoanDoubleCount {
                loan_account_id: loan.account_id,
                loan_name: loan.name.clone(),
                bill_event_id: bill.id,
                bill_name: bill.name.clone(),
                name_match: found.name_match,
                amount_match: found.amount_match,
                strength: found.strength,
                matched_by: found.matched_by,
                bill_type: bill.bill_type.clone(),
                paid_from_account_name,
            });
        }
    }
    Ok(out)
}

/// The display name of an account, if it exists.
fn account_name(conn: &Connection, account: Uuid) -> Result<Option<String>, DbError> {
    let mut stmt = conn.prepare("SELECT name FROM accounts WHERE id = ?1")?;
    let mut rows = stmt.query([account])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

/// `amount` is at least `percent`% of `of`, with no upper bound.
fn at_least_percent(amount: i64, of: i64, percent: i128) -> bool {
    i128::from(amount) * 100 >= i128::from(of) * percent
}

/// `amount` is at most `percent`% of `of`.
fn at_most_percent(amount: i64, of: i64, percent: i128) -> bool {
    i128::from(amount) * 100 <= i128::from(of) * percent
}

/// `amount` is within `percent`% of `of`, either way.
fn within_percent(amount: i64, of: i64, percent: i128) -> bool {
    (i128::from(amount) - i128::from(of)).abs() * 100 <= i128::from(of) * percent
}

/// The funding accounts and payees of the transactions already paired as payments INTO a loan
/// (the most recent few), read from the ledger: a payment is a positive posting on the loan's ledger
/// account whose other leg debits a liquid account.
#[derive(Default)]
struct PairedPayments {
    /// `accounts.id` of the liquid accounts the payments came from.
    sources: HashSet<Uuid>,
    /// Normalized payees of those payments (the merchant key form the recurring detector uses).
    payees: HashSet<String>,
}

/// How many recent payments into a loan are read to learn where it is paid from and to whom.
const PAIRED_PAYMENTS_READ: usize = 24;

fn paired_payments(
    conn: &Connection,
    loan_ledger_account: Uuid,
) -> Result<PairedPayments, DbError> {
    let mut stmt = conn.prepare(
        "SELECT a.id, td.counterparty
           FROM ledger_postings loan_leg
           JOIN ledger_transactions lt ON lt.id = loan_leg.transaction_id AND lt.voided_at IS NULL
           JOIN ledger_postings payer_leg ON payer_leg.transaction_id = lt.id
                AND payer_leg.ledger_account_id <> loan_leg.ledger_account_id
                AND payer_leg.minor_units < 0
           JOIN accounts a ON a.ledger_account_id = payer_leg.ledger_account_id
                AND a.cashflow_role = 'liquid_cash'
           LEFT JOIN transaction_details td ON td.transaction_id = lt.id
          WHERE loan_leg.ledger_account_id = ?1 AND loan_leg.minor_units > 0
          ORDER BY lt.occurred_at DESC
          LIMIT ?2",
    )?;
    let mut paired = PairedPayments::default();
    let rows = stmt.query_map(
        rusqlite::params![loan_ledger_account, PAIRED_PAYMENTS_READ as i64],
        |r| Ok((r.get::<_, Uuid>(0)?, r.get::<_, Option<String>>(1)?)),
    )?;
    for row in rows {
        let (account, payee) = row?;
        paired.sources.insert(account);
        if let Some(key) = payee
            .map(|p| normalize_merchant(&p))
            .filter(|k| !k.is_empty())
        {
            paired.payees.insert(key);
        }
    }
    Ok(paired)
}

/// Each bill's persisted merchant key (set when it was promoted from a detected candidate).
fn bill_merchant_keys(conn: &Connection) -> Result<HashMap<Uuid, String>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, source_merchant_key FROM recurring_events
          WHERE source_merchant_key IS NOT NULL AND is_active = 1 AND deleted_at IS NULL",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, Uuid>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

/// The bill carries the payee of the loan's paired payments: its merchant key, or its own name,
/// normalized the same way.
fn paired_payee_matches(bill: &BillLike, paired: &PairedPayments) -> bool {
    if paired.payees.is_empty() {
        return false;
    }
    let by_key = bill
        .merchant_key
        .as_ref()
        .is_some_and(|key| paired.payees.contains(key));
    by_key || paired.payees.contains(&normalize_merchant(&bill.name))
}

/// How many days either side of the loan's due day a bill's day-of-month may fall and still count as
/// "the same payment".
const DUE_DAY_TOLERANCE: u32 = 7;

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
