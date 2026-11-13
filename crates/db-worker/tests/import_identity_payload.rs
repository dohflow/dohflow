//! The identity a `CommitImportedTransaction` may claim travels in an OPTIONAL member of its payload
//! (`personal-cfo-0g528.3`, ADR 0074 Addendum J item 8): a payload without one is byte for byte what it
//! always was and still decodes, and the claim's own encoding is frozen here, for both kinds.

mod common;

use db_worker::payload::{decode_payload, encode_payload, PAYLOAD_SCHEMA_VERSION};
use db_worker::{ImportIdentityClaim, ImportIdentityComponents, WriteCommand};

use common::samples::{samples, u};

fn commit_without_identity() -> WriteCommand {
    samples()
        .into_iter()
        .find(|(name, _)| *name == "commit_imported_transaction")
        .expect("a sample of the commit")
        .1
}

fn with(claim: ImportIdentityClaim) -> WriteCommand {
    match commit_without_identity() {
        WriteCommand::CommitImportedTransaction {
            transaction_id,
            account_id,
            source_record_id,
            occurred_at,
            amount,
            counterparty,
            memo,
            transaction_date,
            category_id,
            forced,
            ..
        } => WriteCommand::CommitImportedTransaction {
            transaction_id,
            account_id,
            source_record_id,
            occurred_at,
            amount,
            counterparty,
            memo,
            transaction_date,
            category_id,
            forced,
            identity: Some(claim),
        },
        _ => unreachable!(),
    }
}

fn text(command: &WriteCommand) -> String {
    String::from_utf8(encode_payload(command).unwrap()).unwrap()
}

#[test]
fn a_payload_without_an_identity_has_no_member_for_it_and_still_decodes() {
    let json = text(&commit_without_identity());
    assert!(!json.contains("identity"), "{json}");
    // The frozen fixture of the variant (written before the member existed) is the same bytes.
    let fixture = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/payload_v1/commit_imported_transaction.json"),
    )
    .unwrap();
    assert_eq!(json, fixture.trim_end());
    let decoded = decode_payload(PAYLOAD_SCHEMA_VERSION, fixture.trim_end().as_bytes()).unwrap();
    assert_eq!(
        format!("{decoded:?}"),
        format!("{:?}", commit_without_identity())
    );
}

#[test]
fn a_provider_claim_is_frozen_and_round_trips() {
    let command = with(ImportIdentityClaim {
        identity_id: u(700),
        components: ImportIdentityComponents::Provider {
            connection_id: u(701),
            provider_account_key: "ACT-9".to_owned(),
            provider_txn_id: "TXN-1".to_owned(),
        },
    });
    let json = text(&command);
    assert!(
        json.contains(
            r#""identity":{"components":{"provider":{"connection_id":"0198c2c1-5f0a-7c3d-8a4e-9b1d000002bd","provider_account_key":"ACT-9","provider_txn_id":"TXN-1"}},"identity_id":"0198c2c1-5f0a-7c3d-8a4e-9b1d000002bc"}"#
        ),
        "{json}"
    );
    let decoded = decode_payload(PAYLOAD_SCHEMA_VERSION, json.as_bytes()).unwrap();
    assert_eq!(format!("{decoded:?}"), format!("{command:?}"));
    assert_eq!(text(&decoded), json, "one canonical encoding");
}

#[test]
fn a_file_claim_is_frozen_and_round_trips() {
    let command = with(ImportIdentityClaim {
        identity_id: u(702),
        components: ImportIdentityComponents::File {
            file_fingerprint: common::samples::ORACLE_FINGERPRINT.to_owned(),
            row_ordinal: 2,
        },
    });
    let json = text(&command);
    assert!(
        json.contains(
            r#""identity":{"components":{"file":{"file_fingerprint":"sha256:2c30d6d4f9c6954bfc4f2e31460da5660e8d4aa9d893b6228eddc467516a6830","row_ordinal":"2"}},"identity_id":"0198c2c1-5f0a-7c3d-8a4e-9b1d000002be"}"#
        ),
        "{json}"
    );
    let decoded = decode_payload(PAYLOAD_SCHEMA_VERSION, json.as_bytes()).unwrap();
    assert_eq!(format!("{decoded:?}"), format!("{command:?}"));
}

#[test]
fn an_unknown_member_in_a_claim_is_refused() {
    let command = with(ImportIdentityClaim {
        identity_id: u(703),
        components: ImportIdentityComponents::File {
            file_fingerprint: common::samples::ORACLE_FINGERPRINT.to_owned(),
            row_ordinal: 1,
        },
    });
    let json = text(&command).replace(r#""identity_id""#, r#""extra":"x","identity_id""#);
    assert!(decode_payload(PAYLOAD_SCHEMA_VERSION, json.as_bytes()).is_err());
}
