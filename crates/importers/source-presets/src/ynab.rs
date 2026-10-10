//! YNAB (personal-cfo-gvidg / personal-cfo-tulv). YNAB's register export
//! (plan name menu → **Export Plan**, or **Export # Transactions** for a
//! selection — web app only, not the mobile apps) is one file spanning every
//! account, with a two-level category (Category Group + Category) and
//! separate Outflow/Inflow columns rather than one signed amount.
//!
//! Two things about that file need more than a column mapping, so the preset
//! routes it through its own [`YnabRegister`] importer (AC #2):
//!
//! - **Comma-decimal plans export tab-separated.** YNAB writes TSV, not CSV,
//!   "for currencies that use a comma" — amounts like `1.234,56€`. The
//!   importer reads a tab-delimited file with a decimal comma; a
//!   comma-delimited one with a decimal point.
//! - **Date order follows the user's YNAB date setting**, not a fixed
//!   format. The importer decides it once from the whole Date column
//!   (`csv_importer::infer_date_format`) instead of guessing per value, and
//!   says so when the file cannot decide.
//!
//! It also notes, by count, the rows a migrating user should look at: transfers
//! between YNAB accounts (each side imports as its own transaction — pairing
//! them into one DohFlow transfer is not built yet), lines of YNAB split
//! transactions, and rows still uncleared in YNAB.

use csv_importer::{
    column_values, infer_date_format, parse_with_dialect, preview_columns_with_dialect, CsvDialect,
};
use importer_core::{
    register_importer, AccountHandling, CategoryHandling, ColumnMapping, ImporterPlugin,
    ParseError, ParseWarning, ParsedBatch, ParserHints, ParserInput, SignConvention, SourcePreset,
};
use semver::Version;

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
            // YNAB's default date setting; the importer replaces it whenever
            // the file's own Date column proves another order.
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
        // One file spans every account in the plan; the import maps each
        // YNAB account to a DohFlow account (personal-cfo-tulv).
        AccountHandling::AccountColumn
    }

    fn quirks(&self) -> &'static [importer_core::SourceQuirk] {
        use importer_core::SourceQuirk::{
            MemoColumn, PendingFlag, SplitRows, ThousandsSeparator, TransferRows,
        };
        &[
            MemoColumn,
            PendingFlag,
            SplitRows,
            ThousandsSeparator,
            TransferRows,
        ]
    }

    fn verified_against(&self) -> &'static str {
        // Re-verified 2026-10-04 (personal-cfo-tulv):
        // - support.ynab.com/en_us/how-to-export-plan-data-Sy_CouWA9 (YNAB,
        //   updated 2026-08-18): export is web-only; "Export Plan" gives the
        //   plan file and the transaction register as two files, "CSV ... or
        //   TSV (tab-separated, for currencies that use a comma)"; a selection
        //   exports via "Export # Transactions"; targets and category notes
        //   are not included.
        // - beancount.io/tools/csv-to-beancount/ynab ("column mapping verified
        //   against a real YNAB export"): the unused Outflow/Inflow side is
        //   written "$0.00", not blank; every figure carries the plan's
        //   currency symbol (e.g. "$84.23", "84,23€").
        // - Register header row, as REGISTER_HEADERS in the tests. Not checked
        //   against a live export this session: no YNAB account was used.
        "YNAB register export (Account, Flag, Date, Payee, Category Group/Category, Category \
         Group, Category, Memo, Outflow, Inflow, Cleared) -- support.ynab.com export guide \
         and a third-party converter verified against a real export, 2026-10-04"
    }

    fn help_slug(&self) -> &'static str {
        "move-from-ynab"
    }

    fn help_published(&self) -> bool {
        // dohflow-site's src/content/migrate/move-from-ynab.md stays `draft:
        // true`: on 2026-10-04 the owner chose to rewrite the guide in
        // personal-cfo-tulv but keep it unpublished (the earlier steer on
        // site PR #48 was "build out robust migration mechanisms in the app
        // first, then we can come out with these"). Flip to `true` only in the
        // same change that flips that file's draft flag on dohflow-site main
        // (personal-cfo-gvidg review finding F1, PR #15).
        false
    }

    fn importer_id(&self) -> Option<&'static str> {
        Some(YnabRegister.id())
    }

    fn fixture_csv(&self) -> &'static str {
        // Synthesized, never a real export (personal-cfo-tulv AC #1). Row by
        // row (0-based data rows):
        //  0 Starting balance, both sides $0.00 -> skipped "zero amount"
        //  1 Rent: thousands separator outflow
        //  2 Paycheck: inflow, "Inflow: Ready to Assign" category
        //  3 Transfer out of Checking (payee "Transfer : Savings")
        //  4 The same transfer's other side, in Savings
        //  5-6 Two lines of one split purchase on the card
        //  7-8 An exact duplicate pair (same account, date, payee, amount)
        //  9 A negative written with a minus in the Outflow column
        // 10 A negative in parentheses in the Outflow column
        // 11 A row in another currency's sign -> skipped, currency reason
        // 12 A malformed date -> skipped "unparseable date"
        // 13 Uncleared card purchase, no category
        "\"Account\",\"Flag\",\"Date\",\"Payee\",\"Category Group/Category\",\"Category Group\",\"Category\",\"Memo\",\"Outflow\",\"Inflow\",\"Cleared\"\n\
         \"Checking\",\"\",\"06/01/2026\",\"Starting Balance\",\"Inflow: Ready to Assign\",\"Inflow\",\"Ready to Assign\",\"\",$0.00,$0.00,\"Reconciled\"\n\
         \"Checking\",\"\",\"06/01/2026\",\"Landlord LLC\",\"Bills: Rent\",\"Bills\",\"Rent\",\"June rent\",\"$1,200.00\",$0.00,\"Cleared\"\n\
         \"Checking\",\"\",\"06/02/2026\",\"Employer Inc\",\"Inflow: Ready to Assign\",\"Inflow\",\"Ready to Assign\",\"Paycheck\",$0.00,\"$2,500.00\",\"Cleared\"\n\
         \"Checking\",\"\",\"06/03/2026\",\"Transfer : Savings\",\"\",\"\",\"\",\"\",$300.00,$0.00,\"Cleared\"\n\
         \"Savings\",\"\",\"06/03/2026\",\"Transfer : Checking\",\"\",\"\",\"\",\"\",$0.00,$300.00,\"Cleared\"\n\
         \"Credit Card\",\"\",\"06/04/2026\",\"Grocery Co\",\"Everyday: Groceries\",\"Everyday\",\"Groceries\",\"Split (1/2) food\",$60.00,$0.00,\"Cleared\"\n\
         \"Credit Card\",\"\",\"06/04/2026\",\"Grocery Co\",\"Home: Supplies\",\"Home\",\"Supplies\",\"Split (2/2) soap\",$24.20,$0.00,\"Cleared\"\n\
         \"Credit Card\",\"\",\"06/05/2026\",\"Coffee Bar\",\"Everyday: Dining Out\",\"Everyday\",\"Dining Out\",\"\",$4.50,$0.00,\"Cleared\"\n\
         \"Credit Card\",\"\",\"06/05/2026\",\"Coffee Bar\",\"Everyday: Dining Out\",\"Everyday\",\"Dining Out\",\"\",$4.50,$0.00,\"Cleared\"\n\
         \"Checking\",\"\",\"06/06/2026\",\"Bank Fee\",\"Bills: Fees\",\"Bills\",\"Fees\",\"\",-$12.00,$0.00,\"Cleared\"\n\
         \"Checking\",\"\",\"06/07/2026\",\"Hardware Store\",\"Home: Supplies\",\"Home\",\"Supplies\",\"\",($42.00),$0.00,\"Cleared\"\n\
         \"Credit Card\",\"\",\"06/08/2026\",\"Hotel Abroad\",\"Travel: Lodging\",\"Travel\",\"Lodging\",\"\",€80.00,$0.00,\"Cleared\"\n\
         \"Checking\",\"\",\"June 9th\",\"Mystery\",\"\",\"\",\"\",\"\",$1.00,$0.00,\"Cleared\"\n\
         \"Credit Card\",\"\",\"06/13/2026\",\"Gas Station\",\"\",\"\",\"\",\"\",$38.75,$0.00,\"Uncleared\"\n"
    }
}

importer_core::register_preset!(Ynab);

/// The importer [`Ynab`] routes its files through (AC #2). Explicit-only:
/// [`ImporterPlugin::detect_confidence`] is `0`, so it never claims a file
/// on its own — choosing "Import from YNAB" is what selects it.
pub struct YnabRegister;

/// Rows a migrating user should look at, by kind — counted, never echoing a
/// row's own values (personal-cfo-pxi.10 logging rule).
const TRANSFER_NOTE: &str = "a transfer between YNAB accounts — each side was imported as its \
                             own transaction, not linked as a transfer";
const SPLIT_NOTE: &str = "a line of a YNAB split — each line was imported as its own transaction";
const UNCLEARED_NOTE: &str = "uncleared in YNAB — it may not have posted at your bank yet";
const DATE_ORDER_NOTE: &str = "The file's dates fit both month-first and day-first order; they \
                               were read month-first (YNAB's default). Check a few dates.";

/// YNAB's TSV is its comma-decimal export (see the module docs): the header
/// line decides which dialect the whole file is in.
fn dialect(input: &ParserInput) -> CsvDialect {
    let header = input.bytes.split(|&b| b == b'\n').next().unwrap_or(&[]);
    if header.contains(&b'\t') {
        CsvDialect {
            delimiter: b'\t',
            decimal_comma: true,
        }
    } else {
        CsvDialect::default()
    }
}

/// Whether `value` reads as a different date month-first than day-first.
fn order_matters(value: &str) -> bool {
    let parts: Vec<&str> = value.trim().split(['/', '.', '-']).collect();
    match parts.as_slice() {
        [a, b, c] if c.len() == 4 => {
            let (a, b) = (a.parse::<u32>(), b.parse::<u32>());
            matches!((a, b), (Ok(a), Ok(b)) if a != b && (1..=12).contains(&a) && (1..=12).contains(&b))
        }
        _ => false,
    }
}

fn count_note(count: usize, what: &str) -> Option<ParseWarning> {
    (count > 0).then(|| ParseWarning {
        row: None,
        message: if count == 1 {
            format!("1 row is {what}.")
        } else {
            format!("{count} rows are each {what}.")
        },
    })
}

impl ImporterPlugin for YnabRegister {
    fn id(&self) -> &'static str {
        "ynab-register"
    }

    fn display_name(&self) -> &'static str {
        "YNAB register export"
    }

    fn version(&self) -> Version {
        Version::new(1, 0, 0)
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["csv", "tsv"]
    }

    fn detect_confidence(&self, _input: &ParserInput) -> u16 {
        0
    }

    fn preview_columns(&self, input: &ParserInput) -> Vec<String> {
        preview_columns_with_dialect(input, dialect(input))
    }

    fn institution_hint(&self) -> Option<&'static str> {
        Some("YNAB")
    }

    fn parse(&self, input: &ParserInput, hints: &ParserHints) -> Result<ParsedBatch, ParseError> {
        let dialect = dialect(input);
        let mut hints = hints.clone();
        if hints.column_mapping.is_none() {
            hints.column_mapping = Ynab.hints().column_mapping;
        }
        let date_column = hints
            .column_mapping
            .as_ref()
            .and_then(|m| m.date.clone())
            .unwrap_or_else(|| "Date".to_owned());
        let dates = column_values(input, dialect, &date_column);
        let mut notes = Vec::new();
        match infer_date_format(&dates) {
            Some(format) => hints.date_format = Some(format.to_owned()),
            None if dates.iter().any(|d| order_matters(d)) => notes.push(ParseWarning {
                row: None,
                message: DATE_ORDER_NOTE.to_owned(),
            }),
            None => {}
        }

        let mut batch = parse_with_dialect(input, &hints, dialect)?;

        let (mut transfers, mut splits, mut uncleared) = (0, 0, 0);
        for record in &batch.records {
            let Ok(row) = serde_json::from_str::<serde_json::Value>(&record.normalized_json) else {
                continue;
            };
            let field = |name: &str| row.get(name).and_then(|v| v.as_str()).unwrap_or("");
            transfers += usize::from(field("Payee").starts_with("Transfer : "));
            splits += usize::from(field("Memo").starts_with("Split ("));
            uncleared += usize::from(field("Cleared").eq_ignore_ascii_case("Uncleared"));
        }
        notes.extend(count_note(transfers, TRANSFER_NOTE));
        notes.extend(count_note(splits, SPLIT_NOTE));
        notes.extend(count_note(uncleared, UNCLEARED_NOTE));
        notes.append(&mut batch.warnings);
        batch.warnings = notes;
        Ok(batch)
    }
}

register_importer!(YnabRegister);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::parse_fixture;
    use core_money::{Currency, Money};

    // The headers of YNAB's register export, in its own column order — the
    // fixture below and the docs in `Ynab::verified_against` agree on these.
    const REGISTER_HEADERS: [&str; 11] = [
        "Account",
        "Flag",
        "Date",
        "Payee",
        "Category Group/Category",
        "Category Group",
        "Category",
        "Memo",
        "Outflow",
        "Inflow",
        "Cleared",
    ];

    fn amounts(batch: &ParsedBatch) -> Vec<(String, i64)> {
        batch
            .records
            .iter()
            .map(|r| {
                let t = r.transaction.as_ref().unwrap();
                (
                    t.description.clone().unwrap_or_default(),
                    t.amount.minor_units(),
                )
            })
            .collect()
    }

    #[test]
    fn the_fixture_header_is_the_documented_register_header() {
        let header = Ynab.fixture_csv().lines().next().unwrap().replace('"', "");
        assert_eq!(header, REGISTER_HEADERS.join(","));
    }

    #[test]
    fn ynab_fixture_captures_date_sign_description_category_and_account() {
        let batch = parse_fixture(&Ynab);

        // Three distinct accounts staged (AccountHandling::AccountColumn).
        let account_names: Vec<_> = batch
            .accounts
            .iter()
            .map(|a| a.external_name.as_deref().unwrap())
            .collect();
        assert_eq!(account_names, vec!["Checking", "Credit Card", "Savings"]);

        assert_eq!(
            amounts(&batch),
            vec![
                ("Landlord LLC".to_owned(), -120_000),
                ("Employer Inc".to_owned(), 250_000),
                ("Transfer : Savings".to_owned(), -30_000),
                ("Transfer : Checking".to_owned(), 30_000),
                ("Grocery Co".to_owned(), -6_000),
                ("Grocery Co".to_owned(), -2_420),
                ("Coffee Bar".to_owned(), -450),
                ("Coffee Bar".to_owned(), -450),
                // The Outflow column decides direction whatever the cell's
                // own sign (the generic debit rule): still outflows.
                ("Bank Fee".to_owned(), -1_200),
                ("Hardware Store".to_owned(), -4_200),
                ("Gas Station".to_owned(), -3_875),
            ]
        );

        let rent = batch.records[0].transaction.as_ref().unwrap();
        assert_eq!(rent.posted_date.to_string(), "2026-06-01");
        assert_eq!(rent.amount, Money::new(-120_000, Currency::Usd));
        assert_eq!(rent.category.as_deref(), Some("Bills: Rent"));
        assert_eq!(rent.external_account.as_deref(), Some("Checking"));

        let paycheck = batch.records[1].transaction.as_ref().unwrap();
        assert_eq!(
            paycheck.category.as_deref(),
            Some("Inflow: Ready to Assign")
        );

        let transfer_in = batch.records[3].transaction.as_ref().unwrap();
        assert_eq!(transfer_in.external_account.as_deref(), Some("Savings"));
        assert_eq!(transfer_in.category, None);

        // The duplicate pair parses as two rows with one fingerprint; the
        // pipeline's dedupe, not the parser, decides (AC #3's e2e test).
        let dup_a = batch.records[6].transaction.as_ref().unwrap();
        let dup_b = batch.records[7].transaction.as_ref().unwrap();
        assert_eq!(dup_a.txn_fingerprint, dup_b.txn_fingerprint);
        assert_ne!(batch.records[6].source_hash, batch.records[7].source_hash);

        let skipped: Vec<_> = batch
            .skipped
            .iter()
            .map(|w| (w.row, w.message.as_str()))
            .collect();
        assert_eq!(
            skipped,
            vec![
                (Some(0), "zero amount"),
                (Some(11), "currency differs from the import currency"),
                (Some(12), "unparseable date"),
            ]
        );

        let notes: Vec<_> = batch.warnings.iter().map(|w| w.message.as_str()).collect();
        assert_eq!(
            notes,
            vec![
                format!("2 rows are each {TRANSFER_NOTE}."),
                format!("2 rows are each {SPLIT_NOTE}."),
                format!("1 row is {UNCLEARED_NOTE}."),
            ]
        );
    }

    #[test]
    fn a_comma_decimal_plan_exports_tsv_and_imports_with_its_day_first_dates() {
        // YNAB writes TSV for comma-decimal currencies (support.ynab.com,
        // 2026-10-04). Day-first dates: 05/06 alone is ambiguous, 13/06
        // settles the file as day-first, so 05/06 is 5 June.
        let tsv = "Account\tFlag\tDate\tPayee\tCategory Group/Category\tCategory Group\tCategory\tMemo\tOutflow\tInflow\tCleared\n\
                   Girokonto\t\t05/06/2026\tBäckerei\tAlltag: Essen\tAlltag\tEssen\t\t1.234,56€\t0,00€\tCleared\n\
                   Girokonto\t\t13/06/2026\tArbeitgeber\tInflow: Ready to Assign\tInflow\tReady to Assign\t\t0,00€\t2.500,00€\tCleared\n";
        let input = ParserInput::new(tsv.as_bytes().to_vec()).with_filename("plan.tsv");
        let hints = ParserHints {
            default_currency: Some(Currency::Eur),
            ..Ynab.hints()
        };
        let batch = YnabRegister.parse(&input, &hints).unwrap();
        assert!(batch.skipped.is_empty(), "{:?}", batch.skipped);
        assert!(batch.warnings.is_empty(), "{:?}", batch.warnings);
        let first = batch.records[0].transaction.as_ref().unwrap();
        assert_eq!(first.posted_date.to_string(), "2026-06-05");
        assert_eq!(first.amount, Money::new(-123_456, Currency::Eur));
        assert_eq!(
            batch.records[1].transaction.as_ref().unwrap().amount,
            Money::new(250_000, Currency::Eur)
        );
        assert_eq!(
            YnabRegister.preview_columns(&input),
            REGISTER_HEADERS.to_vec()
        );
    }

    #[test]
    fn an_undecidable_date_order_is_read_month_first_with_a_note() {
        let csv = "Account,Flag,Date,Payee,Category Group/Category,Category Group,Category,Memo,Outflow,Inflow,Cleared\n\
                   Checking,,05/06/2026,Store,,,,,$5.00,$0.00,Cleared\n";
        let input = ParserInput::new(csv.as_bytes().to_vec());
        let batch = YnabRegister.parse(&input, &Ynab.hints()).unwrap();
        let txn = batch.records[0].transaction.as_ref().unwrap();
        assert_eq!(txn.posted_date.to_string(), "2026-05-06");
        assert_eq!(batch.warnings[0].message, DATE_ORDER_NOTE);
        assert_eq!(batch.warnings[0].row, None);
    }

    #[test]
    fn notes_never_echo_a_row_s_own_values() {
        let batch = parse_fixture(&Ynab);
        let reported = format!("{:?}{:?}", batch.warnings, batch.skipped);
        for value in [
            "Savings",
            "Checking",
            "Grocery",
            "Gas Station",
            "food",
            "2026",
        ] {
            assert!(!reported.contains(value), "{value} leaked: {reported}");
        }
    }

    #[test]
    fn the_register_importer_is_explicit_only() {
        let input = ParserInput::new(Ynab.fixture_csv().as_bytes().to_vec())
            .with_filename("YNAB Export - Plan.csv");
        assert_eq!(YnabRegister.detect_confidence(&input), 0);
        assert_eq!(
            importer_core::detect_best(&input).map(|p| p.id()),
            Some("generic-csv"),
            "auto-detect is unchanged for a YNAB file chosen without the preset"
        );
        assert_eq!(Ynab.importer_id(), Some("ynab-register"));
        assert!(importer_core::plugin_by_id("ynab-register").is_some());
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
