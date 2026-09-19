//! Generic CSV importer (personal-cfo-cu8) — the first real [`ImporterPlugin`].
//!
//! Turns a CSV export (untrusted bytes) into a typed [`ParsedBatch`] of staged
//! candidates: it resolves columns (explicit mapping or header auto-detect),
//! parses **locale-aware amounts** (currency symbol / thousands separators /
//! parentheses-as-negative / separate debit & credit columns) and **dates with a
//! confidence** (ISO and US/EU `M/D/Y` vs `D/M/Y`, flagging the ambiguous ones),
//! and builds the transaction dedupe fingerprint. Anything it can't read becomes
//! a [`ParseWarning`] rather than a silent guess. It holds no keys/DB/network/IPC
//! (the eay trait) and runs behind the bounded host (`hs9`, ADR 0022).
//!
//! v1 scope: US-style numerics (`,` = thousands, `.` = decimal). European
//! decimal-comma needs an explicit locale hint and is a documented follow-up.

use std::collections::BTreeSet;

use core_money::{Currency, Money};
use importer_core::{
    register_importer, ColumnMapping, ImporterPlugin, ParseError, ParseWarning, ParsedAccount,
    ParsedBatch, ParsedRecord, ParsedTransaction, ParserHints, ParserInput,
};
use semver::Version;

/// The resolved source-column indices.
#[derive(Default)]
struct Columns {
    /// The PRIMARY posted-date column (ADR 0045): a header containing "posted" when
    /// present, else the first date column.
    posted_date: Option<usize>,
    /// A distinct transaction / authorization date column, when the source has one in
    /// addition to the posted date (ADR 0045). `None` for single-date sources.
    transaction_date: Option<usize>,
    description: Option<usize>,
    amount: Option<usize>,
    debit: Option<usize>,
    credit: Option<usize>,
    currency: Option<usize>,
    category: Option<usize>,
    /// A second, coarser category column (personal-cfo-gvidg) — e.g. YNAB's
    /// "Category Group" — combined with `category` as `"{group}: {category}"`.
    /// Only resolved from an explicit mapping (a preset's `ColumnMapping`);
    /// auto-detect never guesses this, since no generic header name for it
    /// is common enough to be safe to guess.
    category_group: Option<usize>,
    /// An account-label column (personal-cfo-gvidg) — a source that exports
    /// several accounts in one file names each row's account here. `None`
    /// for the (today, universal) one-file-per-account case.
    account: Option<usize>,
}

/// Map a currency code to a [`Currency`], or `None` if unsupported.
fn currency_from_code(code: &str) -> Option<Currency> {
    match code.trim().to_ascii_uppercase().as_str() {
        "USD" => Some(Currency::Usd),
        "EUR" => Some(Currency::Eur),
        _ => None,
    }
}

/// Parse a money string into minor units for a currency with `exponent` decimal
/// places. Handles a leading currency symbol, thousands separators, surrounding
/// parentheses (= negative, accounting style), and a leading sign. `None` if it
/// is not a recognizable amount.
fn parse_minor_units(raw: &str, exponent: u8) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut negative = false;
    let mut body = trimmed;
    if let Some(inner) = body.strip_prefix('(').and_then(|b| b.strip_suffix(')')) {
        negative = true;
        body = inner.trim();
    }
    if let Some(rest) = body.strip_prefix('-') {
        negative = !negative;
        body = rest.trim_start();
    } else if let Some(rest) = body.strip_prefix('+') {
        body = rest.trim_start();
    }
    // Keep only digits + the decimal point (drops `$`, thousands separators,
    // spaces). A comma is treated as a thousands separator (US style).
    let cleaned: String = body
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let (int_str, frac_str) = cleaned.split_once('.').unwrap_or((cleaned.as_str(), ""));
    let exp = exponent as usize;
    let mut frac = frac_str.to_owned();
    if frac.len() < exp {
        frac.push_str(&"0".repeat(exp - frac.len()));
    } else {
        frac.truncate(exp);
    }
    let int_val: i64 = if int_str.is_empty() {
        0
    } else {
        int_str.parse().ok()?
    };
    // A stray second '.' (e.g. "1.2.3") leaves a non-digit here → parse fails.
    let frac_val: i64 = if frac.is_empty() {
        0
    } else {
        frac.parse().ok()?
    };
    let scale = 10i64.checked_pow(exp as u32)?;
    let magnitude = int_val.checked_mul(scale)?.checked_add(frac_val)?;
    Some(if negative { -magnitude } else { magnitude })
}

/// Parse a date with a confidence in basis points. An explicit `hint_format` or
/// ISO-8601 is unambiguous (10000); a single US/EU match is high (9000); a value
/// that parses *both* ways to different dates is ambiguous (5000, assumed US).
fn parse_date(raw: &str, hint_format: Option<&str>) -> Option<(chrono::NaiveDate, u16)> {
    use chrono::NaiveDate;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(fmt) = hint_format {
        if let Ok(date) = NaiveDate::parse_from_str(trimmed, fmt) {
            return Some((date, 10_000));
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Some((date, 10_000));
    }
    let us = NaiveDate::parse_from_str(trimmed, "%m/%d/%Y")
        .or_else(|_| NaiveDate::parse_from_str(trimmed, "%m/%d/%y"))
        .ok();
    let eu = NaiveDate::parse_from_str(trimmed, "%d/%m/%Y")
        .or_else(|_| NaiveDate::parse_from_str(trimmed, "%d/%m/%y"))
        .ok();
    match (us, eu) {
        (Some(u), Some(e)) if u == e => Some((u, 9_000)),
        (Some(u), Some(_)) => Some((u, 5_000)),
        (Some(u), None) => Some((u, 9_000)),
        (None, Some(e)) => Some((e, 9_000)),
        (None, None) => None,
    }
}

fn resolve_columns(headers: &csv::StringRecord, mapping: Option<&ColumnMapping>) -> Columns {
    let find = |name: &str| {
        headers
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
    };
    let find_any = |needles: &[&str]| {
        headers.iter().position(|h| {
            let lower = h.trim().to_ascii_lowercase();
            needles.iter().any(|n| lower.contains(n))
        })
    };
    if let Some(m) = mapping {
        // An explicit mapping names a single `date` (the posted date); a separate
        // transaction-date remap is future work (the mapping UI, ADR 0045 slice 3).
        Columns {
            posted_date: m.date.as_deref().and_then(&find),
            transaction_date: None,
            description: m.description.as_deref().and_then(&find),
            amount: m.amount.as_deref().and_then(&find),
            debit: m.debit.as_deref().and_then(&find),
            credit: m.credit.as_deref().and_then(&find),
            currency: m.currency.as_deref().and_then(&find),
            category: m.category.as_deref().and_then(&find),
            category_group: m.category_group.as_deref().and_then(&find),
            account: m.account.as_deref().and_then(&find),
        }
    } else {
        // Every column whose header mentions a date, in source order.
        let date_cols: Vec<usize> = headers
            .iter()
            .enumerate()
            .filter(|(_, h)| h.trim().to_ascii_lowercase().contains("date"))
            .map(|(i, _)| i)
            .collect();
        let has = |i: usize, needle: &str| {
            headers
                .get(i)
                .is_some_and(|h| h.trim().to_ascii_lowercase().contains(needle))
        };
        // Posted date = a "posted" date header if present, else the first date column
        // (ADR 0045: the posted date is primary — never let "transaction date" win it).
        let posted_date = date_cols
            .iter()
            .copied()
            .find(|&i| has(i, "posted"))
            .or_else(|| date_cols.first().copied());
        // Transaction date = a different date column (prefer an explicit
        // transaction/authorization header, else any other date column).
        let transaction_date = date_cols
            .iter()
            .copied()
            .find(|&i| Some(i) != posted_date && (has(i, "transaction") || has(i, "auth")))
            .or_else(|| date_cols.iter().copied().find(|&i| Some(i) != posted_date));
        Columns {
            posted_date,
            transaction_date,
            description: find_any(&["description", "memo", "payee", "merchant", "name"]),
            amount: find_any(&["amount"]),
            debit: find_any(&["debit", "withdrawal"]),
            credit: find_any(&["credit", "deposit"]),
            currency: find_any(&["currency"]),
            category: find_any(&["category"]),
            // Never auto-detected: no generic "group" header name is common
            // enough across exports to guess safely (personal-cfo-gvidg) —
            // only an explicit preset/user mapping sets this.
            category_group: None,
            account: find_any(&["account"]),
        }
    }
}

/// Resolve a row's signed amount: a single `amount` column wins; otherwise a
/// non-zero `debit` is an outflow (negative) and a non-zero `credit` an inflow.
fn resolve_amount(row: &csv::StringRecord, cols: &Columns, exponent: u8) -> Option<i64> {
    if let Some(minor) = cols
        .amount
        .and_then(|i| row.get(i))
        .and_then(|v| parse_minor_units(v, exponent))
    {
        return Some(minor);
    }
    let debit = cols
        .debit
        .and_then(|i| row.get(i))
        .and_then(|v| parse_minor_units(v, exponent));
    let credit = cols
        .credit
        .and_then(|i| row.get(i))
        .and_then(|v| parse_minor_units(v, exponent));
    match (debit, credit) {
        (Some(d), _) if d != 0 => Some(-d.abs()),
        (_, Some(c)) if c != 0 => Some(c.abs()),
        _ => None,
    }
}

fn warn(row: usize, message: impl Into<String>) -> ParseWarning {
    ParseWarning {
        row: Some(row),
        message: message.into(),
    }
}

/// The normalized fields as JSON (persisted; the raw bytes are not — ADR 0014 §4).
fn normalized_json(headers: &csv::StringRecord, row: &csv::StringRecord) -> String {
    let map: serde_json::Map<String, serde_json::Value> = headers
        .iter()
        .zip(row.iter())
        .map(|(h, v)| (h.to_owned(), serde_json::Value::String(v.to_owned())))
        .collect();
    serde_json::Value::Object(map).to_string()
}

/// The generic CSV importer.
pub struct GenericCsv;

impl ImporterPlugin for GenericCsv {
    fn id(&self) -> &'static str {
        "generic-csv"
    }
    fn display_name(&self) -> &'static str {
        "Generic CSV"
    }
    fn version(&self) -> Version {
        Version::new(1, 0, 0)
    }
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["csv"]
    }
    fn detect_confidence(&self, input: &ParserInput) -> u16 {
        if input.extension().as_deref() == Some("csv") {
            return 9_000;
        }
        // A comma in the first line is a weak CSV signal.
        let first_line = input.bytes.split(|&b| b == b'\n').next().unwrap_or(&[]);
        if first_line.contains(&b',') {
            3_000
        } else {
            0
        }
    }
    fn column_mapping_hints(&self) -> &'static [&'static str] {
        &[
            "date",
            "description",
            "amount",
            "debit",
            "credit",
            "currency",
            "memo",
        ]
    }

    fn preview_columns(&self, input: &ParserInput) -> Vec<String> {
        // Same reader config as `parse`, so the returned headers are exactly what a
        // `ColumnMapping` is matched against (ADR 0045 slice 3, personal-cfo-4d8.24.1.2).
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(input.bytes.as_slice());
        reader
            .headers()
            .map(|h| h.iter().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    fn parse(&self, input: &ParserInput, hints: &ParserHints) -> Result<ParsedBatch, ParseError> {
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(input.bytes.as_slice());
        let headers = reader
            .headers()
            .map_err(|e| ParseError::Malformed(format!("unreadable CSV header: {e}")))?
            .clone();

        let cols = resolve_columns(&headers, hints.column_mapping.as_ref());
        let Some(date_col) = cols.posted_date else {
            return Err(ParseError::Unsupported(
                "no date column found — map the columns explicitly".to_owned(),
            ));
        };
        if cols.amount.is_none() && cols.debit.is_none() && cols.credit.is_none() {
            return Err(ParseError::Unsupported(
                "no amount / debit / credit column found".to_owned(),
            ));
        }

        let currency = hints.default_currency.unwrap_or(Currency::Usd);
        let mut records = Vec::new();
        let mut warnings = Vec::new();
        let mut seen_accounts = BTreeSet::new();

        for (idx, result) in reader.records().enumerate() {
            let row = match result {
                Ok(row) => row,
                Err(e) => {
                    warnings.push(warn(idx, format!("unreadable row: {e}")));
                    continue;
                }
            };

            // Mixed-currency rows are rejected, not silently converted.
            if let Some(code) = cols
                .currency
                .and_then(|i| row.get(i))
                .filter(|c| !c.is_empty())
            {
                if currency_from_code(code) != Some(currency) {
                    warnings.push(warn(
                        idx,
                        format!("currency {code:?} differs from the import currency — skipped"),
                    ));
                    continue;
                }
            }

            let raw_date = row.get(date_col).unwrap_or("").to_owned();
            let Some((posted_date, date_confidence_bps)) =
                parse_date(&raw_date, hints.date_format.as_deref())
            else {
                warnings.push(warn(idx, format!("unparseable date {raw_date:?}")));
                continue;
            };
            if date_confidence_bps < 7_000 {
                warnings.push(warn(
                    idx,
                    format!("ambiguous date {raw_date:?} (assumed US M/D/Y)"),
                ));
            }

            let Some(amount_minor) = resolve_amount(&row, &cols, currency.exponent()) else {
                warnings.push(warn(idx, "unparseable / missing amount"));
                continue;
            };

            // The secondary transaction/authorization date (ADR 0045) — parsed leniently:
            // a bad value is simply dropped (it never blocks the row or warns). Dropped
            // when it equals the posted date so the detail view never shows the same date
            // twice (mirrors the OFX importer's DTUSER != DTPOSTED filter).
            let transaction_date = cols
                .transaction_date
                .and_then(|i| row.get(i))
                .filter(|s| !s.trim().is_empty())
                .and_then(|raw| parse_date(raw, hints.date_format.as_deref()))
                .map(|(d, _)| d)
                .filter(|&d| d != posted_date);
            let trimmed = |i: Option<usize>| {
                i.and_then(|i| row.get(i))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            let description = trimmed(cols.description);
            // A group + category combine as "Group: Category"; a group alone
            // (no matching category cell) is used bare rather than dropped
            // (personal-cfo-gvidg) — still a real, if coarser, prefill.
            let category = match (trimmed(cols.category_group), trimmed(cols.category)) {
                (Some(group), Some(cat)) => Some(format!("{group}: {cat}")),
                (Some(group), None) => Some(group),
                (None, cat) => cat,
            };
            let account_label = trimmed(cols.account);
            let normalized_merchant = description.as_ref().map(|d| d.to_ascii_lowercase());
            let txn_fingerprint = format!(
                "{posted_date}|{amount_minor}|{}",
                normalized_merchant.as_deref().unwrap_or("")
            );

            let normalized = normalized_json(&headers, &row);
            // Per-row source hash (row index keeps two identical rows distinct;
            // whole-file re-import is caught by the batch file fingerprint).
            let source_hash =
                importer_core::content_fingerprint(format!("{idx}:{normalized}").as_bytes());

            if let Some(label) = &account_label {
                seen_accounts.insert(label.clone());
            }
            records.push(ParsedRecord {
                external_id: None,
                source_hash,
                normalized_json: normalized,
                parse_confidence_bps: Some(date_confidence_bps),
                transaction: Some(ParsedTransaction {
                    posted_date,
                    transaction_date,
                    raw_date,
                    date_confidence_bps,
                    amount: Money::new(amount_minor, currency),
                    description,
                    category,
                    normalized_merchant,
                    // The account column's raw label, when the source has one
                    // (personal-cfo-gvidg) — doubles as the matching
                    // `ParsedAccount::external_id` below, so a per-account
                    // commit path (e.g. `stage_sync_batch`) can correlate the
                    // two by the same string. `stage_parsed_batch` (today's
                    // only file-import commit path) does not read this field
                    // yet — see this crate's `SourcePreset` docs.
                    external_account: account_label.clone(),
                    txn_fingerprint,
                }),
                balance: None,
            });
        }

        // One ParsedAccount per distinct account label observed (sorted, via
        // BTreeSet — matching happens by label, so order carries no meaning)
        // — never from a source with no account column (accounts stays
        // empty, unchanged from before this field existed).
        let accounts = seen_accounts
            .into_iter()
            .map(|label| ParsedAccount {
                external_id: Some(label.clone()),
                external_name: Some(label),
                external_number_hash: None,
                proposed_subtype: None,
            })
            .collect();

        Ok(ParsedBatch {
            source_format: "csv".to_owned(),
            accounts,
            records,
            warnings,
        })
    }
}

register_importer!(GenericCsv);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minor_units_handles_money_shapes() {
        assert_eq!(parse_minor_units("12.99", 2), Some(1299));
        assert_eq!(parse_minor_units("-12.99", 2), Some(-1299));
        assert_eq!(parse_minor_units("(42.00)", 2), Some(-4200));
        assert_eq!(parse_minor_units("$1,234.56", 2), Some(123_456));
        assert_eq!(parse_minor_units("1000", 2), Some(100_000));
        assert_eq!(parse_minor_units(".50", 2), Some(50));
        assert_eq!(parse_minor_units("  $ 1,000.00 ", 2), Some(100_000));
        assert_eq!(parse_minor_units("", 2), None);
        assert_eq!(parse_minor_units("n/a", 2), None);
        assert_eq!(parse_minor_units("1.2.3", 2), None);
        // Exponent-0 currency would scale differently (defensive).
        assert_eq!(parse_minor_units("1500", 0), Some(1500));
    }

    #[test]
    fn parse_date_scores_confidence() {
        let iso = parse_date("2026-06-20", None).unwrap();
        assert_eq!(iso.1, 10_000);
        // 20 can't be a month → US-only.
        assert_eq!(parse_date("06/20/2026", None).unwrap().1, 9_000);
        // 13 can't be a month → EU-only.
        assert_eq!(parse_date("13/06/2026", None).unwrap().1, 9_000);
        // Both valid + different → ambiguous, assume US.
        let amb = parse_date("05/06/2026", None).unwrap();
        assert_eq!(amb.1, 5_000);
        assert_eq!(amb.0, chrono::NaiveDate::from_ymd_opt(2026, 5, 6).unwrap());
        assert!(parse_date("not a date", None).is_none());
    }

    fn input(csv: &str) -> ParserInput {
        ParserInput::new(csv.as_bytes().to_vec()).with_filename("statement.csv")
    }

    #[test]
    fn capitalone_shaped_csv_uses_posted_date_and_captures_all_fields() {
        // Real CapitalOne credit-card export shape (personal-cfo-4d8.24.1, ADR 0045).
        let csv = "Transaction Date,Posted Date,Card No.,Description,Category,Debit,Credit\n\
                   2026-06-18,2026-06-20,1234,Coffee Shop,Dining,12.99,\n\
                   2026-06-19,2026-06-21,1234,Paycheck,Income,,1500.00\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 2);

        let first = batch.records[0].transaction.as_ref().unwrap();
        // The POSTED date is primary — NOT the transaction date (the pre-fix bug).
        assert_eq!(
            first.posted_date,
            chrono::NaiveDate::from_ymd_opt(2026, 6, 20).unwrap(),
            "Posted Date is the primary date, not Transaction Date"
        );
        assert_eq!(
            first.transaction_date,
            Some(chrono::NaiveDate::from_ymd_opt(2026, 6, 18).unwrap()),
            "the Transaction Date is captured as the secondary date"
        );
        assert_eq!(first.category.as_deref(), Some("Dining"));
        // A Debit is an outflow (negative).
        assert_eq!(first.amount, Money::new(-1299, Currency::Usd));
        // No field is lost: the full row (incl. Card No.) is in normalized_json.
        let raw: serde_json::Value =
            serde_json::from_str(&batch.records[0].normalized_json).unwrap();
        assert_eq!(raw["Card No."], "1234");
        assert_eq!(raw["Category"], "Dining");
        assert_eq!(raw["Posted Date"], "2026-06-20");

        let second = batch.records[1].transaction.as_ref().unwrap();
        // A Credit is an inflow (positive).
        assert_eq!(second.amount, Money::new(150_000, Currency::Usd));
        assert_eq!(second.category.as_deref(), Some("Income"));
        assert!(batch.warnings.is_empty());
    }

    #[test]
    fn csv_drops_a_transaction_date_equal_to_the_posted_date() {
        // When the two date columns hold the same value there is no distinct secondary
        // date to show (ADR 0045) — the detail view must not display it twice.
        let csv = "Transaction Date,Posted Date,Description,Amount\n\
                   2026-06-20,2026-06-20,Coffee,-5.00\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        let txn = batch.records[0].transaction.as_ref().unwrap();
        assert_eq!(
            txn.posted_date,
            chrono::NaiveDate::from_ymd_opt(2026, 6, 20).unwrap()
        );
        assert_eq!(txn.transaction_date, None);
    }

    #[test]
    fn preview_columns_returns_the_trimmed_source_headers() {
        let csv = "Transaction Date, Posted Date ,Card No.,Description,Category,Debit,Credit\n\
                   2026-06-18,2026-06-20,1234,Coffee,Dining,5.00,\n";
        let cols = GenericCsv.preview_columns(&input(csv));
        assert_eq!(
            cols,
            vec![
                "Transaction Date",
                "Posted Date",
                "Card No.",
                "Description",
                "Category",
                "Debit",
                "Credit",
            ]
        );
    }

    #[test]
    fn parses_a_signed_amount_csv() {
        let csv = "Date,Description,Amount\n2026-06-20,Coffee Shop,-12.99\n2026-06-21,Paycheck,\"1,500.00\"\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 2);
        let first = batch.records[0].transaction.as_ref().unwrap();
        assert_eq!(first.amount, Money::new(-1299, Currency::Usd));
        assert_eq!(first.normalized_merchant.as_deref(), Some("coffee shop"));
        assert_eq!(
            first.posted_date,
            chrono::NaiveDate::from_ymd_opt(2026, 6, 20).unwrap()
        );
        let second = batch.records[1].transaction.as_ref().unwrap();
        assert_eq!(second.amount, Money::new(150_000, Currency::Usd));
        assert!(batch.warnings.is_empty());
    }

    #[test]
    fn parses_debit_and_credit_columns() {
        let csv = "Date,Memo,Debit,Credit\n2026-06-20,Rent,1200.00,\n2026-06-25,Refund,,45.00\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 2);
        // Debit is an outflow (negative); credit an inflow (positive).
        assert_eq!(
            batch.records[0].transaction.as_ref().unwrap().amount,
            Money::new(-120_000, Currency::Usd)
        );
        assert_eq!(
            batch.records[1].transaction.as_ref().unwrap().amount,
            Money::new(4500, Currency::Usd)
        );
    }

    #[test]
    fn an_account_column_stages_distinct_parsed_accounts_and_stamps_rows() {
        let csv = "Date,Description,Amount,Account\n2026-06-20,Rent,-1200.00,Checking\n2026-06-21,Paycheck,2500.00,Checking\n2026-06-22,Groceries,-84.20,Credit Card\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 3);
        let mut account_names: Vec<_> = batch
            .accounts
            .iter()
            .map(|a| a.external_name.as_deref().unwrap())
            .collect();
        account_names.sort_unstable();
        assert_eq!(account_names, vec!["Checking", "Credit Card"]);
        // Every ParsedAccount's external_id matches its external_name — the
        // same string a row's external_account carries, so a per-row
        // commit path can correlate the two (personal-cfo-gvidg).
        for account in &batch.accounts {
            assert_eq!(account.external_id, account.external_name);
        }
        let external_accounts: Vec<_> = batch
            .records
            .iter()
            .map(|r| r.transaction.as_ref().unwrap().external_account.clone())
            .collect();
        assert_eq!(
            external_accounts,
            vec![
                Some("Checking".to_owned()),
                Some("Checking".to_owned()),
                Some("Credit Card".to_owned()),
            ]
        );
    }

    #[test]
    fn a_csv_with_no_account_column_stages_no_parsed_accounts() {
        // Unchanged from before ParsedAccount existed on this importer: no
        // account column means no accounts staged, ever.
        let csv = "Date,Description,Amount\n2026-06-20,Rent,-1200.00\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert!(batch.accounts.is_empty());
        assert_eq!(
            batch.records[0]
                .transaction
                .as_ref()
                .unwrap()
                .external_account,
            None
        );
    }

    #[test]
    fn an_explicit_category_group_combines_with_category() {
        let csv = "Date,Description,Amount,Group,Cat\n2026-06-20,Rent,-1200.00,Immediate Obligations,Rent\n2026-06-21,Misc,-10.00,Just for Fun,\n";
        let hints = ParserHints {
            column_mapping: Some(ColumnMapping {
                date: Some("Date".to_owned()),
                amount: Some("Amount".to_owned()),
                category_group: Some("Group".to_owned()),
                category: Some("Cat".to_owned()),
                ..ColumnMapping::default()
            }),
            ..ParserHints::default()
        };
        let batch = GenericCsv.parse(&input(csv), &hints).unwrap();
        assert_eq!(
            batch.records[0].transaction.as_ref().unwrap().category,
            Some("Immediate Obligations: Rent".to_owned())
        );
        // A group with no matching category cell is used bare, not dropped.
        assert_eq!(
            batch.records[1].transaction.as_ref().unwrap().category,
            Some("Just for Fun".to_owned())
        );
    }

    #[test]
    fn a_malformed_row_becomes_a_warning_not_a_transaction() {
        let csv = "Date,Description,Amount\n2026-06-20,Good,-10.00\nbogus,Bad Row,xyz\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 1, "only the valid row is staged");
        assert_eq!(batch.warnings.len(), 1, "the bad row is flagged");
        assert_eq!(batch.warnings[0].row, Some(1));
    }

    #[test]
    fn an_ambiguous_date_is_warned() {
        let csv = "Date,Description,Amount\n05/06/2026,Store,-10.00\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(
            batch.records[0]
                .transaction
                .as_ref()
                .unwrap()
                .date_confidence_bps,
            5_000
        );
        assert!(batch
            .warnings
            .iter()
            .any(|w| w.message.contains("ambiguous")));
    }

    #[test]
    fn a_csv_with_no_amount_column_is_unsupported() {
        let csv = "Date,Note\n2026-06-20,hello\n";
        let err = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap_err();
        assert!(matches!(err, ParseError::Unsupported(_)));
    }

    #[test]
    fn registered_in_the_compile_time_registry() {
        let plugin = importer_core::plugin_by_id("generic-csv").expect("registered");
        assert_eq!(plugin.version(), Version::new(1, 0, 0));
        let best = importer_core::detect_best(&input("Date,Amount\n2026-01-01,1.00\n"))
            .expect("detected by .csv extension");
        assert_eq!(best.id(), "generic-csv");
    }
}
