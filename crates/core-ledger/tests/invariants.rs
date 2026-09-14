//! Property tests for the double-entry invariant (plan DoD §1.5, §4).

use chrono::{DateTime, Utc};
use core_ledger::{
    LedgerAccountId, LedgerError, LedgerTransaction, OperationId, Posting, TransactionId,
};
use core_money::{Currency, Money};
use proptest::prelude::*;

fn at_epoch() -> DateTime<Utc> {
    DateTime::from_timestamp(0, 0).expect("epoch is valid")
}

fn usd(units: i64) -> Posting {
    Posting::new(LedgerAccountId::new(), Money::new(units, Currency::Usd))
}

proptest! {
    /// Any set of postings that sums to zero (in one currency) always
    /// constructs successfully and reports the right shape.
    #[test]
    fn balanced_postings_always_construct(
        amounts in prop::collection::vec(-1_000_000_000i64..1_000_000_000, 1..8),
    ) {
        let balancer: i64 = amounts.iter().sum();
        let mut postings: Vec<Posting> = amounts.iter().map(|&a| usd(a)).collect();
        postings.push(usd(-balancer)); // forces the set to sum to zero

        let tx = LedgerTransaction::new(
            TransactionId::new(),
            OperationId::new(),
            at_epoch(),
            postings.clone(),
        );
        prop_assert!(tx.is_ok());
        let tx = tx.unwrap();
        prop_assert_eq!(tx.postings().len(), postings.len());
        prop_assert_eq!(tx.currency(), Currency::Usd);
    }

    /// A non-zero residual is always rejected as unbalanced.
    #[test]
    fn corrupted_postings_fail_validation(
        a in 1i64..1_000_000_000,
        b in 1i64..1_000_000_000,
    ) {
        // Both positive => sum is strictly positive => cannot balance.
        let postings = vec![usd(a), usd(b)];
        let tx = LedgerTransaction::new(
            TransactionId::new(),
            OperationId::new(),
            at_epoch(),
            postings,
        );
        // Bind to a bool: `prop_assert!` stringifies its argument, and a brace
        // pattern inside the macro is misread as a format placeholder.
        let is_unbalanced = matches!(tx, Err(LedgerError::Unbalanced { .. }));
        prop_assert!(is_unbalanced);
    }

    /// A same-currency two-account transfer balances exactly for any amount.
    #[test]
    fn transfer_balances_exactly(amount in (i64::MIN + 1)..=i64::MAX) {
        let postings = vec![usd(amount), usd(-amount)];
        let tx = LedgerTransaction::new(
            TransactionId::new(),
            OperationId::new(),
            at_epoch(),
            postings,
        );
        prop_assert!(tx.is_ok());
    }

    /// Mixing currencies is always rejected, regardless of amounts.
    #[test]
    fn mixed_currency_always_rejected(amount in (i64::MIN + 1)..=i64::MAX) {
        let postings = vec![
            Posting::new(LedgerAccountId::new(), Money::new(amount, Currency::Usd)),
            Posting::new(LedgerAccountId::new(), Money::new(-amount, Currency::Eur)),
        ];
        let tx = LedgerTransaction::new(
            TransactionId::new(),
            OperationId::new(),
            at_epoch(),
            postings,
        );
        let is_mixed = matches!(tx, Err(LedgerError::MixedCurrency { .. }));
        prop_assert!(is_mixed);
    }
}
