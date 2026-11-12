//! `closure_references` (`db_worker::sync_references`, vault-format.md C.9; `personal-cfo-0g528.2`
//! D4): the class-1b objects a command's payload references. The function is an exhaustive match
//! with no wildcard arm, so a new `WriteCommand` variant is a compile error there; these tests pin
//! the decisions it makes and make a new id-carrying variant a test failure, not a silent default.

use std::collections::BTreeSet;

use chrono::Utc;
use core_ledger::{AccountId, SourceBatchId, SourceRecordId, TransactionId};
use core_money::{Currency, Money};
use db_worker::sync_references::closure_references;
use db_worker::WriteCommand;
use sync_core::artifact_closure::PayloadRef;

const LIB: &str = include_str!("../src/lib.rs");
const REFERENCES: &str = include_str!("../src/sync_references.rs");

/// The `WriteCommand` variants that carry a source batch, source record or attachment id, read from
/// the enum's declaration the way the payload-reference lint reads it.
fn carriers() -> BTreeSet<String> {
    let lines: Vec<&str> = LIB.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with("pub enum WriteCommand {"))
        .expect("the enum");
    let end = (start..lines.len())
        .find(|&i| lines[i] == "}")
        .expect("its end");
    let mut variant = String::new();
    let mut found = BTreeSet::new();
    for line in &lines[start + 1..end] {
        let trimmed = line.trim_start();
        if line.starts_with("    ") && !line.starts_with("     ") {
            let name: String = trimmed
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.chars().next().is_some_and(char::is_uppercase) {
                variant = name;
            }
        }
        if !trimmed.starts_with("//")
            && ["SourceBatchId", "SourceRecordId", "AttachmentId"]
                .iter()
                .any(|ty| trimmed.contains(ty))
        {
            found.insert(variant.clone());
        }
    }
    found
}

#[test]
fn an_imported_transaction_references_the_record_it_was_imported_from() {
    let record = SourceRecordId::new();
    let command = WriteCommand::CommitImportedTransaction {
        transaction_id: TransactionId::new(),
        account_id: AccountId::new(),
        source_record_id: record,
        occurred_at: Utc::now(),
        amount: Money::new(-1_250, Currency::Usd),
        counterparty: None,
        memo: None,
        transaction_date: None,
        category_id: None,
        forced: false,
    };
    assert_eq!(
        closure_references(&command),
        vec![PayloadRef::Record(*record.as_uuid().as_bytes())]
    );
}

#[test]
fn the_local_import_path_names_no_existing_artifact() {
    let batch = SourceBatchId::new();
    for command in [
        WriteCommand::UpdateBatchState {
            batch_id: batch,
            status: "staged".to_owned(),
            staged_count: 1,
            committed_count: 0,
            skipped_count: 0,
        },
        WriteCommand::CreateSourceBatch {
            id: batch,
            source_type: "csv".to_owned(),
            source_name: None,
            file_fingerprint: None,
            parser_version: None,
            connector_connection_id: None,
        },
    ] {
        assert!(closure_references(&command).is_empty());
    }
}

#[test]
fn every_variant_that_carries_a_batch_record_or_attachment_id_is_a_decision_made_here() {
    // The four commands that carry such an id today. A new one fails this test until its author has
    // decided what it references and added it to the match in `sync_references.rs`; the match itself
    // already fails to compile for any new variant.
    let expected: BTreeSet<String> = [
        // Writes a record's batch row on this device.
        "CreateSourceBatch",
        // Writes a record row on this device.
        "AttachSourceRecord",
        // Moves a batch's device-local state.
        "UpdateBatchState",
        // Names the record it was imported from: the only one that references an artifact.
        "CommitImportedTransaction",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(carriers(), expected);
}

#[test]
fn the_match_has_no_wildcard_arm() {
    let code: String = REFERENCES
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("_ =>") && !code.contains("_ if"),
        "a wildcard arm would turn a new variant into a silent default"
    );
}
