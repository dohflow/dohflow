//! Exhaustive ADR 0073 disposition appendix generator.
//!
//! The ADR is the product contract; this test keeps its variant appendix from
//! silently going stale as `WriteCommand` grows. It deliberately parses the
//! enum declaration rather than constructing all 49 payloads, so a new variant
//! without a disposition fails at compile-time test execution before SYNC-2
//! can ship a command with no rebase rule. It also compares every checked-in
//! ADR cell against this table and proves that a one-cell outcome drift fails.

use std::collections::BTreeSet;

const ADR_SOURCE: &str = include_str!("../../../docs/adr/0073-sync-disposition-rules.md");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Create,
    Set,
    Toggle,
    BaseDependent,
    TombstoneFirst,
    ReshapeFirst,
    NeverShips,
}

impl Shape {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "Create",
            Self::Set => "Set",
            Self::Toggle => "Toggle",
            Self::BaseDependent => "Base-dependent",
            Self::TombstoneFirst => "Tombstone first",
            Self::ReshapeFirst => "Reshape first",
            Self::NeverShips => "Never ships",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Disposition {
    variant: &'static str,
    shape: Shape,
    outcome: &'static str,
}

// The generated markdown in docs/adr/0073-sync-disposition-rules.md is copied
// from this test's output.  Rows are grouped by shape for the ADR; the test
// below compares the set against the enum declaration, so order is cosmetic.
const DISPOSITIONS: &[Disposition] = &[
    // Create (8): appends commute when references are valid.
    d(
        "CreateAccount",
        Shape::Create,
        "auto; queue-edit if a reference is invalid",
    ),
    d(
        "RecordTransaction",
        Shape::Create,
        "auto; queue-edit if a reference is invalid",
    ),
    d(
        "Transfer",
        Shape::Create,
        "auto; queue-edit if a reference is invalid",
    ),
    d(
        "CreateRecurringTransfer",
        Shape::Create,
        "auto; queue-edit if a reference is invalid",
    ),
    d(
        "CreateIncomeSource",
        Shape::Create,
        "auto; queue-edit if a reference is invalid",
    ),
    d(
        "CreateRecurringBill",
        Shape::Create,
        "auto; queue-edit if a reference is invalid",
    ),
    d(
        "CreateCategory",
        Shape::Create,
        "auto; queue-edit if a reference is invalid",
    ),
    d(
        "CreateTag",
        Shape::Create,
        "auto; queue-edit if a reference is invalid",
    ),
    // Set (17): every automatic path is gated by the strict untouched-and-
    // valid predicate; conflicts choose or open an existing editor. SetTags
    // remains a replacement payload and is never rewritten as a union.
    d(
        "UpdateAccount",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d(
        "SetAccountSubtype",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d(
        "SetAccountNote",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d(
        "SetAccountLink",
        Shape::Set,
        "queue-edit (liability-link invariant)",
    ),
    d(
        "SetDebtTerms",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d(
        "SetCardStatementBalance",
        Shape::Set,
        "queue-choose (balance assertion; never LWW)",
    ),
    d(
        "UpdateIncomeSource",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d(
        "SetBillAutopay",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d(
        "UpdateRecurringBill",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d(
        "UpdateCategory",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d(
        "RecategorizeTransaction",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d(
        "DismissRecurringSuggestion",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d(
        "MarkReviewed",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d(
        "SetTags",
        Shape::Set,
        "auto iff strict predicate; queue-choose if touched (replacement payload; no union rewrite)",
    ),
    d(
        "SetNote",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d("SetSplits", Shape::Set, "queue-edit (sum invariant)"),
    d("MoveCategory", Shape::Set, "queue-edit (cycle invariant)"),
    // Toggle (12): an equal end state auto-applies only when the strict
    // untouched-and-valid predicate holds; touched or opposed states choose.
    d(
        "ArchiveAccount",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    d(
        "ReinstateAccount",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    d(
        "UnconfirmObligation",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    d(
        "ArchiveIncomeSource",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    d(
        "RestoreIncomeSource",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    d(
        "ArchiveRecurringBill",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    d(
        "RestoreRecurringBill",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    d(
        "SnoozeInboxItem",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    d(
        "DismissInboxItem",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    d(
        "ArchiveCategory",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    d(
        "ReinstateCategory",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    d(
        "VoidTransaction",
        Shape::Toggle,
        "auto iff strict predicate and end state is equal; queue-choose if touched or opposed",
    ),
    // Base-dependent (4): apply against the new base only after re-evaluation.
    d(
        "ConfirmObligationEarly",
        Shape::BaseDependent,
        "auto after idempotent re-evaluation of (event, date)",
    ),
    d(
        "ConvertUnexplainedToTransaction",
        Shape::BaseDependent,
        "auto after residual/assertion re-evaluation",
    ),
    d(
        "ApplyScenario",
        Shape::BaseDependent,
        "queue-choose (reversal handle is command_id)",
    ),
    d(
        "RevertScenarioApply",
        Shape::BaseDependent,
        "queue-choose (reversal handle is command_id)",
    ),
    // Exceptions that must be completed before their command can enter the
    // Sync envelope stream.
    d(
        "DeleteRecurringTransfer",
        Shape::TombstoneFirst,
        "tombstone in SYNC-2b; then Toggle",
    ),
    d(
        "DeleteIncomeSource",
        Shape::TombstoneFirst,
        "tombstone in SYNC-2b; then Toggle",
    ),
    d(
        "DeleteRecurringBill",
        Shape::TombstoneFirst,
        "tombstone in SYNC-2b; then Toggle",
    ),
    d(
        "CreateSourceBatch",
        Shape::ReshapeFirst,
        "reshape to class-1b artifact envelope in SYNC-2c",
    ),
    d(
        "AttachSourceRecord",
        Shape::ReshapeFirst,
        "reshape to class-1b artifact envelope in SYNC-2c",
    ),
    d(
        "UpdateBatchState",
        Shape::ReshapeFirst,
        "reshape to class-1b artifact envelope in SYNC-2c",
    ),
    d(
        "CommitStaged",
        Shape::ReshapeFirst,
        "reshape to materialized transaction in SYNC-2c",
    ),
    d(
        "SkipStaged",
        Shape::NeverShips,
        "never enters an envelope; device-local staging only",
    ),
];

const fn d(variant: &'static str, shape: Shape, outcome: &'static str) -> Disposition {
    Disposition {
        variant,
        shape,
        outcome,
    }
}

fn write_command_variants() -> Vec<&'static str> {
    let source = include_str!("../src/lib.rs");
    let enum_body = source
        .split_once("pub enum WriteCommand")
        .expect("WriteCommand enum must remain present")
        .1
        .split_once("impl WriteCommand")
        .expect("WriteCommand impl must follow its enum")
        .0;

    enum_body
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let first = trimmed
                .split(|c: char| c.is_whitespace() || c == '{' || c == '(' || c == ',')
                .next()?;
            if first.is_empty() || !first.chars().next()?.is_ascii_uppercase() {
                return None;
            }
            if first.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                Some(first)
            } else {
                None
            }
        })
        .collect()
}

type AppendixRow = (String, String, String);

fn parse_adr_appendix(source: &str) -> Result<Vec<AppendixRow>, String> {
    let mut in_table = false;
    let mut rows = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "| `WriteCommand` variant | Shape | Default disposition |" {
            in_table = true;
            continue;
        }
        if in_table && trimmed.starts_with("## ") {
            break;
        }
        if !in_table || !trimmed.starts_with("| `") {
            continue;
        }

        let cells: Vec<_> = trimmed.split('|').map(str::trim).collect();
        if cells.len() != 5 {
            return Err(format!("appendix row has {} cells: {trimmed}", cells.len()));
        }
        let variant = cells[1]
            .strip_prefix('`')
            .and_then(|cell| cell.strip_suffix('`'))
            .ok_or_else(|| format!("appendix variant is not backtick-delimited: {trimmed}"))?;
        rows.push((variant.to_owned(), cells[2].to_owned(), cells[3].to_owned()));
    }

    if !in_table {
        return Err("ADR appendix header is missing".to_owned());
    }
    Ok(rows)
}

fn compare_adr_appendix(source: &str) -> Result<(), String> {
    let actual = parse_adr_appendix(source)?;
    if actual.len() != DISPOSITIONS.len() {
        return Err(format!(
            "ADR appendix has {} rows; generated table has {}",
            actual.len(),
            DISPOSITIONS.len()
        ));
    }

    for (index, (actual, expected)) in actual.iter().zip(DISPOSITIONS).enumerate() {
        let expected = (
            expected.variant.to_owned(),
            expected.shape.as_str().to_owned(),
            expected.outcome.to_owned(),
        );
        if actual != &expected {
            return Err(format!(
                "ADR appendix row {} drifted: actual={actual:?}, expected={expected:?}",
                index + 1
            ));
        }
    }
    Ok(())
}

#[test]
fn disposition_appendix_covers_every_write_command_variant() {
    let variants = write_command_variants();
    assert_eq!(
        variants.len(),
        49,
        "the ADR count must track the current enum"
    );
    assert_eq!(
        DISPOSITIONS.len(),
        variants.len(),
        "every variant needs one row"
    );

    for variant in &variants {
        assert_eq!(
            DISPOSITIONS
                .iter()
                .filter(|row| row.variant == *variant)
                .count(),
            1,
            "every WriteCommand variant needs exactly one disposition row: {variant}"
        );
    }
    for row in DISPOSITIONS {
        assert!(
            variants.contains(&row.variant),
            "disposition row names a variant not present in WriteCommand: {}",
            row.variant
        );
    }

    let shape_count = |shape| DISPOSITIONS.iter().filter(|row| row.shape == shape).count();
    assert_eq!(shape_count(Shape::Create), 8);
    assert_eq!(shape_count(Shape::Set), 17);
    assert_eq!(shape_count(Shape::Toggle), 12);
    assert_eq!(shape_count(Shape::BaseDependent), 4);
    assert_eq!(shape_count(Shape::TombstoneFirst), 3);
    assert_eq!(shape_count(Shape::ReshapeFirst), 4);
    assert_eq!(shape_count(Shape::NeverShips), 1);

    println!("| WriteCommand variant | Shape | Default disposition |");
    println!("| --- | --- | --- |");
    for row in DISPOSITIONS {
        println!(
            "| `{}` | {} | {} |",
            row.variant,
            row.shape.as_str(),
            row.outcome
        );
    }
}

#[test]
fn adr_appendix_matches_generated_rows_cell_for_cell() {
    compare_adr_appendix(ADR_SOURCE).expect("checked-in ADR appendix must match generated rows");
}

#[test]
fn adr_appendix_comparison_rejects_one_cell_outcome_drift() {
    let original = "| `CreateAccount` | Create | auto; queue-edit if a reference is invalid |";
    let drifted = "| `CreateAccount` | Create | queue-choose (drift fixture) |";
    assert_eq!(ADR_SOURCE.matches(original).count(), 1);
    let mutated = ADR_SOURCE.replacen(original, drifted, 1);
    let error =
        compare_adr_appendix(&mutated).expect_err("one outcome cell must fail the contract");
    assert!(error.contains("ADR appendix row 1 drifted"), "{error}");
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VaultState {
    seq: u64,
    touched_rows: BTreeSet<&'static str>,
    archived: bool,
    tags: BTreeSet<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Envelope {
    Toggle {
        target_row: &'static str,
        archived: bool,
    },
    SetTags {
        target_row: &'static str,
        replacement: Vec<&'static str>,
    },
}

fn rebase_two_vaults(
    base_seq: u64,
    new_base: &VaultState,
    envelope: &Envelope,
) -> Result<VaultState, &'static str> {
    let (target_row, validates) = match envelope {
        Envelope::Toggle { target_row, .. } => (*target_row, true),
        Envelope::SetTags {
            target_row,
            replacement,
        } => (*target_row, !replacement.is_empty()),
    };
    let write_set_untouched =
        new_base.seq >= base_seq && !new_base.touched_rows.contains(target_row);
    if !(write_set_untouched && validates) {
        return Err("queue-choose");
    }

    let mut rebased = new_base.clone();
    match envelope {
        Envelope::Toggle { archived, .. } => rebased.archived = *archived,
        Envelope::SetTags { replacement, .. } => {
            rebased.tags = replacement.iter().copied().collect();
        }
    }
    Ok(rebased)
}

fn vault(seq: u64, archived: bool, tags: &[&'static str]) -> VaultState {
    VaultState {
        seq,
        touched_rows: BTreeSet::new(),
        archived,
        tags: tags.iter().copied().collect(),
    }
}

#[test]
fn two_vault_strict_rebase_queues_equal_end_state_toggle_when_touched() {
    let base = vault(10, false, &["base"]);
    let mut remote = base.clone();
    remote.seq = 11;
    remote.touched_rows.insert("account:1");
    remote.archived = true;
    let envelope = Envelope::Toggle {
        target_row: "account:1",
        archived: true,
    };

    assert_eq!(
        rebase_two_vaults(base.seq, &remote, &envelope),
        Err("queue-choose")
    );
    assert!(
        remote.archived,
        "the queued envelope must not rewrite the remote vault"
    );
}

#[test]
fn two_vault_strict_rebase_queues_set_tags_without_union_rewrite() {
    let base = vault(10, false, &["base"]);
    let mut remote = base.clone();
    remote.seq = 11;
    remote.touched_rows.insert("transaction:1");
    remote.tags = ["base", "remote"].into_iter().collect();
    let envelope = Envelope::SetTags {
        target_row: "transaction:1",
        replacement: vec!["base", "local"],
    };

    assert_eq!(
        rebase_two_vaults(base.seq, &remote, &envelope),
        Err("queue-choose")
    );
    assert_eq!(remote.tags, ["base", "remote"].into_iter().collect());
    assert!(
        !remote.tags.contains("local"),
        "rebase must not synthesize a tag union"
    );
}

#[test]
fn two_vault_strict_rebase_auto_applies_only_an_untouched_valid_row() {
    let base = vault(10, false, &["base"]);
    let mut remote = base.clone();
    remote.seq = 11;
    remote.touched_rows.insert("account:2");
    let envelope = Envelope::Toggle {
        target_row: "account:1",
        archived: true,
    };

    let rebased =
        rebase_two_vaults(base.seq, &remote, &envelope).expect("untouched row auto-applies");
    assert!(rebased.archived);
    assert_eq!(rebased.seq, remote.seq);
}
