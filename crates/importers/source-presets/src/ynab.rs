//! YNAB (personal-cfo-gvidg / personal-cfo-tulv) — the first preset landed
//! on the harness (AC #3). YNAB's **Register export** (Budget name menu →
//! Export Budget, or Export Transactions for a subset — web app only, not
//! the mobile app) is a CSV spanning every account in one file, with a
//! two-level category (Category Group + Category) and separate
//! Outflow/Inflow columns rather than one signed amount.

use importer_core::{
    AccountHandling, CategoryHandling, ColumnMapping, ParserHints, SignConvention, SourcePreset,
};

pub struct Ynab;

impl SourcePreset for Ynab {
    fn id(&self) -> &'static str {
        "ynab"
    }

    fn display_name(&self) -> &'static str {
        "YNAB"
    }

    fn source_app_url(&self) -> &'static str {
        "https://www.ynab.com"
    }

    fn hints(&self) -> ParserHints {
        ParserHints {
            column_mapping: Some(ColumnMapping {
                date: Some("Date".to_owned()),
                description: Some("Payee".to_owned()),
                amount: None,
                debit: Some("Outflow".to_owned()),
                credit: Some("Inflow".to_owned()),
                account: Some("Account".to_owned()),
                category: Some("Category".to_owned()),
                category_group: Some("Category Group".to_owned()),
                currency: None,
                memo: Some("Memo".to_owned()),
            }),
            date_format: Some("%m/%d/%Y".to_owned()),
            default_currency: None,
            institution: Some("YNAB".to_owned()),
        }
    }

    fn sign_convention(&self) -> SignConvention {
        SignConvention::SeparateOutflowInflow
    }

    fn category_handling(&self) -> CategoryHandling {
        CategoryHandling::GroupAndCategory
    }

    fn account_handling(&self) -> AccountHandling {
        // The Register export spans every account in one file — see this
        // preset's SCOPE NOTE at the SourcePreset trait for what
        // AccountColumn does and does not do yet.
        AccountHandling::AccountColumn
    }

    fn quirks(&self) -> &'static [importer_core::SourceQuirk] {
        use importer_core::SourceQuirk::{MemoColumn, PendingFlag, TransferRows};
        &[MemoColumn, PendingFlag, TransferRows]
    }

    fn verified_against(&self) -> &'static str {
        // Cross-referenced against public documentation of YNAB's Register
        // export column set (Account, Date, Payee, Category Group, Category,
        // Memo, Outflow, Inflow, Cleared) -- support.ynab.com's own export
        // guides plus community migration tooling (wiki.gnucash.org/wiki/
        // YNAB_Migration) -- 2026-09-19. Not fetched from a live export this
        // session; re-verify against a real Register export before this
        // preset's guide (personal-cfo-tulv) claims an end-to-end result.
        "YNAB Register export column set, cross-referenced 2026-09-19 -- see source comment"
    }

    fn help_slug(&self) -> &'static str {
        "move-from-ynab"
    }

    fn help_published(&self) -> bool {
        // dohflow-site's src/content/migrate/move-from-ynab.md is `draft:
        // true` as of this writing, per the owner's explicit product-
        // sequencing call in personal-cfo-y0o0x's PR #48 review thread:
        // "build out robust migration mechanisms in the app first, then we
        // can come out with these." This preset (the mechanism) shipping is
        // exactly what unblocks that "then" -- but publishing the guide
        // itself is still a separate, owner/tulv decision. Flip to `true`
        // only once that file's draft flag actually flips on dohflow-site
        // main (personal-cfo-gvidg review finding F1, PR #15).
        false
    }

    fn fixture_csv(&self) -> &'static str {
        // Synthesized, never a real export. Exercises: two distinct
        // accounts (AccountColumn), a category group + category combining,
        // separate Outflow/Inflow columns, a thousands-separator amount, a
        // memo column (unmapped today -- see the crate-level note), and an
        // unmapped Cleared column (harmlessly ignored, still present in
        // normalized_json).
        "Account,Flag,Date,Payee,Category Group,Category,Memo,Outflow,Inflow,Cleared\n\
         Checking,,06/01/2026,Landlord LLC,Immediate Obligations,Rent,June rent,\"1,200.00\",,Cleared\n\
         Checking,,06/02/2026,Employer Inc,Income,Ready to Assign,Paycheck,,2500.00,Cleared\n\
         Credit Card,,06/03/2026,Grocery Co,Everyday Expenses,,,84.20,,Uncleared\n"
    }
}

importer_core::register_preset!(Ynab);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::parse_fixture;
    use core_money::{Currency, Money};

    #[test]
    fn ynab_fixture_captures_date_sign_description_category_and_account() {
        let batch = parse_fixture(&Ynab);
        assert_eq!(batch.records.len(), 3);

        // Two distinct accounts staged (AccountHandling::AccountColumn).
        let mut account_names: Vec<_> = batch
            .accounts
            .iter()
            .map(|a| a.external_name.as_deref().unwrap())
            .collect();
        account_names.sort_unstable();
        assert_eq!(account_names, vec!["Checking", "Credit Card"]);

        let rent = batch.records[0].transaction.as_ref().unwrap();
        assert_eq!(rent.posted_date.to_string(), "2026-06-01");
        // Outflow -> negative (a thousands-separator amount, handled
        // generically by the CSV importer's parse_minor_units).
        assert_eq!(rent.amount, Money::new(-120_000, Currency::Usd));
        assert_eq!(rent.description.as_deref(), Some("Landlord LLC"));
        assert_eq!(
            rent.category.as_deref(),
            Some("Immediate Obligations: Rent")
        );
        assert_eq!(rent.external_account.as_deref(), Some("Checking"));

        let paycheck = batch.records[1].transaction.as_ref().unwrap();
        // Inflow -> positive.
        assert_eq!(paycheck.amount, Money::new(250_000, Currency::Usd));
        assert_eq!(
            paycheck.category.as_deref(),
            Some("Income: Ready to Assign")
        );
        assert_eq!(paycheck.external_account.as_deref(), Some("Checking"));

        let groceries = batch.records[2].transaction.as_ref().unwrap();
        assert_eq!(groceries.amount, Money::new(-8_420, Currency::Usd));
        assert_eq!(groceries.external_account.as_deref(), Some("Credit Card"));
        // A group with no matching category cell (the fixture leaves
        // Category blank for this row) is used bare, not dropped.
        assert_eq!(groceries.category.as_deref(), Some("Everyday Expenses"));

        assert!(
            batch.warnings.is_empty(),
            "a clean fixture should parse with no warnings, got {:?}",
            batch.warnings
        );
    }

    #[test]
    fn help_guide_is_not_marked_published_while_the_site_page_is_still_draft() {
        // Regression guard for personal-cfo-gvidg review finding F1 (PR
        // #15): the frontend must not be able to render a guide link for
        // this preset until dohflow-site's move-from-ynab.md actually
        // publishes. Flip this assertion in the SAME commit that flips
        // that file's draft flag -- not before.
        assert!(!Ynab.help_published());
    }
}
