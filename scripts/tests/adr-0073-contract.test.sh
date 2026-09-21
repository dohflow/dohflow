#!/usr/bin/env bash
#
# Contract regression test for ADR 0073 (personal-cfo-vlfd).
#
# The ADR is the protocol contract consumed by the SYNC-2b, SYNC-2c, and S3-2
# beads.  These assertions guard the safety rules and the exhaustive appendix;
# the Rust test is the source-of-truth check that the appendix covers the live
# WriteCommand enum.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
ADR="$REPO_ROOT/docs/adr/0073-sync-disposition-rules.md"
ADR074="$REPO_ROOT/docs/adr/0074-dohflow-sync-architecture.md"
TEST="$REPO_ROOT/crates/db-worker/tests/adr_0073_disposition.rs"

for file in "$ADR" "$ADR074" "$TEST"; do
  if [ ! -f "$file" ]; then
    echo "FAIL — missing document or test: $file" >&2
    exit 1
  fi
done

CASES=0
require_text() {
  local label="$1" file="$2" needle="$3"
  CASES=$((CASES + 1))
  if grep -Fq -- "$needle" "$file"; then
    echo "    ok   — $label"
  else
    echo "    FAIL — $label (missing: $needle)" >&2
    exit 1
  fi
}

require_text "ADR is accepted" "$ADR" "- **Status:** Accepted"
require_text "ADR is public" "$ADR" "- **Tier:** Public"
require_text "ADR names the bead" "$ADR" "- **Bead:** \`personal-cfo-vlfd\`"
require_text "ADR identifies the owner decision" "$ADR" "program plan v0.8.1 §9.4"
require_text "ADR builds on CAS and replay" "$ADR" "ADR 0074](0074-dohflow-sync-architecture.md)"
require_text "ADR builds on balance assertions" "$ADR" "ADR 0027 (additive balance assertions)"
require_text "ADR builds on scenario handles" "$ADR" "ADR 0055 (scenario reversal handles)"
require_text "CommandMeta carries command id" "$ADR" "\`command_id\`"
require_text "CommandMeta carries correlation id" "$ADR" "\`correlation_id\`"
require_text "CommandMeta carries causation id" "$ADR" "\`causation_id\`"
require_text "source points at CommandMeta" "$ADR" "crates/db-worker/src/lib.rs:174-188"
require_text "plan pointer is reconciled" "$ADR" "finance-kernel/lib.rs:176"
require_text "kernel propagation is cited" "$ADR" "finance-kernel/src/lib.rs:2503-2505"
require_text "source points at WriteCommand" "$ADR" "crates/db-worker/src/lib.rs:223-698"
require_text "hard delete sites are named" "$ADR" "recurring_transfers\` at \`:93-108\`"
require_text "income delete site is named" "$ADR" "income_sources\` at \`:264-278\`"
require_text "bill delete site is named" "$ADR" "bill_contracts\` plus \`recurring_events\` at \`:633-654\`"

require_text "three outcomes are explicit" "$ADR" "There are exactly three user-visible dispositions"
require_text "auto outcome is defined" "$ADR" "**Auto.** Reapply the envelope"
require_text "queue-choose outcome is defined" "$ADR" "**Queue-choose.** Preserve both"
require_text "queue-edit outcome is defined" "$ADR" "**Queue-edit.** Open the existing editor"
require_text "auto iff rule is explicit" "$ADR" "The rule is **auto iff** both predicates hold"
require_text "whole correlation group queues" "$ADR" "the whole \`correlation_id\` group queues"
require_text "causal descendants queue" "$ADR" "causal descendant (\`causation_id\`)"
require_text "no data is dropped" "$ADR" "No financial data is"

require_text "Create shape is counted" "$ADR" "#### Create (8)"
require_text "Set shape is counted" "$ADR" "#### Set (17)"
require_text "Toggle shape is counted" "$ADR" "#### Toggle (12)"
require_text "base-dependent shape is counted" "$ADR" "#### Base-dependent (4)"
require_text "create invalid references edit" "$ADR" "reference opens **queue-edit**"
require_text "set structural exceptions edit" "$ADR" "structural Sets are **queue-edit**"
require_text "tags use set union" "$ADR" "**auto**: tags merge by set union"
require_text "balance assertions never LWW" "$ADR" "it is never LWW"
require_text "toggle opposed state chooses" "$ADR" "Equal end states auto-apply; opposed states produce"
require_text "confirm is idempotent" "$ADR" "idempotent re-evaluation of"
require_text "convert rechecks residual" "$ADR" "after re-reading the latest"
require_text "scenario uses reversal handle" "$ADR" "\`command_id\` is ADR 0055's reversal handle"

require_text "tombstone exception is counted" "$ADR" "**Tombstone first (3).**"
require_text "reshape exception is counted" "$ADR" "**Reshape first (4).**"
require_text "never-ships exception is counted" "$ADR" "**Never ships (1).**"
require_text "staging bypass is forbidden" "$ADR" "a replica never looks up an origin"
require_text "rough tally is recorded" "$ADR" "23 default-auto rows, 15 scalar/Scenario queue-choose rows"

require_text "audit base sequence is retained" "$ADR" "\`applied_base_seq\` and \`rebased_from\`"
require_text "command is not applied twice" "$ADR" "never applies a command twice"
require_text "queue age is visible" "$ADR" "shows a nag at 24 hours"
require_text "simulator case one is required" "$ADR" "pull-first with an untouched valid envelope"
require_text "simulator seed-twin case is required" "$ADR" "seed-twin bootstrap"
require_text "simulator queue visibility case is required" "$ADR" "a queued correlation group visible"
require_text "simulator audit case is required" "$ADR" "audit row records \`applied_base_seq\` / \`rebased_from\`"
require_text "simulator is property-based" "$ADR" "The exit test is property-based"
require_text "simulator randomizes streams" "$ADR" "random command"
require_text "simulator randomizes push order" "$ADR" "random push interleavings"
require_text "class one and two converge" "$ADR" "class-1 and class-2"
require_text "simulator forbids duplicate application" "$ADR" "no envelope is lost or applied twice"
require_text "staging cannot bypass the outbox" "$ADR" "device-local staging without the outbox"

require_text "appendix is generated" "$ADR" "The following table is generated by the \`#[test]\` named above"
require_text "Rust appendix test is named" "$ADR" "crates/db-worker/tests/adr_0073_disposition.rs"
require_text "new variants fail closed" "$ADR" "A new variant without a row fails the test"
require_text "rejected alternatives are present" "$ADR" "## Rejected alternatives"
require_text "revisit conditions are present" "$ADR" "## Revisit if"
require_text "downstream bead paths are present" "$ADR" "personal-cfo-egn67\` (SYNC-2b)"
require_text "S3 consumer path is present" "$ADR" "personal-cfo-mpep3\` (S3-2)"

variants=(
  CreateAccount UpdateAccount ArchiveAccount ReinstateAccount SetAccountSubtype
  SetAccountNote SetAccountLink SetDebtTerms SetCardStatementBalance RecordTransaction
  ConfirmObligationEarly UnconfirmObligation ConvertUnexplainedToTransaction Transfer
  CreateRecurringTransfer DeleteRecurringTransfer CreateIncomeSource UpdateIncomeSource
  DeleteIncomeSource ArchiveIncomeSource RestoreIncomeSource CreateRecurringBill
  SetBillAutopay UpdateRecurringBill DeleteRecurringBill ArchiveRecurringBill
  RestoreRecurringBill CreateSourceBatch AttachSourceRecord UpdateBatchState CommitStaged
  SkipStaged SnoozeInboxItem DismissInboxItem DismissRecurringSuggestion CreateCategory
  UpdateCategory MoveCategory ArchiveCategory ReinstateCategory RecategorizeTransaction
  VoidTransaction MarkReviewed ApplyScenario RevertScenarioApply CreateTag SetTags SetNote
  SetSplits
)
for variant in "${variants[@]}"; do
  require_text "appendix maps $variant" "$ADR" "| \`$variant\` |"
done

require_text "test defines the table" "$TEST" "const DISPOSITIONS: &[Disposition]"
require_text "test asserts live enum count" "$TEST" "variants.len(),"
require_text "test pins the current enum count" "$TEST" "        49,"
require_text "test asserts shape counts" "$TEST" "assert_eq!(shape_count(Shape::Create), 8"
require_text "test rejects unmapped variants" "$TEST" "exactly one disposition row"
require_text "0074 links accepted ADR 0073" "$ADR074" "ADR 0073 (sync disposition rules, accepted"

echo "PASS — $CASES ADR 0073 contract assertions, 0 failures"
