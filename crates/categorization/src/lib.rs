//! Merchant normalization v1 (personal-cfo-7yh0, plan §11.2).
//!
//! Pure + deterministic: a raw bank/card description ("SQ *BLUE BOTTLE COFFEE OAKLAND CA",
//! "AMZN MKTP US*1A2B3C") becomes a canonical merchant key ("BLUE BOTTLE COFFEE OAKLAND",
//! "AMZN MKTP US"). The key is **stable across a merchant's repeat transactions** — the
//! per-transaction noise that varies (auth/reference codes, store numbers) is stripped — so
//! it is the join key the merchant-memory auto-categorization, rules, and classifier build
//! on (personal-cfo-5n4).
//!
//! v1 is the deterministic *string* layer. The canonical merchant-**identity** entity
//! tables + fuzzy alias matching (so "AMZN MKTP" and "AMAZON.COM" resolve to one identity,
//! and one Starbucks key spans locations) are the v2 entity layer, which needs the merchant
//! schema (personal-cfo-zrpg). v1 keeps the city, so its key is per-location — safe, never
//! mis-grouping; v2 does the cross-location grouping.

pub mod merchant_grouping;
pub mod recurring_detection;
pub mod spend_classifier;

pub use merchant_grouping::{brand_anchor, keys_conflict, same_merchant, truncation_variant};
pub use recurring_detection::{
    amount_band_minor, detect_recurring, Observation, RecurringCandidate,
};
pub use spend_classifier::{classify, CategoryProfile, ClassReason, Classification, SpendClass};

use std::sync::OnceLock;

use regex::Regex;

/// Normalize a raw transaction description into a canonical merchant key (see module docs).
/// Idempotent: `normalize_merchant(&normalize_merchant(x)) == normalize_merchant(x)`.
#[must_use]
pub fn normalize_merchant(raw: &str) -> String {
    let mut s = raw.trim().to_uppercase();
    s = processor_prefix().replace(&s, "").into_owned();
    s = reference_code().replace_all(&s, " ").into_owned();
    s = trailing_state().replace(&s, "").into_owned();
    s = collapse_ws().replace_all(&s, " ").into_owned();
    s.trim()
        .trim_matches(|c: char| matches!(c, '-' | '*' | '.' | ',' | '#' | '/' | '&'))
        .trim()
        .to_owned()
}

/// Leading payment-processor / POS prefixes, one or more chained: asterisk-style ("SQ *",
/// "TST*", "PP*", "PAYPAL *") and word-style ("VENMO ", "POS ", "DEBIT CARD PURCHASE ").
fn processor_prefix() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?:(?:SQ|TST|PP|PYPL|PAYPAL|SP|VEN|CKE|GOOGLE)\s?\*|(?:VENMO|ZELLE|CASH ?APP|POS|PURCHASE|DEBIT(?: CARD)?(?: PURCHASE)?|CHECKCARD|PRE-?AUTH|RECURRING)\s+)+",
        )
        .unwrap()
    })
}

/// Per-transaction noise that must not enter the key: asterisk-prefixed auth/reference
/// codes ("*1A2B3C"), store numbers ("#1234", "STORE 0123"), and bare long digit runs.
fn reference_code() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\*[A-Z0-9]{3,}|#\s?\d+|\bSTORE\s+\d+\b|\b\d{4,}\b").unwrap())
}

/// A trailing US state abbreviation (the tail of a "... CITY ST" location). Conservative:
/// only the 2-letter state token is stripped (the city stays, keeping a per-location key).
fn trailing_state() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\s+(?:A[KLRZ]|C[AOT]|D[CE]|FL|GA|HI|I[ADLN]|K[SY]|LA|M[ADEINOST]|N[CDEHJMVY]|O[HKR]|PA|RI|S[CD]|T[NX]|UT|V[AT]|W[AIVY])$",
        )
        .unwrap()
    })
}

/// Runs of whitespace, collapsed to a single space.
fn collapse_ws() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s+").unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A labeled fixture of real-world raw → canonical pairs (personal-cfo-7yh0 AC). The
    /// invariant that matters for auto-categorization: a merchant's repeat transactions
    /// (which differ only in auth/reference codes) collapse to one key.
    #[test]
    fn normalizes_real_world_descriptions() {
        let cases = [
            (
                "SQ *BLUE BOTTLE COFFEE OAKLAND CA",
                "BLUE BOTTLE COFFEE OAKLAND",
            ),
            ("TST* TACO BELL #1234", "TACO BELL"),
            ("AMZN MKTP US*1A2B3C", "AMZN MKTP US"),
            ("PAYPAL *SPOTIFY", "SPOTIFY"),
            ("PP*UBER EATS", "UBER EATS"),
            ("SHELL OIL 57521234 HOUSTON TX", "SHELL OIL HOUSTON"),
            ("7-ELEVEN 35021 SAN JOSE CA", "7-ELEVEN SAN JOSE"),
            ("COSTCO WHSE #0455 SEATTLE WA", "COSTCO WHSE SEATTLE"),
            ("TRADER JOE'S #123 PORTLAND OR", "TRADER JOE'S PORTLAND"),
            ("DEBIT CARD PURCHASE STARBUCKS", "STARBUCKS"),
            ("POS PURCHASE WHOLEFDS MKT 10259", "WHOLEFDS MKT"),
            ("netflix.com", "NETFLIX.COM"),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalize_merchant(raw), expected, "raw: {raw:?}");
        }
    }

    #[test]
    fn repeat_transactions_collapse_to_one_key() {
        // The same coffee shop on three days, each with a different auth code → one key.
        let a = normalize_merchant("SQ *RITUAL COFFEE *A1B2C3 SF CA");
        let b = normalize_merchant("SQ *RITUAL COFFEE *Z9Y8X7 SF CA");
        let c = normalize_merchant("SQ *RITUAL COFFEE *0Q1W2E SF CA");
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(a, "RITUAL COFFEE SF");
    }

    #[test]
    fn is_idempotent() {
        for raw in [
            "SQ *BLUE BOTTLE COFFEE OAKLAND CA",
            "AMZN MKTP US*1A2B3C",
            "POS PURCHASE WHOLEFDS MKT 10259",
            "netflix.com",
        ] {
            let once = normalize_merchant(raw);
            assert_eq!(normalize_merchant(&once), once, "not idempotent: {raw:?}");
        }
    }

    #[test]
    fn blank_in_blank_out() {
        assert_eq!(normalize_merchant("   "), "");
        assert_eq!(normalize_merchant(""), "");
    }
}
