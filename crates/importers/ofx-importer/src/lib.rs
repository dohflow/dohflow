//! OFX / QFX importer (personal-cfo-fr79) — the second real [`ImporterPlugin`].
//!
//! Turns an OFX statement download (untrusted bytes) into a typed
//! [`ParsedBatch`] of staged candidates. Both eras are handled: **OFX 1.x**
//! (SGML — `OFXHEADER:100` key:value header lines, tags commonly left
//! unclosed) and **OFX 2.x** (XML — `<?xml`/`<?OFX` prologue, closed tags).
//! Rather than a real SGML/XML parser, a hand-rolled scanner locates
//! `<STMTTRN>` blocks case-insensitively and reads each tag's value as the
//! text after `<TAG>` up to the next `<` — which covers closed and unclosed
//! forms uniformly. `FITID` (the bank's stable transaction id) becomes both
//! the record's `external_id` and the anchor of the dedupe fingerprint; a
//! FITID-less transaction falls back to csv-importer's date+amount+merchant
//! scheme. Anything unreadable becomes a [`ParseWarning`] rather than a silent
//! guess. It holds no keys/DB/network/IPC (the eay trait) and runs behind the
//! bounded host (`hs9`, ADR 0022).
//!
//! v1 scope: bytes are decoded as UTF-8 (lossy) — exotic OFX 1.x charsets are
//! read best-effort; amounts are strict OFX decimals (`-12.99`, no thousands
//! separators), so a decimal-comma locale is a documented follow-up.

use core_money::{Currency, Money};
use importer_core::{
    register_importer, ImporterPlugin, ParseError, ParseWarning, ParsedBatch, ParsedRecord,
    ParsedTransaction, ParserHints, ParserInput,
};
use semver::Version;

/// Map a currency code to a [`Currency`], or `None` if unsupported.
fn currency_from_code(code: &str) -> Option<Currency> {
    match code.trim().to_ascii_uppercase().as_str() {
        "USD" => Some(Currency::Usd),
        "EUR" => Some(Currency::Eur),
        _ => None,
    }
}

/// Parse an OFX decimal amount into minor units for a currency with `exponent`
/// decimal places. OFX carries plain signed decimals (`-12.99`, `+7.5`,
/// `1500`): a leading sign, digits, at most one `.`, **no** thousands
/// separators or currency symbols. Anything else is `None` — better a warning
/// than a mis-read amount.
fn parse_minor_units(raw: &str, exponent: u8) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (negative, body) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let (int_str, frac_str) = body.split_once('.').unwrap_or((body, ""));
    if int_str.is_empty() && frac_str.is_empty() {
        return None;
    }
    if !int_str.bytes().all(|b| b.is_ascii_digit()) || !frac_str.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
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

/// Parse a `DTPOSTED` value: the first 8 chars are `YYYYMMDD`; any suffix
/// (time, `[-5:EST]` zone) is ignored. OFX dates are unambiguous, so a
/// successful parse is full confidence — a bad one is a warning + skip,
/// mirroring csv-importer's unparseable-date handling.
fn parse_dtposted(raw: &str) -> Option<chrono::NaiveDate> {
    let head = raw.trim().get(..8)?;
    chrono::NaiveDate::parse_from_str(head, "%Y%m%d").ok()
}

/// Undo the five predefined XML entities (OFX 2.x values may carry them; a
/// 1.x SGML value passes through untouched).
fn unescape_xml(raw: &str) -> String {
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Read `<TAG>`'s value inside a block: the text after the (lowercase) open
/// tag up to the next `<`, trimmed and entity-unescaped. Handles OFX 1.x
/// unclosed tags and 2.x `<TAG>value</TAG>` uniformly. `None` when the tag is
/// absent or its value is empty (e.g. an aggregate like `<PAYEE><NAME>…`).
fn tag_value(block_lower: &str, block_orig: &str, tag_lower: &str) -> Option<String> {
    let open = format!("<{tag_lower}>");
    let at = block_lower.find(&open)?;
    let start = at + open.len();
    let end = block_lower[start..]
        .find('<')
        .map_or(block_lower.len(), |i| start + i);
    let value = unescape_xml(block_orig[start..end].trim());
    (!value.is_empty()).then_some(value)
}

/// `(start, end)` byte ranges of each `<STMTTRN>` block body. A block runs to
/// its `</STMTTRN>`, or — since OFX 1.x SGML often leaves it unclosed — to the
/// next `<STMTTRN>` / `</BANKTRANLIST>` / end of input, whichever comes first.
fn stmttrn_blocks(lower: &str) -> Vec<(usize, usize)> {
    const OPEN: &str = "<stmttrn>";
    const CLOSE: &str = "</stmttrn>";
    const LIST_CLOSE: &str = "</banktranlist>";
    let mut blocks = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = lower[cursor..].find(OPEN) {
        let start = cursor + rel + OPEN.len();
        let rest = &lower[start..];
        let end = [rest.find(OPEN), rest.find(CLOSE), rest.find(LIST_CLOSE)]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(rest.len());
        blocks.push((start, start + end));
        cursor = start + end;
    }
    blocks
}

/// Positions of each `<CURDEF>` and its currency, so a transaction takes the
/// nearest *preceding* statement currency (a file can hold several statement
/// blocks). An unsupported code rejects the file — better than silently
/// booking foreign amounts under the wrong currency.
fn curdef_positions(lower: &str, orig: &str) -> Result<Vec<(usize, Currency)>, ParseError> {
    const TAG: &str = "<curdef>";
    let mut curdefs = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = lower[cursor..].find(TAG) {
        let start = cursor + rel + TAG.len();
        let end = lower[start..].find('<').map_or(lower.len(), |i| start + i);
        let code = orig[start..end].trim();
        if !code.is_empty() {
            let currency = currency_from_code(code).ok_or_else(|| {
                ParseError::Unsupported(format!("unsupported statement currency {code:?}"))
            })?;
            curdefs.push((start, currency));
        }
        cursor = end;
    }
    Ok(curdefs)
}

fn warn(row: usize, message: impl Into<String>) -> ParseWarning {
    ParseWarning {
        row: Some(row),
        message: message.into(),
    }
}

/// The normalized fields as JSON (persisted; the raw bytes are not — ADR 0014 §4).
fn normalized_json(fields: &[(&str, &str)]) -> String {
    let map: serde_json::Map<String, serde_json::Value> = fields
        .iter()
        .map(|(tag, value)| {
            (
                (*tag).to_owned(),
                serde_json::Value::String((*value).to_owned()),
            )
        })
        .collect();
    serde_json::Value::Object(map).to_string()
}

/// The OFX / QFX importer.
pub struct OfxImporter;

impl ImporterPlugin for OfxImporter {
    fn id(&self) -> &'static str {
        "ofx"
    }
    fn display_name(&self) -> &'static str {
        "OFX / Quicken (.ofx, .qfx)"
    }
    fn version(&self) -> Version {
        Version::new(1, 0, 0)
    }
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["ofx", "qfx"]
    }
    fn detect_confidence(&self, input: &ParserInput) -> u16 {
        // The OFX signature near the top of the payload is decisive: 1.x opens
        // with `OFXHEADER:100`, 2.x with an `<?OFX …?>` prologue and an `<OFX>`
        // root. The extension alone is a strong hint (mirrors GenericCsv).
        let head_len = input.bytes.len().min(1024);
        let head = String::from_utf8_lossy(&input.bytes[..head_len]).to_ascii_lowercase();
        if head.contains("ofxheader") || head.contains("<ofx") || head.contains("<?ofx") {
            return 9_500;
        }
        if matches!(input.extension().as_deref(), Some("ofx" | "qfx")) {
            return 9_000;
        }
        0
    }

    fn parse(&self, input: &ParserInput, hints: &ParserHints) -> Result<ParsedBatch, ParseError> {
        // OFX is ASCII-structured; decode lossily and scan a lowercased copy
        // (byte offsets line up — ASCII lowercasing is length-preserving),
        // slicing values out of the original to preserve their case.
        let text = String::from_utf8_lossy(&input.bytes).into_owned();
        let lower = text.to_ascii_lowercase();
        if !(lower.contains("ofxheader") || lower.contains("<ofx") || lower.contains("<?ofx")) {
            return Err(ParseError::Unsupported(
                "no OFXHEADER / <OFX> signature — not an OFX or QFX download".to_owned(),
            ));
        }

        let default_currency = hints.default_currency.unwrap_or(Currency::Usd);
        let curdefs = curdef_positions(&lower, &text)?;
        let mut records = Vec::new();
        let mut warnings = Vec::new();

        for (idx, &(start, end)) in stmttrn_blocks(&lower).iter().enumerate() {
            let block_lower = &lower[start..end];
            let block_orig = &text[start..end];
            let value = |tag: &str| tag_value(block_lower, block_orig, tag);

            // The statement's CURDEF governs every transaction in it (the
            // nearest one before this block); default when absent, mirror csv.
            let currency = curdefs
                .iter()
                .rev()
                .find(|(pos, _)| *pos < start)
                .map_or(default_currency, |(_, c)| *c);

            let Some(raw_date) = value("dtposted") else {
                warnings.push(warn(idx, "missing DTPOSTED"));
                continue;
            };
            let Some(posted_date) = parse_dtposted(&raw_date) else {
                warnings.push(warn(idx, format!("unparseable DTPOSTED {raw_date:?}")));
                continue;
            };
            // OFX dates are unambiguous YYYYMMDD — a successful parse is certain.
            let date_confidence_bps = 10_000;

            let Some(raw_amount) = value("trnamt") else {
                warnings.push(warn(idx, "missing TRNAMT"));
                continue;
            };
            let Some(amount_minor) = parse_minor_units(&raw_amount, currency.exponent()) else {
                warnings.push(warn(idx, format!("unparseable TRNAMT {raw_amount:?}")));
                continue;
            };

            let trntype = value("trntype");
            let fitid = value("fitid");
            let name = value("name");
            let payee = value("payee");
            let memo = value("memo");
            let dtuser = value("dtuser");
            // DTUSER is the user/authorization date — the secondary transaction date
            // when the source carries one distinct from DTPOSTED (ADR 0045).
            let transaction_date = dtuser
                .as_deref()
                .and_then(parse_dtposted)
                .filter(|&d| d != posted_date);

            // NAME (or PAYEE) is the merchant; MEMO is the description when
            // present, else the merchant doubles as the description.
            let merchant = name.clone().or_else(|| payee.clone());
            let description = memo.clone().or_else(|| merchant.clone());
            let normalized_merchant = merchant.as_ref().map(|m| m.to_ascii_lowercase());

            // FITID is the bank's stable id: it anchors the dedupe fingerprint
            // (surviving a reworded MEMO); without one, fall back to
            // csv-importer's date+amount+merchant content scheme.
            let txn_fingerprint = match &fitid {
                Some(id) => format!("{posted_date}|{amount_minor}|fitid:{id}"),
                None => format!(
                    "{posted_date}|{amount_minor}|{}",
                    normalized_merchant.as_deref().unwrap_or("")
                ),
            };

            let mut fields: Vec<(&str, &str)> = vec![
                ("DTPOSTED", raw_date.as_str()),
                ("TRNAMT", raw_amount.as_str()),
            ];
            for (tag, val) in [
                ("TRNTYPE", &trntype),
                ("FITID", &fitid),
                ("NAME", &name),
                ("PAYEE", &payee),
                ("MEMO", &memo),
                ("DTUSER", &dtuser),
            ] {
                if let Some(v) = val {
                    fields.push((tag, v.as_str()));
                }
            }
            let normalized = normalized_json(&fields);
            // Per-row source hash (block index keeps two identical blocks
            // distinct; a whole-file re-import is caught by the batch file
            // fingerprint).
            let source_hash =
                importer_core::content_fingerprint(format!("{idx}:{normalized}").as_bytes());

            records.push(ParsedRecord {
                external_id: fitid,
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
                    // OFX has no category field; a category prefill comes from
                    // categorization rules, not the source (ADR 0045 §3).
                    category: None,
                    normalized_merchant,
                    external_account: None,
                    txn_fingerprint,
                }),
                balance: None,
            });
        }

        Ok(ParsedBatch {
            source_format: "ofx".to_owned(),
            accounts: vec![],
            records,
            warnings,
        })
    }
}

register_importer!(OfxImporter);

#[cfg(test)]
mod tests {
    use super::*;

    /// OFX 1.x SGML: key:value header lines, unclosed tags throughout. Four
    /// transactions — a negative NAME+MEMO purchase, a positive NAME-only
    /// deposit, a PAYEE-only debit *without* a FITID, and a `+`-signed
    /// MEMO-only credit.
    const OFX_V1: &str = "\
OFXHEADER:100
DATA:OFXSGML
VERSION:102
SECURITY:NONE
ENCODING:USASCII
CHARSET:1252
COMPRESSION:NONE
OLDFILEUID:NONE
NEWFILEUID:NONE

<OFX>
<BANKMSGSRSV1>
<STMTTRNRS>
<TRNUID>1
<STMTRS>
<CURDEF>USD
<BANKACCTFROM>
<BANKID>123456789
<ACCTID>000123456
<ACCTTYPE>CHECKING
</BANKACCTFROM>
<BANKTRANLIST>
<DTSTART>20260601
<DTEND>20260630
<STMTTRN>
<TRNTYPE>DEBIT
<DTPOSTED>20260620120000[-5:EST]
<TRNAMT>-12.99
<FITID>2026062001
<NAME>COFFEE SHOP
<MEMO>CARD PURCHASE 06/20
<STMTTRN>
<TRNTYPE>CREDIT
<DTPOSTED>20260621
<TRNAMT>1500.00
<FITID>2026062102
<NAME>EMPLOYER PAYROLL
<STMTTRN>
<TRNTYPE>DEBIT
<DTPOSTED>20260622
<TRNAMT>-45.00
<PAYEE>GAS STATION
<STMTTRN>
<TRNTYPE>CREDIT
<DTPOSTED>20260623
<TRNAMT>+7.50
<FITID>2026062304
<MEMO>ATM FEE REBATE
</BANKTRANLIST>
<LEDGERBAL>
<BALAMT>5230.10
<DTASOF>20260630
</LEDGERBAL>
</STMTRS>
</STMTTRNRS>
</BANKMSGSRSV1>
</OFX>
";

    /// OFX 2.x XML: `<?xml` + `<?OFX?>` prologue, every tag closed. Three
    /// transactions, including an `&amp;` entity in a NAME.
    const OFX_V2: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<?OFX OFXHEADER="200" VERSION="203" SECURITY="NONE" OLDFILEUID="NONE" NEWFILEUID="NONE"?>
<OFX>
  <BANKMSGSRSV1>
    <STMTTRNRS>
      <TRNUID>1</TRNUID>
      <STMTRS>
        <CURDEF>USD</CURDEF>
        <BANKTRANLIST>
          <DTSTART>20260601</DTSTART>
          <DTEND>20260630</DTEND>
          <STMTTRN>
            <TRNTYPE>DEBIT</TRNTYPE>
            <DTPOSTED>20260605080000.000[-5:EST]</DTPOSTED>
            <TRNAMT>-82.45</TRNAMT>
            <FITID>X-1001</FITID>
            <NAME>BOOKS &amp; RECORDS</NAME>
            <MEMO>ONLINE ORDER</MEMO>
          </STMTTRN>
          <STMTTRN>
            <TRNTYPE>CREDIT</TRNTYPE>
            <DTPOSTED>20260610</DTPOSTED>
            <TRNAMT>250.00</TRNAMT>
            <FITID>X-1002</FITID>
            <NAME>TRANSFER IN</NAME>
          </STMTTRN>
          <STMTTRN>
            <TRNTYPE>DEBIT</TRNTYPE>
            <DTPOSTED>20260615</DTPOSTED>
            <TRNAMT>-9.99</TRNAMT>
            <FITID>X-1003</FITID>
            <PAYEE>STREAMING SVC</PAYEE>
          </STMTTRN>
        </BANKTRANLIST>
      </STMTRS>
    </STMTTRNRS>
  </BANKMSGSRSV1>
</OFX>
"#;

    fn input(ofx: &str) -> ParserInput {
        ParserInput::new(ofx.as_bytes().to_vec()).with_filename("statement.ofx")
    }

    #[test]
    fn dtuser_populates_the_secondary_transaction_date() {
        // DTPOSTED is the primary posted date; a distinct DTUSER is the authorization /
        // transaction date (ADR 0045, personal-cfo-4d8.24.1).
        let ofx = "OFXHEADER:100\nDATA:OFXSGML\n\n<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS>\
                   <CURDEF>USD<BANKTRANLIST>\
                   <STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20260620<DTUSER>20260618\
                   <TRNAMT>-12.99<FITID>x1<NAME>COFFEE</BANKTRANLIST></STMTRS></STMTTRNRS>\
                   </BANKMSGSRSV1></OFX>";
        let batch = OfxImporter
            .parse(&input(ofx), &ParserHints::default())
            .unwrap();
        let txn = batch.records[0].transaction.as_ref().unwrap();
        assert_eq!(
            txn.posted_date,
            chrono::NaiveDate::from_ymd_opt(2026, 6, 20).unwrap()
        );
        assert_eq!(
            txn.transaction_date,
            Some(chrono::NaiveDate::from_ymd_opt(2026, 6, 18).unwrap())
        );
    }

    #[test]
    fn parse_minor_units_handles_ofx_amounts() {
        assert_eq!(parse_minor_units("-12.99", 2), Some(-1299));
        assert_eq!(parse_minor_units("+7.5", 2), Some(750));
        assert_eq!(parse_minor_units("1500.00", 2), Some(150_000));
        assert_eq!(parse_minor_units("1500", 2), Some(150_000));
        assert_eq!(parse_minor_units(".50", 2), Some(50));
        assert_eq!(parse_minor_units("", 2), None);
        // OFX has no thousands separators / symbols — reject, don't guess.
        assert_eq!(parse_minor_units("1,500.00", 2), None);
        assert_eq!(parse_minor_units("$12.99", 2), None);
        assert_eq!(parse_minor_units("1.2.3", 2), None);
    }

    #[test]
    fn parse_dtposted_ignores_time_and_zone_suffix() {
        let d = chrono::NaiveDate::from_ymd_opt(2026, 6, 20).unwrap();
        assert_eq!(parse_dtposted("20260620"), Some(d));
        assert_eq!(parse_dtposted("20260620120000[-5:EST]"), Some(d));
        assert_eq!(parse_dtposted("2026062"), None);
        assert_eq!(parse_dtposted("not-a-date"), None);
    }

    #[test]
    fn parses_the_sgml_v1_sample() {
        let batch = OfxImporter
            .parse(&input(OFX_V1), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.source_format, "ofx");
        assert_eq!(batch.records.len(), 4);
        assert!(batch.warnings.is_empty());

        // NAME+MEMO pair: NAME → merchant, MEMO → description.
        let first = &batch.records[0];
        let txn = first.transaction.as_ref().unwrap();
        assert_eq!(first.external_id.as_deref(), Some("2026062001"));
        assert_eq!(txn.amount, Money::new(-1299, Currency::Usd));
        assert_eq!(
            txn.posted_date,
            chrono::NaiveDate::from_ymd_opt(2026, 6, 20).unwrap()
        );
        assert_eq!(txn.normalized_merchant.as_deref(), Some("coffee shop"));
        assert_eq!(txn.description.as_deref(), Some("CARD PURCHASE 06/20"));
        assert_eq!(txn.date_confidence_bps, 10_000);
        assert_eq!(txn.txn_fingerprint, "2026-06-20|-1299|fitid:2026062001");

        // NAME-only: the merchant doubles as the description.
        let second = batch.records[1].transaction.as_ref().unwrap();
        assert_eq!(second.amount, Money::new(150_000, Currency::Usd));
        assert_eq!(second.description.as_deref(), Some("EMPLOYER PAYROLL"));
        assert_eq!(
            second.normalized_merchant.as_deref(),
            Some("employer payroll")
        );

        // PAYEE-only + no FITID: fallback content fingerprint, no external_id.
        let third_record = &batch.records[2];
        let third = third_record.transaction.as_ref().unwrap();
        assert_eq!(third_record.external_id, None);
        assert_eq!(third.amount, Money::new(-4500, Currency::Usd));
        assert_eq!(third.normalized_merchant.as_deref(), Some("gas station"));
        assert_eq!(third.txn_fingerprint, "2026-06-22|-4500|gas station");

        // '+'-signed MEMO-only credit.
        let fourth = batch.records[3].transaction.as_ref().unwrap();
        assert_eq!(fourth.amount, Money::new(750, Currency::Usd));
        assert_eq!(fourth.normalized_merchant, None);
        assert_eq!(fourth.description.as_deref(), Some("ATM FEE REBATE"));
    }

    #[test]
    fn parses_the_xml_v2_sample() {
        let batch = OfxImporter
            .parse(&input(OFX_V2), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 3);
        assert!(batch.warnings.is_empty());

        let first = &batch.records[0];
        let txn = first.transaction.as_ref().unwrap();
        assert_eq!(first.external_id.as_deref(), Some("X-1001"));
        assert_eq!(txn.amount, Money::new(-8245, Currency::Usd));
        assert_eq!(
            txn.posted_date,
            chrono::NaiveDate::from_ymd_opt(2026, 6, 5).unwrap()
        );
        // The XML entity is unescaped in the merchant.
        assert_eq!(txn.normalized_merchant.as_deref(), Some("books & records"));
        assert_eq!(txn.description.as_deref(), Some("ONLINE ORDER"));

        let second = batch.records[1].transaction.as_ref().unwrap();
        assert_eq!(second.amount, Money::new(25_000, Currency::Usd));
        assert_eq!(second.description.as_deref(), Some("TRANSFER IN"));

        let third = batch.records[2].transaction.as_ref().unwrap();
        assert_eq!(third.amount, Money::new(-999, Currency::Usd));
        assert_eq!(third.normalized_merchant.as_deref(), Some("streaming svc"));
    }

    #[test]
    fn source_hashes_are_unique_per_record() {
        for sample in [OFX_V1, OFX_V2] {
            let batch = OfxImporter
                .parse(&input(sample), &ParserHints::default())
                .unwrap();
            let mut hashes: Vec<&str> = batch
                .records
                .iter()
                .map(|r| r.source_hash.as_str())
                .collect();
            hashes.sort_unstable();
            hashes.dedup();
            assert_eq!(hashes.len(), batch.records.len());
        }
    }

    #[test]
    fn fingerprints_are_stable_and_fitid_sensitive() {
        let hints = ParserHints::default();
        let a = OfxImporter.parse(&input(OFX_V1), &hints).unwrap();
        let b = OfxImporter.parse(&input(OFX_V1), &hints).unwrap();
        // Same input twice → identical fingerprints (and hashes).
        for (ra, rb) in a.records.iter().zip(&b.records) {
            assert_eq!(ra.source_hash, rb.source_hash);
            assert_eq!(
                ra.transaction.as_ref().unwrap().txn_fingerprint,
                rb.transaction.as_ref().unwrap().txn_fingerprint
            );
        }
        // A changed FITID (same date/amount/merchant) → a different fingerprint.
        let mutated = OFX_V1.replace("<FITID>2026062001", "<FITID>9999999999");
        let c = OfxImporter.parse(&input(&mutated), &hints).unwrap();
        assert_ne!(
            a.records[0].transaction.as_ref().unwrap().txn_fingerprint,
            c.records[0].transaction.as_ref().unwrap().txn_fingerprint
        );
    }

    #[test]
    fn an_unparseable_block_becomes_a_warning_not_a_transaction() {
        let mutated = OFX_V1.replace("<DTPOSTED>20260621", "<DTPOSTED>bogus");
        let batch = OfxImporter
            .parse(&input(&mutated), &ParserHints::default())
            .unwrap();
        assert_eq!(batch.records.len(), 3, "only the valid blocks are staged");
        assert_eq!(batch.warnings.len(), 1, "the bad block is flagged");
        assert_eq!(batch.warnings[0].row, Some(1));
    }

    #[test]
    fn an_unsupported_curdef_is_rejected_not_misbooked() {
        let mutated = OFX_V1.replace("<CURDEF>USD", "<CURDEF>GBP");
        let err = OfxImporter
            .parse(&input(&mutated), &ParserHints::default())
            .unwrap_err();
        assert!(matches!(err, ParseError::Unsupported(_)));
    }

    #[test]
    fn a_non_ofx_payload_is_unsupported() {
        let err = OfxImporter
            .parse(
                &ParserInput::new(b"Date,Amount\n2026-01-01,1.00\n".to_vec()),
                &ParserHints::default(),
            )
            .unwrap_err();
        assert!(matches!(err, ParseError::Unsupported(_)));
    }

    #[test]
    fn detect_scores_ofx_high_and_csv_low() {
        // Both eras score high on content alone (no filename needed).
        assert_eq!(
            OfxImporter.detect_confidence(&ParserInput::new(OFX_V1.as_bytes().to_vec())),
            9_500
        );
        assert_eq!(
            OfxImporter.detect_confidence(&ParserInput::new(OFX_V2.as_bytes().to_vec())),
            9_500
        );
        // The extension is a strong hint even without a signature.
        assert_eq!(
            OfxImporter.detect_confidence(&ParserInput::new(vec![]).with_filename("download.qfx")),
            9_000
        );
        // A CSV payload is not claimed.
        let csv = ParserInput::new(b"Date,Description,Amount\n2026-06-20,Coffee,-12.99\n".to_vec())
            .with_filename("statement.csv");
        assert_eq!(OfxImporter.detect_confidence(&csv), 0);
    }

    #[test]
    fn registered_in_the_compile_time_registry() {
        let plugin = importer_core::plugin_by_id("ofx").expect("registered");
        assert_eq!(plugin.version(), Version::new(1, 0, 0));
        let best = importer_core::detect_best(&input(OFX_V1)).expect("detected by signature");
        assert_eq!(best.id(), "ofx");
    }
}
