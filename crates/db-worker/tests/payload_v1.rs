//! Command payloads, schema version 1 (personal-cfo-t2s4b part 3b).
//!
//! A sample of every `WriteCommand` variant is encoded as canonical JSON, compared with a
//! frozen fixture, and decoded back. Three guards work together:
//!
//! * the **round trip** (`decode(encode(x))` re-encodes to the same bytes and carries the
//!   same value), with integers at their awkward extremes, is the authority: any type that
//!   buffers its content or is not one-to-one fails it;
//! * the **frozen fixtures** and a **pinned digest** of them make a change of shape visible:
//!   a field added, renamed or retyped fails until `PAYLOAD_SCHEMA_VERSION` is bumped on
//!   purpose and the old fixtures are kept;
//! * a **coverage guard** reads the enum's source, so a new variant with no sample fails.
//!
//! A source scan for the serde attributes that break integer-as-string decoding gives an
//! earlier, clearer error than the round trip would; it is an early warning, not the guard.
//!
//! To regenerate the fixtures after an intentional, version-bumping change:
//! `PAYLOAD_FIXTURES_UPDATE=1 cargo test -p db-worker --test payload_v1`.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, TimeZone, Utc};
use core_ledger::{
    Account, AccountFlags, AccountId, AccountSubtype, BillContractId, CashflowRole, CategoryId,
    IncomeSourceId, LedgerAccountId, RecurringEventId, RecurringTransferId, SourceBatchId,
    SourceRecordId, StagedTransactionId, TagId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::payload::{
    decode_payload, encode_payload, prepare_payload, PayloadError, PAYLOAD_SCHEMA_VERSION,
};
use db_worker::{DebtTermsInput, RepaymentPhilosophy, SplitLineInput, WriteCommand};
use pay_schedule::Frequency;
use sha2::{Digest, Sha256};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Deterministic sample values
// ---------------------------------------------------------------------------

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0000 + n)
}
fn account(n: u128) -> AccountId {
    AccountId::from_uuid(u(n))
}
fn category(n: u128) -> CategoryId {
    CategoryId::from_uuid(u(n))
}
fn tag(n: u128) -> TagId {
    TagId::from_uuid(u(n))
}
fn txn(n: u128) -> TransactionId {
    TransactionId::from_uuid(u(n))
}
fn event(n: u128) -> RecurringEventId {
    RecurringEventId::from_uuid(u(n))
}
fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}
fn instant() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 17, 12, 30, 45).unwrap()
}
fn usd(minor: i64) -> Money {
    Money::new(minor, Currency::Usd)
}

/// 2^53 + 1: the first integer a double cannot hold. A JSON number would corrupt it.
const BEYOND_DOUBLE: i64 = 9_007_199_254_740_993;

/// One sample per variant, named by the variant's wire key (its snake-case name).
fn samples() -> Vec<(&'static str, WriteCommand)> {
    let flags = AccountFlags {
        retirement: false,
        tax_advantaged: true,
        joint: false,
        business: true,
        active: true,
    };
    let checking = Account::new(
        account(1),
        LedgerAccountId::from_uuid(u(2)),
        "Checking \u{e9}",
        CashflowRole::LiquidCash,
        Currency::Usd,
        flags,
    );
    let terms = DebtTermsInput {
        apr_bps: Some(2_499),
        statement_close_day: Some(15),
        payment_due_day: Some(10),
        grace_period_days: Some(25),
        credit_limit_minor: Some(BEYOND_DOUBLE),
        repayment_philosophy: RepaymentPhilosophy::PayStatementBalance,
        fixed_amount_minor: None,
        min_payment_percent_bps: Some(200),
        min_payment_floor_minor: Some(2_500),
        paying_source_account_id: Some(account(3)),
        original_principal_minor: Some(i64::MAX),
    };
    vec![
        (
            "create_account",
            WriteCommand::CreateAccount {
                account: Box::new(checking.clone()),
                opening_balance: Some(usd(-150_000)),
            },
        ),
        (
            "update_account",
            WriteCommand::UpdateAccount {
                id: account(1),
                name: "Joint \"main\"".into(),
            },
        ),
        ("archive_account", WriteCommand::ArchiveAccount(account(1))),
        (
            "reinstate_account",
            WriteCommand::ReinstateAccount(account(1)),
        ),
        (
            "set_account_subtype",
            WriteCommand::SetAccountSubtype {
                id: account(1),
                subtype: Some(AccountSubtype::Savings),
            },
        ),
        (
            "set_account_note",
            WriteCommand::SetAccountNote {
                id: account(1),
                note: Some("line one\nline two \u{1f4b0}".into()),
            },
        ),
        (
            "set_account_link",
            WriteCommand::SetAccountLink {
                asset_id: account(4),
                liability_id: Some(account(5)),
            },
        ),
        (
            "set_debt_terms",
            WriteCommand::SetDebtTerms {
                account_id: account(5),
                terms,
            },
        ),
        (
            "set_card_statement_balance",
            WriteCommand::SetCardStatementBalance {
                account_id: account(5),
                cycle_close: date(2026, 9, 15),
                statement_balance_minor: Some(i64::MIN),
            },
        ),
        (
            "record_transaction",
            WriteCommand::RecordTransaction {
                transaction_id: txn(10),
                account_id: account(1),
                amount: usd(-1),
                occurred_at: instant(),
            },
        ),
        (
            "confirm_obligation_early",
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id: event(11),
                scheduled_date: date(2026, 11, 1),
                actual_amount: Money::new(-BEYOND_DOUBLE, Currency::Eur),
                actual_date: instant(),
                paying_account_id: account(1),
            },
        ),
        (
            "unconfirm_obligation",
            WriteCommand::UnconfirmObligation {
                recurring_event_id: event(11),
                scheduled_date: date(2026, 11, 1),
            },
        ),
        (
            "convert_unexplained_to_transaction",
            WriteCommand::ConvertUnexplainedToTransaction {
                account_id: account(1),
            },
        ),
        (
            "transfer",
            WriteCommand::Transfer {
                source_account_id: account(1),
                dest_account_id: account(4),
                amount: usd(0),
                occurred_at: instant(),
            },
        ),
        (
            "create_recurring_transfer",
            WriteCommand::CreateRecurringTransfer {
                id: RecurringTransferId::from_uuid(u(12)),
                source_account_id: account(1),
                dest_account_id: account(4),
                amount: usd(50_000),
                frequency: Frequency::EveryNWeeks(6),
                anchor: date(2026, 1, 31),
            },
        ),
        (
            "delete_recurring_transfer",
            WriteCommand::DeleteRecurringTransfer(RecurringTransferId::from_uuid(u(12))),
        ),
        (
            "reinstate_recurring_transfer",
            WriteCommand::ReinstateRecurringTransfer(RecurringTransferId::from_uuid(u(12))),
        ),
        (
            "create_income_source",
            WriteCommand::CreateIncomeSource {
                id: IncomeSourceId::from_uuid(u(13)),
                name: "Paycheck".into(),
                net_amount: usd(312_345),
                frequency: Frequency::SemiMonthly,
                anchor: date(2026, 2, 15),
                deposit_account_id: Some(account(1)),
            },
        ),
        (
            "update_income_source",
            WriteCommand::UpdateIncomeSource {
                id: IncomeSourceId::from_uuid(u(13)),
                name: "Paycheck (raise)".into(),
                net_amount: usd(330_000),
                frequency: Frequency::Biweekly,
                anchor: date(2026, 2, 13),
                deposit_account_id: None,
            },
        ),
        (
            "delete_income_source",
            WriteCommand::DeleteIncomeSource(IncomeSourceId::from_uuid(u(13))),
        ),
        (
            "reinstate_income_source",
            WriteCommand::ReinstateIncomeSource(IncomeSourceId::from_uuid(u(13))),
        ),
        (
            "archive_income_source",
            WriteCommand::ArchiveIncomeSource(IncomeSourceId::from_uuid(u(13))),
        ),
        (
            "restore_income_source",
            WriteCommand::RestoreIncomeSource(IncomeSourceId::from_uuid(u(13))),
        ),
        (
            "create_recurring_bill",
            WriteCommand::CreateRecurringBill {
                event_id: event(14),
                contract_id: BillContractId::from_uuid(u(15)),
                name: "Rent".into(),
                amount: usd(-215_000),
                bill_type: "rent".into(),
                frequency: Frequency::Monthly,
                anchor: date(2026, 3, 1),
                autopay_account_id: Some(account(1)),
                description: Some("Lease #12".into()),
                source_merchant_key: Some("landlord llc".into()),
                category_id: Some(category(16)),
                tag_ids: vec![tag(17), tag(18)],
            },
        ),
        (
            "set_bill_autopay",
            WriteCommand::SetBillAutopay {
                event_id: event(14),
                autopay: true,
            },
        ),
        (
            "update_recurring_bill",
            WriteCommand::UpdateRecurringBill {
                event_id: event(14),
                name: "Rent".into(),
                amount: usd(-220_000),
                bill_type: "rent".into(),
                frequency: Frequency::EveryNMonths(3),
                anchor: date(2026, 3, 1),
                autopay_account_id: None,
                description: None,
            },
        ),
        (
            "delete_recurring_bill",
            WriteCommand::DeleteRecurringBill {
                event_id: event(14),
            },
        ),
        (
            "reinstate_recurring_bill",
            WriteCommand::ReinstateRecurringBill {
                event_id: event(14),
            },
        ),
        (
            "archive_recurring_bill",
            WriteCommand::ArchiveRecurringBill(event(14)),
        ),
        (
            "restore_recurring_bill",
            WriteCommand::RestoreRecurringBill(event(14)),
        ),
        (
            "link_bill_to_loan",
            WriteCommand::LinkBillToLoan {
                event_id: event(14),
                loan_account_id: account(1),
            },
        ),
        (
            "unlink_bill_from_loan",
            WriteCommand::UnlinkBillFromLoan {
                event_id: event(14),
            },
        ),
        (
            "set_bill_not_loan_payment",
            WriteCommand::SetBillNotLoanPayment {
                event_id: event(14),
                loan_account_id: Some(account(1)),
            },
        ),
        (
            "create_source_batch",
            WriteCommand::CreateSourceBatch {
                id: SourceBatchId::from_uuid(u(19)),
                source_type: "csv".into(),
                source_name: Some("export.csv".into()),
                file_fingerprint: Some("sha256:ab".into()),
                parser_version: Some("csv-v1".into()),
                connector_connection_id: Some(u(20)),
            },
        ),
        (
            "attach_source_record",
            WriteCommand::AttachSourceRecord {
                id: SourceRecordId::from_uuid(u(21)),
                batch_id: SourceBatchId::from_uuid(u(19)),
                external_id: Some("ext-1".into()),
                source_hash: "sha256:cd".into(),
                normalized_json: "{\"amount\":-1299,\"memo\":\"caf\u{e9}\"}".into(),
                parse_confidence_bps: Some(10_000),
            },
        ),
        (
            "update_batch_state",
            WriteCommand::UpdateBatchState {
                batch_id: SourceBatchId::from_uuid(u(19)),
                status: "committed".into(),
                staged_count: 12,
                committed_count: 10,
                skipped_count: 2,
            },
        ),
        (
            "commit_staged",
            WriteCommand::CommitStaged {
                staged_transaction_id: StagedTransactionId::from_uuid(u(22)),
                force: false,
            },
        ),
        (
            "skip_staged",
            WriteCommand::SkipStaged {
                staged_transaction_id: StagedTransactionId::from_uuid(u(22)),
            },
        ),
        (
            "snooze_inbox_item",
            WriteCommand::SnoozeInboxItem {
                item_id: u(23),
                until: date(2026, 12, 25),
            },
        ),
        (
            "dismiss_inbox_item",
            WriteCommand::DismissInboxItem {
                item_id: u(23),
                reason: "not mine".into(),
            },
        ),
        (
            "dismiss_recurring_suggestion",
            WriteCommand::DismissRecurringSuggestion {
                merchant_key: "coffee shop".into(),
                currency: "USD".into(),
                amount_minor: -450,
                frequency: "weekly".into(),
                reason: None,
            },
        ),
        (
            "create_category",
            WriteCommand::CreateCategory {
                id: category(16),
                parent_id: Some(category(24)),
                name: "Groceries".into(),
                category_type: "expense".into(),
                color: Some("#00aa55".into()),
                icon: None,
            },
        ),
        (
            "update_category",
            WriteCommand::UpdateCategory {
                id: category(16),
                name: "Food".into(),
                color: None,
                icon: Some("apple".into()),
            },
        ),
        (
            "move_category",
            WriteCommand::MoveCategory {
                id: category(16),
                new_parent_id: None,
            },
        ),
        (
            "archive_category",
            WriteCommand::ArchiveCategory(category(16)),
        ),
        (
            "reinstate_category",
            WriteCommand::ReinstateCategory(category(16)),
        ),
        (
            "recategorize_transaction",
            WriteCommand::RecategorizeTransaction {
                transaction_id: txn(10),
                category_id: Some(category(16)),
            },
        ),
        (
            "void_transaction",
            WriteCommand::VoidTransaction {
                transaction_id: txn(10),
            },
        ),
        (
            "mark_reviewed",
            WriteCommand::MarkReviewed {
                transaction_id: txn(10),
                reviewed: true,
            },
        ),
        (
            "apply_scenario",
            WriteCommand::ApplyScenario { scenario_id: u(25) },
        ),
        (
            "revert_scenario_apply",
            WriteCommand::RevertScenarioApply { scenario_id: u(25) },
        ),
        (
            "create_tag",
            WriteCommand::CreateTag {
                id: tag(17),
                name: "trip".into(),
                color: Some("#123456".into()),
            },
        ),
        (
            "set_tags",
            WriteCommand::SetTags {
                transaction_id: txn(10),
                tag_ids: vec![tag(17), tag(18)],
            },
        ),
        (
            "set_note",
            WriteCommand::SetNote {
                transaction_id: txn(10),
                note: None,
            },
        ),
        (
            "set_splits",
            WriteCommand::SetSplits {
                transaction_id: txn(10),
                lines: vec![
                    SplitLineInput {
                        amount: usd(-700),
                        category_id: Some(category(16)),
                        note: Some("part".into()),
                        tag_ids: vec![tag(17)],
                    },
                    SplitLineInput {
                        amount: usd(-300),
                        category_id: None,
                        note: None,
                        tag_ids: vec![],
                    },
                ],
            },
        ),
        (
            "set_household_timezone",
            WriteCommand::SetHouseholdTimezone {
                timezone: "America/Chicago".into(),
            },
        ),
    ]
}

/// Debug output carries every field of every nested type, so equal strings mean equal
/// values even though `WriteCommand` does not implement `PartialEq`.
fn same_value(a: &WriteCommand, b: &WriteCommand) -> bool {
    format!("{a:?}") == format!("{b:?}")
}

// ---------------------------------------------------------------------------
// Coverage
// ---------------------------------------------------------------------------

fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The variants of `WriteCommand`, read from the enum's own source.
fn source_variants() -> Vec<String> {
    let source = include_str!("../src/lib.rs");
    let body = source
        .split_once("pub enum WriteCommand")
        .unwrap()
        .1
        .split_once("impl WriteCommand")
        .unwrap()
        .0;
    body.lines()
        .filter_map(|line| {
            let t = line.trim_start();
            let first = t
                .split(|c: char| c.is_whitespace() || matches!(c, '{' | '(' | ','))
                .next()?;
            (!first.is_empty()
                && first.chars().next()?.is_ascii_uppercase()
                && first.chars().all(|c| c.is_ascii_alphanumeric()))
            .then(|| snake_case(first))
        })
        .collect()
}

#[test]
fn every_write_command_variant_has_exactly_one_sample() {
    let mut expected = source_variants();
    expected.sort();
    let mut actual: Vec<String> = samples().iter().map(|(n, _)| (*n).to_owned()).collect();
    actual.sort();
    assert_eq!(expected.len(), 56, "the enum has 56 variants");
    assert_eq!(
        actual, expected,
        "a variant without a sample, or a sample without a variant"
    );
}

#[test]
fn each_sample_encodes_under_its_own_variant_key() {
    for (name, command) in samples() {
        let bytes = encode_payload(&command).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            text.starts_with(&format!("{{\"{name}\":")) || text == format!("\"{name}\""),
            "{name}: {text}"
        );
    }
}

// ---------------------------------------------------------------------------
// Round trip (the authority) and the one-to-one property
// ---------------------------------------------------------------------------

#[test]
fn every_variant_round_trips_through_the_versioned_payload() {
    for (name, command) in samples() {
        let bytes = encode_payload(&command).unwrap_or_else(|e| panic!("{name}: {e}"));
        let decoded = decode_payload(PAYLOAD_SCHEMA_VERSION, &bytes)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            same_value(&command, &decoded),
            "{name}: {command:?} != {decoded:?}"
        );
        assert_eq!(
            encode_payload(&decoded).unwrap(),
            bytes,
            "{name}: not one-to-one"
        );
    }
}

#[test]
fn no_float_and_no_json_number_appears_in_any_payload() {
    for (name, command) in samples() {
        let text = String::from_utf8(encode_payload(&command).unwrap()).unwrap();
        let bytes = text.as_bytes();
        // Outside a string, a digit or '-' can only start a JSON number.
        let mut in_string = false;
        let mut escaped = false;
        for &b in bytes {
            if in_string {
                match (escaped, b) {
                    (true, _) => escaped = false,
                    (false, b'\\') => escaped = true,
                    (false, b'"') => in_string = false,
                    _ => {}
                }
            } else if b == b'"' {
                in_string = true;
            } else {
                assert!(
                    !(b.is_ascii_digit() || b == b'-' || b == b'.'),
                    "{name}: a number outside a string in {text}"
                );
            }
        }
    }
}

#[test]
fn integers_at_their_extremes_survive_as_strings() {
    let cases: Vec<WriteCommand> = [
        0_i64,
        -1,
        1,
        BEYOND_DOUBLE,
        -BEYOND_DOUBLE,
        i64::MIN,
        i64::MAX,
    ]
    .into_iter()
    .map(|n| WriteCommand::SetCardStatementBalance {
        account_id: account(5),
        cycle_close: date(2026, 9, 15),
        statement_balance_minor: Some(n),
    })
    .chain(
        [0_i64, -1, BEYOND_DOUBLE, i64::MIN, i64::MAX]
            .into_iter()
            .map(|n| WriteCommand::RecordTransaction {
                transaction_id: txn(10),
                account_id: account(1),
                amount: usd(n),
                occurred_at: instant(),
            }),
    )
    .chain(
        [1_u32, 45, 366]
            .into_iter()
            .map(|n| WriteCommand::CreateRecurringTransfer {
                id: RecurringTransferId::from_uuid(u(12)),
                source_account_id: account(1),
                dest_account_id: account(4),
                amount: usd(1),
                frequency: Frequency::EveryNDays(n),
                anchor: date(2026, 1, 1),
            }),
    )
    .collect();
    for command in cases {
        let bytes = encode_payload(&command).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        let decoded = decode_payload(PAYLOAD_SCHEMA_VERSION, &bytes).unwrap();
        assert!(same_value(&command, &decoded), "{text}");
    }
    // The value a double would round is carried exactly.
    let exact = encode_payload(&WriteCommand::SetCardStatementBalance {
        account_id: account(5),
        cycle_close: date(2026, 9, 15),
        statement_balance_minor: Some(BEYOND_DOUBLE),
    })
    .unwrap();
    assert!(String::from_utf8(exact)
        .unwrap()
        .contains("\"9007199254740993\""));
}

#[test]
fn the_origin_applies_exactly_what_every_replica_will_decode() {
    for (name, command) in samples() {
        let prepared = prepare_payload(&command).unwrap();
        assert_eq!(prepared.bytes, encode_payload(&command).unwrap(), "{name}");
        assert!(same_value(&prepared.command, &command), "{name}");
        // Applying the prepared command re-prepares to the identical bytes.
        assert_eq!(
            prepare_payload(&prepared.command).unwrap().bytes,
            prepared.bytes,
            "{name}"
        );
    }
}

// ---------------------------------------------------------------------------
// Versioning and strict decode
// ---------------------------------------------------------------------------

#[test]
fn a_payload_of_another_version_is_refused_loudly() {
    let (_, command) = samples().remove(0);
    let bytes = encode_payload(&command).unwrap();
    for version in [0_u32, 2, 3, u32::MAX] {
        assert_eq!(
            decode_payload(version, &bytes).unwrap_err(),
            PayloadError::UnsupportedVersion {
                found: version,
                supported: PAYLOAD_SCHEMA_VERSION
            }
        );
    }
    assert_eq!(PAYLOAD_SCHEMA_VERSION, 1);
}

#[test]
fn a_changed_or_unknown_payload_is_refused() {
    let (_, command) = samples()
        .into_iter()
        .find(|(n, _)| *n == "set_account_note")
        .unwrap();
    let good = String::from_utf8(encode_payload(&command).unwrap()).unwrap();
    for bad in [
        good.replace("set_account_note", "set_account_notes"), // unknown variant
        good.replace("\"note\":", "\"extra\":\"1\",\"note\":"), // unknown member
        good.replace("{\"id\"", "{ \"id\""),                   // non-canonical spelling
        format!("{good}\n"),
        good.replace("\"id\":\"", "\"id\":\"X"), // a bad id
        String::new(),
        "null".to_owned(),
        "[]".to_owned(),
        "{\"archive_account\":1}".to_owned(), // a JSON number
        "{\"archive_account\":\"not-a-uuid\"}".to_owned(),
        "{\"archive_account\":null}".to_owned(),
    ] {
        assert!(
            decode_payload(PAYLOAD_SCHEMA_VERSION, bad.as_bytes()).is_err(),
            "{bad}"
        );
    }
    // Two variants in one object is not a command.
    let two = format!(
        "{{\"archive_account\":\"{}\",\"reinstate_account\":\"{}\"}}",
        account(1).as_uuid(),
        account(1).as_uuid()
    );
    assert!(decode_payload(PAYLOAD_SCHEMA_VERSION, two.as_bytes()).is_err());
}

#[test]
fn a_money_value_with_the_wrong_exponent_is_refused() {
    let good = encode_payload(&WriteCommand::RecordTransaction {
        transaction_id: txn(10),
        account_id: account(1),
        amount: usd(-5),
        occurred_at: instant(),
    })
    .unwrap();
    let text = String::from_utf8(good).unwrap();
    assert!(text.contains("\"currency_exponent\":\"2\""), "{text}");
    for wrong in ["0", "3", "255"] {
        let tampered = text.replace(
            "\"currency_exponent\":\"2\"",
            &format!("\"currency_exponent\":\"{wrong}\""),
        );
        assert!(
            decode_payload(PAYLOAD_SCHEMA_VERSION, tampered.as_bytes()).is_err(),
            "exponent {wrong}"
        );
    }
}

// ---------------------------------------------------------------------------
// Frozen fixtures
// ---------------------------------------------------------------------------

fn fixtures_dir(version: u32) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/payload_v{version}"))
}

/// SHA-256 over `name NUL bytes NUL` for each fixture in name order.
fn fixture_digest(files: &[(String, Vec<u8>)]) -> String {
    let mut hasher = Sha256::new();
    for (name, bytes) in files {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        hasher.update([0]);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The digest of the version 1 fixtures. Changing a payload's shape changes the bytes, so
/// this must change, and `PAYLOAD_SCHEMA_VERSION` must be bumped with it, keeping these
/// files as the version 1 record. Update it only through that deliberate step.
///
/// One exception is recorded here: adding a **variant** is additive, not a change of any
/// existing payload's shape, so while no released app writes version 2 payloads it re-pins
/// this digest without a version bump (planning's call, 2026-10-19). The three `Reinstate*`
/// variants of personal-cfo-egn67 did that: the 50 existing fixtures are byte-identical and
/// three files were added. So did `LinkBillToLoan`, `UnlinkBillFromLoan` and
/// `SetBillNotLoanPayment` of personal-cfo-6wk.22: the 53 existing fixtures are byte-identical
/// and three files were added (planning's rule: new additive v1 payloads, no field added to any
/// existing payload).
const PAYLOAD_V1_DIGEST: &str = "3a8f00be27e2936616a18fc4db854cc2bf3c542b4c85d506308f832120e7525f";

#[test]
fn the_frozen_version_1_fixtures_match_the_samples_and_the_pinned_digest() {
    let dir = fixtures_dir(1);
    let mut current: Vec<(String, Vec<u8>)> = samples()
        .into_iter()
        .map(|(name, command)| (name.to_owned(), encode_payload(&command).unwrap()))
        .collect();
    current.sort();

    if std::env::var_os("PAYLOAD_FIXTURES_UPDATE").is_some() {
        fs::create_dir_all(&dir).unwrap();
        for (name, bytes) in &current {
            fs::write(dir.join(format!("{name}.json")), bytes).unwrap();
        }
        println!("PAYLOAD_V1_DIGEST = {}", fixture_digest(&current));
        return;
    }

    let mut frozen: Vec<(String, Vec<u8>)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|entry| {
            let path = entry.path();
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            (name, fs::read(&path).unwrap())
        })
        .collect();
    frozen.sort();

    assert_eq!(
        frozen.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
        current.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
        "one fixture per variant"
    );
    for ((name, bytes), (_, expected)) in current.iter().zip(&frozen) {
        assert_eq!(
            bytes, expected,
            "{name}: the payload shape changed. Bump PAYLOAD_SCHEMA_VERSION, keep these \
             fixtures as the previous version's record, and add the new version's fixtures."
        );
    }
    assert_eq!(
        fixture_digest(&frozen),
        PAYLOAD_V1_DIGEST,
        "the version 1 fixtures were edited"
    );
}

#[test]
fn every_frozen_fixture_decodes_under_its_version() {
    for version in 1..=PAYLOAD_SCHEMA_VERSION {
        let dir = fixtures_dir(version);
        let mut count = 0;
        for entry in fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .flatten()
        {
            let bytes = fs::read(entry.path()).unwrap();
            let command = decode_payload(version, &bytes)
                .unwrap_or_else(|e| panic!("{}: {e}", entry.path().display()));
            assert_eq!(
                encode_payload(&command).unwrap(),
                bytes,
                "{}",
                entry.path().display()
            );
            count += 1;
        }
        assert_eq!(
            count, 56,
            "version {version} has a fixture for every variant"
        );
    }
}

// ---------------------------------------------------------------------------
// An early warning for the serde attributes that break integer-as-string decoding
// ---------------------------------------------------------------------------

/// serde's internally tagged, adjacently tagged, untagged and flattened representations
/// buffer their content, and a buffer hands integers back as strings that the integer
/// deserializers then refuse. The round trip above is the real guard; this scan only
/// names the cause sooner. It covers the crates that define the payload types and ignores
/// comments, so it is blunt by design.
#[test]
fn payload_types_use_no_buffering_serde_attributes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut hits = Vec::new();
    for krate in [
        "db-worker",
        "core-ledger",
        "core-money",
        "core-ids",
        "pay-schedule",
    ] {
        scan(&root.join(krate).join("src"), &mut hits);
    }
    assert!(
        hits.is_empty(),
        "buffering serde attributes in payload types:\n{}",
        hits.join("\n")
    );
}

fn scan(dir: &Path, hits: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(&path, hits);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            for (i, line) in fs::read_to_string(&path).unwrap().lines().enumerate() {
                let t = line.trim_start();
                if t.starts_with("//") || !t.starts_with("#[serde(") {
                    continue;
                }
                let attr = t.replace(' ', "");
                if attr.contains("flatten")
                    || attr.contains("untagged")
                    || attr.contains("tag=")
                    || attr.contains("content=")
                {
                    hits.push(format!("{}:{}: {t}", path.display(), i + 1));
                }
            }
        }
    }
}

/// The "clear" form of `SetBillNotLoanPayment` (`None`) cannot be a second sample (the suite wants
/// exactly one per variant), so it round-trips here: it must encode to a payload that decodes back
/// to the same command and is not the "remember" payload.
#[test]
fn the_clear_form_of_set_bill_not_loan_payment_round_trips() {
    let clear = WriteCommand::SetBillNotLoanPayment {
        event_id: event(14),
        loan_account_id: None,
    };
    let bytes = encode_payload(&clear).unwrap();
    assert!(same_value(&decode_payload(1, &bytes).unwrap(), &clear));
    let remember = WriteCommand::SetBillNotLoanPayment {
        event_id: event(14),
        loan_account_id: Some(account(1)),
    };
    assert_ne!(bytes, encode_payload(&remember).unwrap());
}
