# ADR 0055 — Applying a scenario promotes its events to base; it never rewrites entities

- **Status:** Accepted
- **Date:** 2026-08-02
- **Beads:** `personal-cfo-4d8.27.6.3` (this decision + its implementation),
  `4d8.27.6.4` (multi-scenario compose), `4d8.27.6.5` (conflict diff/reconcile)
- **Fills the gap left by:** ADR 0051 §4 — "applying a scenario onto real data is the sole
  path by which scenario content becomes base content… it gets its own decision"
- **Builds on:** ADR 0026 §4/§5 (the assumption-event model), ADR 0011 (hybrid ledger +
  operation log)
- **Constrained by:** AGENTS.md §1 — financial history is append-friendly; use reversals
  and superseding records, never invisible destructive mutation

## Context

A scenario is a set of forecast assumption events carrying a `scenario_id`. Base is the
same table with `scenario_id IS NULL`, and a run for scenario X is *base events plus X's
events* (ADR 0026 §5). Everything about scenarios so far — create, clone, archive, expire,
delete — was careful to never touch the user's real numbers (ADR 0051 §4).

Applying is where that changes. It is the first operation in this area that alters what
the household's forecast actually says, so the question the bead posed — **rewrite the
entities, or promote the events?** — is the entire risk of the feature.

## Decision

### 1. Apply inserts a base copy of each active scenario event

For every `Active` assumption event carrying the scenario's id, insert a new event with
`scenario_id = NULL`, the same kind / target / params, and
`promoted_from_scenario_id = <the scenario>`.

It does **not** edit recurring bills, income sources, accounts, or any ledger row.

Four reasons, in order of weight:

**a. It covers every assumption kind. Rewriting entities covers a third of them.**

Of the twelve kinds in `AssumptionKind`, only `bill_amount`, `bill_date`, `income_amount`
and `recurring_debt_payment` have an entity whose column could be rewritten.
`inflation_rate`, `minimum_cash_floor`, `variable_spend_override`,
`card_payment_behavior`, `one_time_event`, `exclusion`, `scenario_toggle` and
`income_date` are model parameters or synthetic events with no row to update. An apply
path that silently drops two-thirds of what a scenario can express — while telling the
user it applied the scenario — is the worst available outcome on a financial surface.

**b. It introduces no new forecast semantics.** The pipeline already derives base from
`scenario_id IS NULL`. A promoted event is an ordinary base event; nothing downstream
learns a new concept.

**c. It is reversible by construction.** Promotion *inserts*; revert marks those inserts
`cleared`, which is an existing and already-reversible `AssumptionStatus`. No prior state
is overwritten, so revert is exact rather than reconstructed — the same shape as
`VoidTransaction`, which writes a reversing entry rather than deleting.

**d. It touches no ledger rows.** A scenario is a set of assumptions; applying one changes
assumptions. The blast radius is the forecast, not the ledger — and on this app that
distinction is the difference between a recoverable mistake and a corrupted history.

**Rejected: rewriting entities.** Beyond (a), it destroys the prior value unless a shadow
copy is kept, which is a worse version of what the event table already does natively. It
would also make "revert" mean "write the old amount back", which is indistinguishable in
the ledger from the user having edited it twice.

### 2. A promoted event supersedes the base event it collides with

When base already holds an `Active` event with the same
`(kind, target_entity_type, target_entity_id)`, the promoted event supersedes it: the old
row gets `status = 'superseded'` and `superseded_by = <new id>`. Revert restores it to
`Active`.

This is the existing supersede machinery, not new mechanism. Two events of the same kind
against the same target must not both be `Active` — the forecast would have to pick one
arbitrarily, and which one it picked would be an accident of row order.

The *interactive* story for collisions — showing the user what will be overwritten before
they commit — is `4d8.27.6.5`. This ADR defines only the rule that makes a single apply
correct on its own.

#### Amendment to §2 (2026-11-01, `personal-cfo-5ie.13`): only collision kinds supersede

§2 assumed one `Active` event per `(kind, target_entity_type, target_entity_id)`. That is false
for a kind with several instances against one target, and for a kind with no target at all. A
free-standing `one_time_event` (and every manual future entry) has a NULL target type and a NULL
target id, so promoting one scenario one-off superseded **every** active base one-off, the user's
manual future entries included, and two one-offs in one scenario superseded each other as they
were promoted (only the last survived). The same held for `recurring_debt_payment`, and for amount
overrides: two disjoint windows of one scenario on one bill superseded each other.

**The rule.** The forecast reads at most one event per `(kind, target)` for a *collision kind*, so
promoting a second one for the same key replaces the first (the supersession of §2, unchanged). It
reads every active instance of an *adding kind*, so promoting one adds a row and replaces nothing.
Apply decides by the kind alone, through one exhaustive function
(`AssumptionKind::supersedes_on_promotion`); a new kind does not compile until it is classified.

The test is the invariant behind §1b: after an apply, the forecast base reads must equal what a run
of "base plus this scenario" showed, because a promoted event is an ordinary base event. Base ranks
below every scenario in the preview (ADR 0059 section 2), and a promoted row is the newest base
row, so for a collision kind the later event winning is exactly what the preview showed.

| Kind | Class | Why, from how the forecast reads it |
| --- | --- | --- |
| `bill_amount` | ADDS | Targets a recurring bill; `entity_overrides` keeps every amount segment and the later event wins only where windows overlap, so the preview applies base windows the scenario's window does not overlap. |
| `income_amount` | ADDS | The same, for an income source. |
| `bill_date` | COLLIDES | One anchor per target; the later event wins (`entity_overrides`). |
| `income_date` | COLLIDES | The same. |
| `exclusion` | COLLIDES | One exclusion per target; the later event wins (`entity_overrides`). The target is the excluded bill or income source (its type is stored NULL, its id is not). |
| `minimum_cash_floor` | COLLIDES | A singleton model parameter: one floor. No loader reads the kind yet (the shipped floor is a setting); classified by what it means. |
| `inflation_rate` | COLLIDES | A singleton model parameter: one rate. No loader reads the kind yet. |
| `card_payment_behavior` | COLLIDES | One behavior per target account. No loader reads the kind yet. |
| `scenario_toggle` | COLLIDES | One switch per target. No loader reads the kind yet, and nothing writes it. |
| `one_time_event` | ADDS | A free-standing cash event with no target; every active instance is folded (`collect_manual_events`). |
| `recurring_debt_payment` | ADDS | A free-standing monthly payment with no target; every active instance is folded (`active_recurring_debt_payments`). |
| `variable_spend_override` | ADDS | Targets a category, but the cuts are additive across base and the selected scenarios ("membership is enough", `category_spend_adjustments`), so replacing a base cut by a scenario's would change the total the preview showed. |

The four kinds nothing reads yet are classified by what they mean, so the day a loader appears the
apply already does the right thing. Reclassifying one later is a change of what apply writes and so
a change of `engine_version`.

**Ordering of the promoted rows.** An adding kind that composes by window (`bill_amount`,
`income_amount`) resolves overlapping windows by creation order, so apply promotes a scenario's
events in the scenario's own `(created_at, id)` order and stamps the k-th promoted row
`created_at = issued_at + k` microseconds. The promoted rows therefore keep the order the scenario
had, and a scenario may legitimately hold overlapping windows: they are not refused.

**Revert is unchanged in shape.** It restores whatever a promoted row superseded and clears the
promoted rows; for an adding kind there is nothing to restore, so revert clears exactly the rows
apply added and leaves the base events as they were. A vault that applied a scenario before this
amendment may hold base events apply wrongly superseded (including a base amount override a
scenario's window did not overlap): a read-only check counts them for every adding kind, and any
repair is a data change the owner approves first.

### 3. Apply is recorded on the operation log, exceptionally

Assumption events normally bypass the `WriteCommand` bus: they are forecast inputs, not
ledger mutations (`crates/db-worker/src/assumptions.rs` module doc). Apply goes through
the bus anyway.

The exception is deliberate. Apply is the one assumption-layer operation that changes what
the user's real forecast says, and the operation id is what makes it undoable: it is
stored on the scenario as the reversal handle. An operation with a reversal handle belongs
in the log that records operations.

### 4. Applied-ness is a timestamp, not a status

`scenarios.applied_at` + `scenarios.applied_op_id`, rather than a fourth `status` token.

Status is a lifecycle (`draft` / `active` / `archived`) and applied-ness is orthogonal to
it: an applied scenario can still be archived, and archiving it must not un-apply anything
— the promoted events are base events now and stand on their own. Folding applied-ness
into `status` would make those two ideas fight over one column.

### 5. Revert restores, it does not delete

`RevertScenarioApply` sets every event with this scenario's `promoted_from_scenario_id`
to `cleared`, restores anything they superseded to `Active`, and clears `applied_at` /
`applied_op_id`. The promoted rows stay in the table as history.

Reverting is only defined for a scenario that is currently applied; on one that is not, it
is an error rather than a silent no-op, because "revert" succeeding when nothing was
reverted is how a user comes to believe a change was undone.

## Schema (migration v47 — additive and nullable)

- `scenarios.applied_at TEXT`
- `scenarios.applied_op_id BLOB`
- `forecast_assumption_events.promoted_from_scenario_id BLOB`

Three nullable column adds, no data rewrite, no backfill. Migrations apply to the owner's
real encrypted vault on next open, so additive-nullable is the only shape worth taking
here.

## Consequences

- **The Bills screen and the forecast will disagree, visibly.** An applied `bill_amount`
  means the forecast uses the new amount while the bill's stored amount is unchanged —
  which is correct under this decision, and confusing if unsaid. The bill must show that
  it is adjusted by an applied scenario. Tracked as `personal-cfo-abhr`, which blocks on
  this bead; this ADR is what makes it necessary.
- Applying twice is idempotent: a scenario already carrying `applied_at` is rejected
  rather than promoted again.
- A scenario with no active events does not become "applied" — there is nothing to
  promote, and marking it applied would offer a revert that undoes nothing.
- `4d8.27.6.4` (compose) and `4d8.27.6.5` (conflict reconcile) build on this. Compose in
  particular will need a rule for two scenarios promoting colliding events in one gesture;
  §2 gives it a single-apply base to extend rather than redefine.
- Nothing here lets a scenario reach the ledger. If a future feature wants "apply" to
  create real transactions, that is a different operation with a different name and its
  own ADR — it must not be folded into this one.
