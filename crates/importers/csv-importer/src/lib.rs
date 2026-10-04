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
//! Numerics default to US style (`,` = thousands, `.` = decimal). A
//! [`CsvDialect`] with `decimal_comma` reads the European style (`.`/space =
//! thousands, `,` = decimal) — chosen by a caller that *knows* the file's
//! convention (the YNAB importer, whose comma-decimal plans export as TSV;
//! personal-cfo-tulv), never guessed per value. Either way a separator in an
//! impossible place (`12,50` read US-style) refuses the value instead of
//! silently scaling it.

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

/// How a delimited file is written (personal-cfo-tulv): its field delimiter
/// and whether amounts use a decimal comma. [`GenericCsv`] always reads the
/// default (comma-delimited, decimal point); a source-specific importer that
/// knows its file's convention passes its own to [`parse_with_dialect`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsvDialect {
    pub delimiter: u8,
    /// `1.234,56` rather than `1,234.56`.
    pub decimal_comma: bool,
}

impl Default for CsvDialect {
    fn default() -> Self {
        Self {
            delimiter: b',',
            decimal_comma: false,
        }
    }
}

/// Currency signs the importer recognizes inside an amount cell. A cell
/// carrying one of these that is not the import currency's own sign is a
/// different currency, never silently relabeled (personal-cfo-tulv).
const CURRENCY_SIGNS: &[char] = &[
    '$', '€', '£', '¥', '₹', '₩', '₽', '₺', '₪', '₫', '₱', '₦', '₴', '₸', '¢', '₡', '₲', '₵', '₭',
    '₮', '₼', '₾', '฿',
];

/// Why an amount cell yielded no usable value.
#[derive(Debug, PartialEq, Eq)]
enum AmountProblem {
    /// Empty, or not a number in the file's convention.
    Unreadable,
    /// The cell names a currency other than the import currency.
    ForeignCurrency,
}

/// The currency marker written around an amount (`$`, `C$`, `€`, `US$`), if
/// it carries a currency sign — letters alone (`USD`) are not treated as a
/// currency here, and a debit/credit indicator (`CR`, `DR`, any case) is not
/// part of the marker: `$1,234.56 CR` is a dollar amount, as it was before
/// personal-cfo-tulv. A letter run touching the sign (`C$`) is kept, so it
/// still names its own currency.
fn currency_marker(raw: &str) -> Option<String> {
    // Split into letter runs and everything else, dropping a whole run that
    // is a debit/credit indicator. Whitespace separates runs, then goes.
    let mut marker = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, marker: &mut String| {
        if !(run.eq_ignore_ascii_case("cr") || run.eq_ignore_ascii_case("dr")) {
            marker.push_str(run);
        }
        run.clear();
    };
    for c in raw
        .chars()
        .filter(|c| !c.is_ascii_digit() && !"+-().,'".contains(*c))
    {
        if c.is_ascii_alphabetic() {
            run.push(c);
        } else {
            flush(&mut run, &mut marker);
            if !c.is_whitespace() {
                marker.push(c);
            }
        }
    }
    flush(&mut run, &mut marker);
    marker.contains(CURRENCY_SIGNS).then_some(marker)
}

/// Whether `marker` names `currency` itself.
fn marker_matches(marker: &str, currency: Currency) -> bool {
    match currency {
        Currency::Usd => matches!(marker, "$" | "US$" | "USD$"),
        Currency::Eur => marker == "€",
        _ => false,
    }
}

/// Whether `int_part` uses `sep` only as a thousands separator in valid
/// places: a leading group of 1–3 digits, then groups of exactly 3.
fn valid_grouping(int_part: &str, sep: char) -> bool {
    let mut groups = int_part.split(sep);
    let first = groups.next().unwrap_or("");
    if !int_part.contains(sep) {
        return true;
    }
    (1..=3).contains(&first.len()) && groups.all(|g| g.len() == 3)
}

/// Parse a money string into minor units for a currency with `exponent` decimal
/// places. Handles a leading currency symbol, thousands separators, surrounding
/// parentheses (= negative, accounting style), and a leading sign. `None` if it
/// is not a recognizable amount — including a thousands separator in a place
/// no real grouping puts it (`12,50` read US-style is refused, not read as
/// 1250; personal-cfo-tulv).
fn parse_minor_units(raw: &str, exponent: u8, decimal_comma: bool) -> Option<i64> {
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
    // A sign after the currency symbol (`$-12.00`, `-$12.00` handled above).
    if let Some(pos) = body.find('-') {
        if body[..pos].chars().all(|c| !c.is_ascii_digit()) {
            negative = !negative;
            body = &body[pos + 1..];
        }
    }
    let (decimal, thousands) = if decimal_comma {
        (',', '.')
    } else {
        ('.', ',')
    };
    // Keep only digits + the two separators (drops `$`, spaces, letters).
    // Space and apostrophe groupings are dropped unvalidated, as before.
    let cleaned: String = body
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == decimal || *c == thousands)
        .collect();
    if !cleaned.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let (int_part, frac_str) = match cleaned.split_once(decimal) {
        Some((int_part, frac)) => (int_part, frac),
        None => (cleaned.as_str(), ""),
    };
    // A second decimal separator, or a thousands separator after the
    // decimal one, is not an amount.
    if frac_str.contains(decimal) || frac_str.contains(thousands) {
        return None;
    }
    if !valid_grouping(int_part, thousands) {
        return None;
    }
    let int_str: String = int_part.chars().filter(char::is_ascii_digit).collect();
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
    let frac_val: i64 = if frac.is_empty() {
        0
    } else {
        frac.parse().ok()?
    };
    let scale = 10i64.checked_pow(exp as u32)?;
    let magnitude = int_val.checked_mul(scale)?.checked_add(frac_val)?;
    Some(if negative { -magnitude } else { magnitude })
}

/// Parse one amount cell for `currency`: refuses a cell whose currency sign
/// names a different currency before reading the number.
fn parse_amount_cell(
    raw: &str,
    currency: Currency,
    decimal_comma: bool,
) -> Result<i64, AmountProblem> {
    if let Some(marker) = currency_marker(raw) {
        if !marker_matches(&marker, currency) {
            return Err(AmountProblem::ForeignCurrency);
        }
    }
    parse_minor_units(raw, currency.exponent(), decimal_comma).ok_or(AmountProblem::Unreadable)
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

/// The date format a whole column of `values` is written in, when the column
/// itself proves it (personal-cfo-tulv): `Y-M-D` when the first part has four
/// digits; otherwise month-first if some value's second part exceeds 12, or
/// day-first if some value's first part does. `None` when the column does not
/// decide (every value fits both orders, the evidence conflicts, or the
/// values mix separators) — the caller then keeps its own default and
/// per-value ambiguity scoring. A value that is not three numeric parts is
/// ignored here: that row is refused by the parse itself, and one bad cell
/// must not cost the rest of the file its date order.
///
/// Deciding once per file is what a per-value guess cannot do: in a
/// day-first file, `05/06/2026` reads as 5 May on its own, but the file's
/// `13/06/2026` settles that it is 5 June.
#[must_use]
pub fn infer_date_format(values: &[String]) -> Option<&'static str> {
    let mut separator = None;
    let (mut year_first, mut month_first, mut day_first) = (false, false, false);
    for value in values {
        let value = value.trim();
        let Some(sep) = value.chars().find(|c| matches!(c, '/' | '.' | '-')) else {
            continue;
        };
        let parts: Vec<&str> = value.split(sep).collect();
        let [a, b, c] = parts.as_slice() else {
            continue;
        };
        let numeric = |p: &&&str| !p.is_empty() && p.chars().all(|ch| ch.is_ascii_digit());
        if ![a, b, c].iter().all(numeric) || (a.len() != 4 && c.len() != 4) {
            continue;
        }
        if *separator.get_or_insert(sep) != sep {
            return None;
        }
        if a.len() == 4 {
            year_first = true;
            continue;
        }
        let (first, second): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
        month_first |= second > 12;
        day_first |= first > 12;
    }
    let sep = separator?;
    match (year_first, month_first, day_first) {
        (true, false, false) => Some(match sep {
            '/' => "%Y/%m/%d",
            '.' => "%Y.%m.%d",
            _ => "%Y-%m-%d",
        }),
        (false, true, false) => Some(match sep {
            '/' => "%m/%d/%Y",
            '.' => "%m.%d.%Y",
            _ => "%m-%d-%Y",
        }),
        (false, false, true) => Some(match sep {
            '/' => "%d/%m/%Y",
            '.' => "%d.%m.%Y",
            _ => "%d-%m-%Y",
        }),
        _ => None,
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
/// A row whose only amounts are zero resolves to `0` (a source such as YNAB
/// writes `$0.00` on the unused side rather than leaving it blank), so it is
/// reported as a zero amount rather than an unreadable one.
fn resolve_amount(
    row: &csv::StringRecord,
    cols: &Columns,
    currency: Currency,
    decimal_comma: bool,
) -> Result<i64, AmountProblem> {
    let cell = |col: Option<usize>| -> Option<Result<i64, AmountProblem>> {
        col.and_then(|i| row.get(i))
            .filter(|v| !v.trim().is_empty())
            .map(|v| parse_amount_cell(v, currency, decimal_comma))
    };
    if let Some(amount) = cell(cols.amount) {
        // A readable signed amount wins; an unreadable one falls through to
        // a debit/credit pair when the source has one (unchanged behavior).
        if amount.is_ok() || (cols.debit.is_none() && cols.credit.is_none()) {
            return amount;
        }
    }
    let debit = cell(cols.debit);
    let credit = cell(cols.credit);
    if matches!(debit, Some(Err(AmountProblem::ForeignCurrency)))
        || matches!(credit, Some(Err(AmountProblem::ForeignCurrency)))
    {
        return Err(AmountProblem::ForeignCurrency);
    }
    match (debit, credit) {
        (Some(Ok(d)), _) if d != 0 => Ok(-d.abs()),
        (_, Some(Ok(c))) if c != 0 => Ok(c.abs()),
        (Some(Ok(_)), _) | (_, Some(Ok(_))) => Ok(0),
        _ => Err(AmountProblem::Unreadable),
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
        preview_columns_with_dialect(input, CsvDialect::default())
    }

    fn parse(&self, input: &ParserInput, hints: &ParserHints) -> Result<ParsedBatch, ParseError> {
        parse_with_dialect(input, hints, CsvDialect::default())
    }
}

register_importer!(GenericCsv);

/// The reader every entry point shares, so the headers [`preview_columns_with_dialect`]
/// returns are exactly what a `ColumnMapping` is matched against (ADR 0045
/// slice 3, personal-cfo-4d8.24.1.2).
fn reader(input: &ParserInput, dialect: CsvDialect) -> csv::Reader<&[u8]> {
    csv::ReaderBuilder::new()
        .delimiter(dialect.delimiter)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(input.bytes.as_slice())
}

/// The source column headers of `input`, read with `dialect`.
#[must_use]
pub fn preview_columns_with_dialect(input: &ParserInput, dialect: CsvDialect) -> Vec<String> {
    reader(input, dialect)
        .headers()
        .map(|h| h.iter().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Every non-empty value in the column named `column` (case-insensitive), in
/// source order — for an importer that decides something about the whole file
/// from one column before parsing it (personal-cfo-tulv: YNAB's date order).
#[must_use]
pub fn column_values(input: &ParserInput, dialect: CsvDialect, column: &str) -> Vec<String> {
    let mut reader = reader(input, dialect);
    let Some(index) = reader.headers().ok().and_then(|headers| {
        headers
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(column))
    }) else {
        return Vec::new();
    };
    reader
        .records()
        .filter_map(Result::ok)
        .filter_map(|row| row.get(index).map(str::to_owned))
        .filter(|value| !value.is_empty())
        .collect()
}

/// The generic CSV parse, for a file written in `dialect` (personal-cfo-tulv).
/// [`GenericCsv`] is this with [`CsvDialect::default`].
///
/// # Errors
/// [`ParseError::Malformed`] for an unreadable header;
/// [`ParseError::Unsupported`] when no date or amount column resolves.
#[allow(clippy::too_many_lines)]
pub fn parse_with_dialect(
    input: &ParserInput,
    hints: &ParserHints,
    dialect: CsvDialect,
) -> Result<ParsedBatch, ParseError> {
    let mut reader = reader(input, dialect);
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
    // Rows not staged. Each reason is fixed text: a skipped row's own
    // values never travel into the import summary (personal-cfo-pxi.10).
    let mut skipped = Vec::new();
    let mut seen_accounts = BTreeSet::new();

    for (idx, result) in reader.records().enumerate() {
        let row = match result {
            Ok(row) => row,
            Err(_) => {
                skipped.push(warn(idx, "unreadable row"));
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
                skipped.push(warn(idx, "currency differs from the import currency"));
                continue;
            }
        }

        let raw_date = row.get(date_col).unwrap_or("").to_owned();
        let Some((posted_date, date_confidence_bps)) =
            parse_date(&raw_date, hints.date_format.as_deref())
        else {
            skipped.push(warn(idx, "unparseable date"));
            continue;
        };
        if date_confidence_bps < 7_000 {
            warnings.push(warn(idx, "ambiguous date (assumed US M/D/Y)"));
        }

        let amount_minor = match resolve_amount(&row, &cols, currency, dialect.decimal_comma) {
            Ok(minor) => minor,
            Err(AmountProblem::ForeignCurrency) => {
                skipped.push(warn(idx, "currency differs from the import currency"));
                continue;
            }
            Err(AmountProblem::Unreadable) => {
                skipped.push(warn(idx, "unparseable / missing amount"));
                continue;
            }
        };
        // A 0.00 row moves no money and the ledger refuses it; skip it with a
        // reason rather than stage a row that can never commit (personal-cfo-pxi.9).
        if amount_minor == 0 {
            skipped.push(warn(idx, "zero amount"));
            continue;
        }

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
                // `ParsedAccount::external_id` below; a routed file import
                // (`stage_parsed_batch_routed`, personal-cfo-tulv) sends the
                // row to the account the user mapped this label to.
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
            currency: None,
        })
        .collect();

    Ok(ParsedBatch {
        source_format: "csv".to_owned(),
        accounts,
        records,
        warnings,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minor_units_handles_money_shapes() {
        let us = |raw| parse_minor_units(raw, 2, false);
        assert_eq!(us("12.99"), Some(1299));
        assert_eq!(us("-12.99"), Some(-1299));
        assert_eq!(us("(42.00)"), Some(-4200));
        assert_eq!(us("$1,234.56"), Some(123_456));
        assert_eq!(us("1000"), Some(100_000));
        assert_eq!(us(".50"), Some(50));
        assert_eq!(us("  $ 1,000.00 "), Some(100_000));
        assert_eq!(us("$1,234,567.89"), Some(123_456_789));
        assert_eq!(us("-$12.00"), Some(-1200));
        assert_eq!(us("$-12.00"), Some(-1200));
        assert_eq!(us(""), None);
        assert_eq!(us("n/a"), None);
        assert_eq!(us("1.2.3"), None);
        // Exponent-0 currency would scale differently (defensive).
        assert_eq!(parse_minor_units("1500", 0, false), Some(1500));
    }

    #[test]
    fn a_separator_in_an_impossible_place_is_refused_not_rescaled() {
        // personal-cfo-tulv: before this, "12,50" read US-style was 1250.00 —
        // a decimal-comma value silently multiplied by 100.
        let us = |raw| parse_minor_units(raw, 2, false);
        assert_eq!(us("12,50"), None);
        assert_eq!(us("1.234,56"), None);
        assert_eq!(us("1,23,456.00"), None);
        assert_eq!(us(",500.00"), None);
        assert_eq!(us("1234,567.00"), None);
        // A genuine US grouping still reads.
        assert_eq!(us("1,234"), Some(123_400));
    }

    #[test]
    fn decimal_comma_reads_european_amounts() {
        let eu = |raw| parse_minor_units(raw, 2, true);
        assert_eq!(eu("1.234,56"), Some(123_456));
        assert_eq!(eu("1.234,56€"), Some(123_456));
        assert_eq!(eu("-84,23€"), Some(-8423));
        assert_eq!(eu("€0,50"), Some(50));
        assert_eq!(eu("12"), Some(1200));
        assert_eq!(eu("1 234,56"), Some(123_456));
        // A US-style value in a decimal-comma file is refused, not misread.
        assert_eq!(eu("1,234.56"), None);
        assert_eq!(eu("12.50"), None);
    }

    #[test]
    fn a_foreign_currency_sign_is_refused_never_relabeled() {
        assert_eq!(parse_amount_cell("$12.00", Currency::Usd, false), Ok(1200));
        assert_eq!(
            parse_amount_cell("US$12.00", Currency::Usd, false),
            Ok(1200)
        );
        assert_eq!(parse_amount_cell("12,00€", Currency::Eur, true), Ok(1200));
        for (raw, currency) in [
            ("€12.00", Currency::Usd),
            ("C$12.00", Currency::Usd),
            ("£12.00", Currency::Usd),
            ("$12,00", Currency::Eur),
        ] {
            assert_eq!(
                parse_amount_cell(raw, currency, currency == Currency::Eur),
                Err(AmountProblem::ForeignCurrency),
                "{raw} as {currency:?}"
            );
        }
        // Letters alone are not a currency sign (a bank's CR/DR suffix).
        assert_eq!(
            parse_amount_cell("12.00 CR", Currency::Usd, false),
            Ok(1200)
        );
        // A debit/credit indicator beside a sign is not part of the currency
        // (04-review F1, PR #70): these imported before personal-cfo-tulv.
        for (raw, minor) in [
            ("$1,234.56 CR", 123_456),
            ("$1,234.56 DR", 123_456),
            ("$12.00CR", 1200),
            ("$12.00 cr", 1200),
            ("CR $12.00", 1200),
        ] {
            assert_eq!(
                parse_amount_cell(raw, Currency::Usd, false),
                Ok(minor),
                "{raw} must stay a dollar amount"
            );
        }
        // A letter run touching the sign still names its own currency.
        assert_eq!(
            parse_amount_cell("C$12.00 CR", Currency::Usd, false),
            Err(AmountProblem::ForeignCurrency)
        );
    }

    #[test]
    fn infer_date_format_decides_from_the_whole_column() {
        let col = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            infer_date_format(&col(&["05/06/2026", "13/06/2026"])),
            Some("%d/%m/%Y")
        );
        assert_eq!(
            infer_date_format(&col(&["05/06/2026", "06/13/2026"])),
            Some("%m/%d/%Y")
        );
        assert_eq!(
            infer_date_format(&col(&["2026-06-05", "2026-06-13"])),
            Some("%Y-%m-%d")
        );
        assert_eq!(
            infer_date_format(&col(&["05.06.2026", "13.06.2026"])),
            Some("%d.%m.%Y")
        );
        // Undecided, conflicting, or mixed shapes keep the caller's default.
        assert_eq!(infer_date_format(&col(&["05/06/2026", "01/02/2026"])), None);
        assert_eq!(infer_date_format(&col(&["13/06/2026", "06/13/2026"])), None);
        assert_eq!(infer_date_format(&col(&["13/06/2026", "2026-06-13"])), None);
        // A malformed cell is ignored, not allowed to undecide the file.
        assert_eq!(
            infer_date_format(&col(&["13/06/2026", "June 9th", "1/2"])),
            Some("%d/%m/%Y")
        );
        assert_eq!(infer_date_format(&col(&["bogus"])), None);
        assert_eq!(infer_date_format(&[]), None);
    }

    #[test]
    fn a_tab_delimited_decimal_comma_file_parses_with_its_dialect() {
        let tsv = "Date\tPayee\tOutflow\tInflow\n\
                   13/06/2026\tBäckerei\t1.234,56€\t0,00€\n\
                   14/06/2026\tGehalt\t0,00€\t2.500,00€\n";
        let hints = ParserHints {
            column_mapping: Some(ColumnMapping {
                date: Some("Date".to_owned()),
                description: Some("Payee".to_owned()),
                debit: Some("Outflow".to_owned()),
                credit: Some("Inflow".to_owned()),
                ..ColumnMapping::default()
            }),
            date_format: Some("%d/%m/%Y".to_owned()),
            default_currency: Some(Currency::Eur),
            institution: None,
        };
        let dialect = CsvDialect {
            delimiter: b'\t',
            decimal_comma: true,
        };
        let batch = parse_with_dialect(&input(tsv), &hints, dialect).unwrap();
        assert!(batch.skipped.is_empty(), "{:?}", batch.skipped);
        let amounts: Vec<_> = batch
            .records
            .iter()
            .map(|r| r.transaction.as_ref().unwrap().amount)
            .collect();
        assert_eq!(
            amounts,
            vec![
                Money::new(-123_456, Currency::Eur),
                Money::new(250_000, Currency::Eur)
            ]
        );
        assert_eq!(
            preview_columns_with_dialect(&input(tsv), dialect),
            vec!["Date", "Payee", "Outflow", "Inflow"]
        );
        assert_eq!(column_values(&input(tsv), dialect, "date").len(), 2);
    }

    #[test]
    fn a_row_whose_debit_and_credit_are_both_zero_is_a_zero_amount() {
        // YNAB writes "$0.00" on the unused side; a row with both sides zero
        // (a zero starting balance) is a zero amount, not an unreadable one.
        let csv = "Date,Payee,Outflow,Inflow\n2026-06-01,Starting Balance,$0.00,$0.00\n2026-06-02,Store,$5.00,$0.00\n";
        let hints = ParserHints {
            column_mapping: Some(ColumnMapping {
                date: Some("Date".to_owned()),
                description: Some("Payee".to_owned()),
                debit: Some("Outflow".to_owned()),
                credit: Some("Inflow".to_owned()),
                ..ColumnMapping::default()
            }),
            ..ParserHints::default()
        };
        let batch = GenericCsv.parse(&input(csv), &hints).unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.skipped[0].message, "zero amount");
        assert_eq!(
            batch.records[0].transaction.as_ref().unwrap().amount,
            Money::new(-500, Currency::Usd)
        );
    }

    #[test]
    fn a_bank_file_s_cr_dr_suffix_still_imports() {
        // 04-review F1 (PR #70): a generic bank CSV writing a debit/credit
        // indicator after a dollar amount imported on main and must still.
        let csv = "Date,Description,Amount\n\
                   2026-06-20,Deposit,\"$1,234.56 CR\"\n\
                   2026-06-21,Payment,\"$1,234.56 DR\"\n\
                   2026-06-22,Refund,$12.00CR\n\
                   2026-06-23,Abroad,C$12.00\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 3, "{:?}", batch.skipped);
        assert_eq!(batch.skipped.len(), 1);
        assert_eq!(batch.skipped[0].row, Some(3));
        assert_eq!(
            batch.skipped[0].message,
            "currency differs from the import currency"
        );
    }

    #[test]
    fn a_row_in_another_currency_sign_is_skipped_with_the_currency_reason() {
        let csv = "Date,Description,Amount\n2026-06-20,Cafe,-$3.00\n2026-06-21,Hotel,-€80.00\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.skipped.len(), 1);
        assert_eq!(batch.skipped[0].row, Some(1));
        assert_eq!(
            batch.skipped[0].message,
            "currency differs from the import currency"
        );
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
    fn a_malformed_row_is_reported_as_skipped_not_a_transaction() {
        let csv = "Date,Description,Amount\n2026-06-20,Good,-10.00\nbogus,Bad Row,xyz\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 1, "only the valid row is staged");
        assert_eq!(batch.skipped.len(), 1, "the bad row is reported");
        assert_eq!(batch.skipped[0].row, Some(1));
        assert!(batch.warnings.is_empty(), "a skip is not a staged-row note");
    }

    #[test]
    fn every_skipped_row_is_reported_with_a_fixed_reason_and_none_of_its_values() {
        // personal-cfo-pxi.10: one skip per reason, each row carrying values
        // that must not reach the summary.
        let csv = "Date,Description,Amount,Currency\n\
                   2026-06-20,Good,-10.00,USD\n\
                   2026-06-21,SENTINEL-MERCHANT,-12.00,EUR\n\
                   SENTINEL-DATE,Payee Two,-13.00,USD\n\
                   2026-06-22,Payee Three,SENTINEL-NOT-A-NUMBER,USD\n\
                   2026-06-23,Payee Four,,USD\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 1, "{:#?}", batch.records);
        let skipped: Vec<(Option<usize>, &str)> = batch
            .skipped
            .iter()
            .map(|w| (w.row, w.message.as_str()))
            .collect();
        assert_eq!(
            skipped,
            vec![
                (Some(1), "currency differs from the import currency"),
                (Some(2), "unparseable date"),
                (Some(3), "unparseable / missing amount"),
                (Some(4), "unparseable / missing amount"),
            ]
        );
        let reported = format!("{:?}{:?}", batch.skipped, batch.warnings);
        for value in ["SENTINEL", "EUR", "Payee", "2026-06"] {
            assert!(!reported.contains(value), "{value} leaked: {reported}");
        }
    }

    #[test]
    fn a_zero_amount_row_is_skipped_with_a_reason() {
        // personal-cfo-pxi.9: 0.00 in the amount column, and in a debit/credit pair.
        let csv = "Date,Description,Amount\n2026-06-20,Fee waived,0.00\n2026-06-21,Coffee,-3.50\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 1, "only the non-zero row is staged");
        assert_eq!(batch.skipped.len(), 1);
        assert_eq!(batch.skipped[0].row, Some(0));
        assert_eq!(batch.skipped[0].message, "zero amount");

        let csv = "Date,Description,Amount\n2026-06-20,Adjustment,-0.00\n";
        let batch = GenericCsv
            .parse(&input(csv), &ParserHints::default())
            .unwrap();
        assert!(batch.records.is_empty());
        assert_eq!(batch.skipped[0].message, "zero amount");
    }

    #[test]
    fn an_unreadable_row_is_skipped_without_echoing_it() {
        let mut bytes = b"Date,Description,Amount\n2026-06-20,Good,-10.00\n2026-06-21,".to_vec();
        bytes.extend_from_slice(b"SENTINEL\xff\xfe,-1.00\n");
        let batch = GenericCsv
            .parse(&ParserInput::new(bytes), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.skipped.len(), 1);
        assert_eq!(batch.skipped[0].message, "unreadable row");
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
        assert!(
            batch.warnings.iter().all(|w| !w.message.contains("05/06")),
            "the note names the reason, not the row's date"
        );
        assert!(
            batch.skipped.is_empty(),
            "an ambiguous date is still staged"
        );
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
