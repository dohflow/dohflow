//! Exhaustive ADR 0073 disposition appendix generator.
//!
//! The ADR is the product contract; this test keeps its variant appendix from
//! silently going stale as `WriteCommand` grows.  It deliberately parses the
//! enum declaration rather than constructing all 49 payloads, so a new variant
//! without a disposition fails at compile-time test execution before SYNC-2
//! can ship a command with no rebase rule.

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
    // Set (17): thirteen scalar sets choose on conflict; the three structural
    // edits open an existing editor; tags use a commutative set union.
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
        "auto (set union; never removes another device's tag)",
    ),
    d(
        "SetNote",
        Shape::Set,
        "queue-choose (scalar; idempotent no-op is auto)",
    ),
    d("SetSplits", Shape::Set, "queue-edit (sum invariant)"),
    d("MoveCategory", Shape::Set, "queue-edit (cycle invariant)"),
    // Toggle (12): an equal end state is an idempotent auto-apply; opposed
    // states are shown as a choose, never silently last-write-wins.
    d(
        "ArchiveAccount",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
    ),
    d(
        "ReinstateAccount",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
    ),
    d(
        "UnconfirmObligation",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
    ),
    d(
        "ArchiveIncomeSource",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
    ),
    d(
        "RestoreIncomeSource",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
    ),
    d(
        "ArchiveRecurringBill",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
    ),
    d(
        "RestoreRecurringBill",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
    ),
    d(
        "SnoozeInboxItem",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
    ),
    d(
        "DismissInboxItem",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
    ),
    d(
        "ArchiveCategory",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
    ),
    d(
        "ReinstateCategory",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
    ),
    d(
        "VoidTransaction",
        Shape::Toggle,
        "auto if end state is equal; queue-choose if opposed",
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
