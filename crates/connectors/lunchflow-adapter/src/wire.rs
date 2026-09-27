//! LunchFlow Personal API wire shapes (lunchflow.app/docs/api, read
//! 2026-09-27), deserialized tolerantly: unknown fields are ignored, optional
//! fields default, and ids may arrive as JSON numbers or strings. Provider
//! payloads are hostile input — every string that reaches a message is
//! sanitized by the caller.
//!
//! Money: `amount` is a JSON number. It is captured as its raw JSON text
//! ([`RawValue`]) and parsed as an exact decimal — never through a float — and
//! more fractional digits than the currency allows is a rejection, not a
//! rounding.

use chrono::NaiveDate;
use core_money::Currency;
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;

/// `GET /accounts`. The `accounts` array is REQUIRED: a 200 without it is a
/// changed response shape, which must fail loudly — never read as "this key
/// sees no accounts" (found by the owner's live drill, 2026-09-27).
#[derive(Debug, Deserialize)]
pub struct AccountList {
    pub accounts: Vec<WireAccount>,
}

#[derive(Debug, Deserialize)]
pub struct WireAccount {
    #[serde(deserialize_with = "id_string")]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub institution_name: Option<String>,
    /// ISO 4217, as LunchFlow reports it.
    #[serde(default)]
    pub currency: Option<String>,
    /// `"ACTIVE"` when the bank connection is healthy.
    #[serde(default)]
    pub status: Option<String>,
}

impl WireAccount {
    /// Missing status is read as active: the field is documented but its
    /// non-active values are not.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.status
            .as_deref()
            .is_none_or(|s| s.trim().eq_ignore_ascii_case("active"))
    }
}

/// `GET /accounts/{id}/transactions`.
#[derive(Debug, Deserialize)]
pub struct TransactionList {
    /// Required, like `AccountList::accounts`: missing is a shape change.
    pub transactions: Vec<WireTransaction>,
    /// How many transactions matched. More than were returned means the
    /// response was truncated — the caller holds that account's watermark.
    #[serde(default)]
    pub total: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireTransaction {
    #[serde(deserialize_with = "id_string")]
    pub id: String,
    pub amount: Box<RawValue>,
    #[serde(default)]
    pub currency: Option<String>,
    pub date: String,
    #[serde(default)]
    pub merchant: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_pending: bool,
}

/// `GET /accounts/{id}/balance`.
#[derive(Debug, Deserialize)]
pub struct BalanceEnvelope {
    pub balance: WireBalance,
}

#[derive(Debug, Deserialize)]
pub struct WireBalance {
    pub amount: Box<RawValue>,
    #[serde(default)]
    pub currency: Option<String>,
}

/// An error body: `{ "error": "...", "message": "..." }`.
#[derive(Debug, Deserialize)]
pub struct ErrorBody {
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

/// Accept an id as a JSON number (documented for accounts) or a string
/// (documented for transactions); either way it becomes an opaque string.
fn id_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Unsigned(u64),
        Signed(i64),
        Text(String),
    }
    Ok(match Id::deserialize(deserializer)? {
        Id::Unsigned(n) => n.to_string(),
        Id::Signed(n) => n.to_string(),
        Id::Text(s) => s,
    })
}

/// Map an ISO 4217 code to a supported [`Currency`]. `None` for codes the app
/// cannot hold yet — the caller surfaces those rows as warnings.
#[must_use]
pub fn currency_from_code(code: &str) -> Option<Currency> {
    match code.trim().to_ascii_uppercase().as_str() {
        "USD" => Some(Currency::Usd),
        "EUR" => Some(Currency::Eur),
        _ => None,
    }
}

/// An ISO 4217-shaped code (three ASCII letters), uppercased — or `None`.
#[must_use]
pub fn iso_code(raw: &str) -> Option<String> {
    let code = raw.trim().to_ascii_uppercase();
    (code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase())).then_some(code)
}

/// The exact decimal text of a JSON amount: a bare number literal, or a
/// quoted decimal string (tolerated in case the provider ever sends one).
#[must_use]
pub fn amount_text(raw: &RawValue) -> &str {
    let text = raw.get().trim();
    text.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(text)
}

/// Parse a decimal amount into minor units, strictly: optional single leading
/// `-`, ASCII digits, at most one `.`. No exponent, no symbols, no thousands
/// separators. Fractional digits beyond the currency exponent are a
/// REJECTION, not a rounding — money is never silently rounded.
#[must_use]
pub fn parse_amount_minor(raw: &str, exponent: u8) -> Option<i64> {
    let (negative, body) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let (int_str, frac_str) = body.split_once('.').unwrap_or((body, ""));
    if int_str.is_empty()
        || !int_str.bytes().all(|b| b.is_ascii_digit())
        || !frac_str.bytes().all(|b| b.is_ascii_digit())
        || (body.contains('.') && frac_str.is_empty())
    {
        return None;
    }
    let exp = usize::from(exponent);
    if frac_str.len() > exp {
        return None;
    }
    let mut digits = int_str.to_owned();
    digits.push_str(frac_str);
    digits.push_str(&"0".repeat(exp - frac_str.len()));
    let magnitude: i64 = digits.parse().ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

/// A transaction date: `YYYY-MM-DD`, or the date part of an ISO 8601
/// timestamp.
#[must_use]
pub fn parse_date(raw: &str) -> Option<NaiveDate> {
    let date = raw.trim().split('T').next().unwrap_or("");
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

/// Epoch seconds → UTC calendar date.
#[must_use]
pub fn epoch_to_date(epoch: i64) -> Option<NaiveDate> {
    chrono::DateTime::from_timestamp(epoch, 0).map(|dt| dt.date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_parse_exactly_or_not_at_all() {
        let cases = [
            ("12.34", Some(1234)),
            ("-12.34", Some(-1234)),
            ("-0.5", Some(-50)),
            ("7", Some(700)),
            ("105884.8", Some(10_588_480)),
            ("0", Some(0)),
            ("12.345", None), // more precision than cents: reject, never round
            ("1e3", None),
            ("1.2e3", None),
            ("", None),
            ("-", None),
            (".5", None),
            ("5.", None),
            ("1,000.00", None),
            ("$5.00", None),
            ("--5", None),
            ("+5", None),
        ];
        for (raw, expected) in cases {
            assert_eq!(parse_amount_minor(raw, 2), expected, "input {raw:?}");
        }
    }

    #[test]
    fn amount_text_reads_numbers_and_quoted_decimals() {
        let number: Box<RawValue> = serde_json::from_str("-45.10").unwrap();
        let quoted: Box<RawValue> = serde_json::from_str("\"-45.10\"").unwrap();
        assert_eq!(amount_text(&number), "-45.10");
        assert_eq!(amount_text(&quoted), "-45.10");
    }

    #[test]
    fn ids_accept_numbers_and_strings() {
        let a: WireAccount = serde_json::from_str(r#"{"id": 4242}"#).unwrap();
        let b: WireAccount = serde_json::from_str(r#"{"id": "acct_9"}"#).unwrap();
        assert_eq!(a.id, "4242");
        assert_eq!(b.id, "acct_9");
    }

    #[test]
    fn dates_accept_plain_dates_and_timestamps() {
        let d = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        assert_eq!(parse_date("2026-09-01"), Some(d));
        assert_eq!(parse_date("2026-09-01T13:45:00Z"), Some(d));
        assert_eq!(parse_date("09/01/2026"), None);
    }

    #[test]
    fn iso_codes_are_three_letters() {
        assert_eq!(iso_code(" gbp "), Some("GBP".to_owned()));
        assert_eq!(iso_code("US"), None);
        assert_eq!(iso_code("U$D"), None);
    }
}
