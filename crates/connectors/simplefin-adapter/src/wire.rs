//! SimpleFIN wire shapes + strict field parsing, clean-room from the protocol
//! spec (v1.0.7 and v2.0.0-draft; docs/research/simplefin-feasibility.md).
//!
//! One lenient envelope covers both spec versions: v1 responses carry an
//! `errors` string array and a per-account `org`; v2 (`?version=2`) carries a
//! structured `errlist` and per-account `conn_id`. Unknown fields (the
//! Bridge's `holdings`, `payee`, `memo`, `mcc` extensions) are ignored until a
//! staged shape exists for them (holdings: bead personal-cfo-kmw5).

use chrono::{DateTime, NaiveDate};
use core_money::Currency;
use serde::Deserialize;

/// The `GET /accounts` envelope — v1 and v2 fields side by side, all
/// defaulted, so either shape parses. A Bridge 403 body is *also* this shape.
#[derive(Debug, Clone, Deserialize)]
pub struct AccountSet {
    /// v1: display-ready error strings (must be sanitized before display).
    #[serde(default)]
    pub errors: Vec<String>,
    /// v2: structured errors with spec-level codes (`gen.auth`, `con.auth`, …).
    #[serde(default)]
    pub errlist: Vec<WireError>,
    /// v2: one entry per institution login; accounts reference these by
    /// `conn_id`. Two logins at one bank are two Connections.
    #[serde(default)]
    pub connections: Vec<WireConnection>,
    #[serde(default)]
    pub accounts: Vec<WireAccount>,
}

/// A v2 Connection — the institution-login scope that account ids are unique
/// within.
#[derive(Debug, Clone, Deserialize)]
pub struct WireConnection {
    pub conn_id: String,
    /// Human-friendly, includes the institution name.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub org_name: Option<String>,
}

/// A v2 structured error. Codes are `prefix.[subcode]` with prefixes
/// `gen`/`con`/`act`; unknown subcodes fall back to the naked prefix.
#[derive(Debug, Clone, Deserialize)]
pub struct WireError {
    pub code: String,
    pub msg: String,
    #[serde(default)]
    pub conn_id: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WireAccount {
    /// Unique within the connection/organization — not globally.
    pub id: String,
    pub name: String,
    /// ISO-4217 code, or a URL for a custom currency (unsupported here).
    pub currency: String,
    /// Numeric string, e.g. `"-33293.43"`.
    pub balance: String,
    #[serde(rename = "balance-date")]
    pub balance_date: i64,
    #[serde(default)]
    pub transactions: Vec<WireTransaction>,
    /// v1 institution object.
    #[serde(default)]
    pub org: Option<WireOrg>,
    /// v2: the Connection this account belongs to. Account ids are unique
    /// only within a connection — identity keys must include it.
    #[serde(default)]
    pub conn_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WireOrg {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WireTransaction {
    /// Never reused within an account; may repeat across accounts.
    pub id: String,
    /// Epoch seconds; may be `0` for pending transactions.
    pub posted: i64,
    /// Numeric string; positive = deposit into the account.
    pub amount: String,
    pub description: String,
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub transacted_at: Option<i64>,
}

/// Map an ISO-4217 code to a supported [`Currency`]. `None` for unsupported
/// codes and for custom-currency URLs (the spec's "currency is a URL" case).
/// The `sync` path surfaces such records as batch warnings; the per-account
/// `fetch_*` trait methods have no warnings channel yet (connector-core
/// follow-up bead) and drop them with a code comment at the call site —
/// never mispriced, but only `sync` reports the reason today.
#[must_use]
pub fn currency_from_code(code: &str) -> Option<Currency> {
    match code.trim().to_ascii_uppercase().as_str() {
        "USD" => Some(Currency::Usd),
        "EUR" => Some(Currency::Eur),
        _ => None,
    }
}

/// Parse a SimpleFIN "numeric string" amount into minor units, strictly:
/// optional single leading `-`, ASCII digits, at most one `.`, no symbols, no
/// thousands separators (the observed wire grammar). Fractional digits beyond
/// the currency exponent are a REJECTION, not a rounding — money is never
/// silently rounded. Fewer digits pad (`"105884.8"` → `10588480` at exp 2).
#[must_use]
pub fn parse_amount_minor(raw: &str, exponent: u8) -> Option<i64> {
    let body = raw.strip_prefix('-').unwrap_or(raw);
    if body.is_empty() {
        return None;
    }
    let (int_str, frac_str) = body.split_once('.').unwrap_or((body, ""));
    if int_str.is_empty() && frac_str.is_empty() {
        return None;
    }
    if !int_str.chars().all(|c| c.is_ascii_digit()) || !frac_str.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let exp = usize::from(exponent);
    if frac_str.len() > exp {
        return None;
    }
    let mut frac = frac_str.to_owned();
    frac.push_str(&"0".repeat(exp - frac_str.len()));
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
    let scale = 10_i64.checked_pow(u32::try_from(exp).ok()?)?;
    let magnitude = int_val.checked_mul(scale)?.checked_add(frac_val)?;
    Some(if raw.starts_with('-') {
        -magnitude
    } else {
        magnitude
    })
}

/// Epoch seconds (UTC) → calendar date. `None` for out-of-range values and
/// for `0`, which SimpleFIN uses as the pending-transaction sentinel.
#[must_use]
pub fn epoch_to_date(secs: i64) -> Option<NaiveDate> {
    if secs <= 0 {
        return None;
    }
    DateTime::from_timestamp(secs, 0).map(|dt| dt.date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_parse_strictly() {
        assert_eq!(parse_amount_minor("-33293.43", 2), Some(-3_329_343));
        assert_eq!(parse_amount_minor("100.23", 2), Some(10_023));
        assert_eq!(parse_amount_minor("0.00", 2), Some(0));
        // Fewer decimals than the exponent pad (observed Bridge holding).
        assert_eq!(parse_amount_minor("105884.8", 2), Some(10_588_480));
        assert_eq!(parse_amount_minor("7", 2), Some(700));
        assert_eq!(parse_amount_minor(".5", 2), Some(50));
        // Rejections: more precision than the currency, junk, signs we never
        // observed, separators.
        assert_eq!(parse_amount_minor("12.345", 2), None);
        assert_eq!(parse_amount_minor("1.2.3", 2), None);
        assert_eq!(parse_amount_minor("+1.00", 2), None);
        assert_eq!(parse_amount_minor("1,000.00", 2), None);
        assert_eq!(parse_amount_minor("$5.00", 2), None);
        assert_eq!(parse_amount_minor("", 2), None);
        assert_eq!(parse_amount_minor("-", 2), None);
        assert_eq!(parse_amount_minor(".", 2), None);
    }

    #[test]
    fn custom_currency_urls_are_unsupported_not_misparsed() {
        assert_eq!(currency_from_code("USD"), Some(Currency::Usd));
        assert_eq!(currency_from_code(" eur "), Some(Currency::Eur));
        assert_eq!(currency_from_code("ZMW"), None);
        assert_eq!(
            currency_from_code("https://www.example.com/flight-miles"),
            None
        );
    }

    #[test]
    fn epoch_zero_is_the_pending_sentinel_not_1970() {
        assert_eq!(epoch_to_date(0), None);
        assert_eq!(
            epoch_to_date(1_755_000_000),
            NaiveDate::from_ymd_opt(2025, 8, 12)
        );
    }

    #[test]
    fn both_envelope_shapes_parse() {
        let v1 = r#"{"errors":["Forbidden"],"accounts":[]}"#;
        let set: AccountSet = serde_json::from_str(v1).unwrap();
        assert_eq!(set.errors, vec!["Forbidden"]);
        assert!(set.errlist.is_empty());

        let v2 =
            r#"{"errlist":[{"code":"gen.auth","msg":"Forbidden"}],"accounts":[],"connections":[]}"#;
        let set: AccountSet = serde_json::from_str(v2).unwrap();
        assert_eq!(set.errlist[0].code, "gen.auth");

        // Bridge extensions (holdings/payee/memo/mcc/org extras) are ignored.
        let live = r#"{"errors":[],"accounts":[{"id":"ACT-1","name":"Checking",
            "currency":"USD","balance":"100.23","available-balance":"100.23",
            "balance-date":1755000000,"org":{"domain":"demo.bank","name":"Demo",
            "sfin-url":"https://x/simplefin","url":"https://demo.bank","id":"demo"},
            "holdings":[],"transactions":[{"id":"TXN-1","posted":1755000000,
            "amount":"-5.00","description":"Coffee","payee":"CAFE","memo":"m",
            "mcc":"5812","extra":{}}]}]}"#;
        let set: AccountSet = serde_json::from_str(live).unwrap();
        assert_eq!(set.accounts[0].transactions[0].id, "TXN-1");
        assert_eq!(
            set.accounts[0].org.as_ref().unwrap().name.as_deref(),
            Some("Demo")
        );
    }
}
