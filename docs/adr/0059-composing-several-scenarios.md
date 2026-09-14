# ADR 0059 — Composing several scenarios: selection order is precedence

- **Status:** Accepted
- **Date:** 2026-08-07
- **Bead:** `personal-cfo-4d8.27.6.4`
- **Amends:** ADR 0026 §5 (a run is base events plus *a* scenario's events)
- **Enables:** `personal-cfo-4d8.27.6.5` (conflict diff + reconcile)

## Context

A forecast run is base events plus one selected scenario's events. Households do not think
in one change at a time: *"what if I take the new job **and** the rent goes up?"* is the
question, and answering it today means authoring a third scenario that duplicates both.

The machinery is nearly there — `entity_overrides` already merges two sources (base and one
scenario) into a per-entity override map. Widening the input to a set is small. **Deciding
what happens when two selected scenarios touch the same field is the actual decision.**

Today every rule inside that merge is **creation-order**: overlapping amount windows
resolve later-`created_at`-wins, and anchor/exclusion overrides are last-write-wins over the
same ordering.

## Decision

### 1. The selection is an ordered list, and its order is the precedence

Scenarios compose in the order the user stacked them. A later scenario's event beats an
earlier one's for the same entity and field. Within a single scenario, creation order still
breaks ties exactly as it does today.

Precedence is therefore `(selection_rank, created_at, id)` — a total order, so composition
is deterministic and reproducible for a persisted run.

Creation-order-across-everything was the alternative, and it is what the code does now. It
was rejected because it makes the user's order decorative: the answer to *"which change
wins?"* would depend on invisible timestamps, and two scenarios authored months apart would
resolve in an order the user cannot see or change. On a surface whose whole job is *"what
if?"*, the user has to be able to say **"…and then this"**.

### 2. Base is always the lowest precedence

Base events rank below every selected scenario, regardless of when they were created.

This **fixes a latent inversion**. Under pure creation-ordering, a base event authored after
a scenario's event on the same field wins — so an overlay silently fails to overlay. That is
not a hypothetical after ADR 0055: applying a scenario promotes its events into base with a
*fresh* `created_at`, so any later-viewed scenario touching the same bill would have lost to
the thing it was meant to override.

### 3. Conflicts compose rather than block

Two scenarios touching the same field is **not an error**. The later one wins and the run
proceeds.

Refusing to compose overlapping scenarios was considered and rejected: the common case is
two unrelated plans that happen to share one bill, and making that unusable until the user
reconciles would be heavy friction for a question the precedence rule already answers.

What conflicts *do* deserve is **visibility** — the user should be able to see that one
scenario is overriding another rather than discover it in a number. That is `4d8.27.6.5`,
and this ADR is what gives it a rule to describe: a conflict is two selected scenarios with
an active event on the same `(kind, target_entity_id)` and overlapping effect windows, and
the winner is the later-stacked one.

### 4. A single selection behaves exactly as before

One scenario composes to the same result it does today, so no existing forecast changes —
except where §2 corrects the inversion, which is a fix rather than a regression.

## Consequences

- `entity_overrides`, `category_spend_adjustments`, and the forecast entry points take an
  ordered `&[Uuid]` instead of `Option<Uuid>`. Empty means base-only, which is what `None`
  meant.
- **Persisted runs must store the ordered selection**, not a single id, or a stored run
  cannot be reproduced. A run recorded against an unordered set is not reproducible, which
  would break the determinism ADR 0026 §15 depends on.
- The scenario picker becomes multi-select, and its **order must be visible and
  reorderable** — a precedence the user cannot see is as good as no precedence. If reorder
  does not fit the first implementation, the order must at least be shown, and the gap filed
  (it did not, and the gap is `personal-cfo-8n24`).
- `4d8.27.6.5` gains a defined conflict predicate to diff against.
- Nothing here applies anything. Composition is a *read-time* overlay; ADR 0055 still owns
  the one path by which scenario content becomes base content.

## Addendum, 2026-08-07 — the conflict predicate as implemented (`4d8.27.6.5`)

§3 defined a conflict as *two selected scenarios with an active event on the same
`(kind, target_entity_id)` and overlapping effect windows*. Implementing the detector
forced four details worth recording, because each one is a decision about what counts as a
contradiction rather than a coincidence:

1. **Only windowed, entity-targeting kinds can conflict** — `bill_amount`, `income_amount`,
   `bill_date`, `income_date`, `exclusion`. The list is closed rather than "anything with a
   target".
2. **`one_time_event` is excluded on purpose.** Two one-offs on the same day are two
   separate cash movements; summing them is correct and flagging them would be noise.
3. **An absent window end is unbounded.** No `effective_date` means "from always", no
   `end_date` means "until always" — so an open-ended change conflicts with anything on the
   same field. Treating absent as "no window" would silently miss the most common case,
   since most overrides carry no dates at all.
4. **Two events from the same source never conflict.** Within one scenario, creation order
   already settles it (§1) — that is authorship, not a contradiction the user must resolve.
5. **A malformed event is not reported.** If its params will not parse, the detector stays
   quiet rather than claiming a specific contradiction it cannot substantiate.

**Where the diff is shown.** On **apply**, not only on the compose path. Composition is a
read-time overlay the user undoes by deselecting; applying **promotes events into base**
(ADR 0055) and supersedes what they collide with, so that is the moment a conflict stops
being a view and becomes durable.

**"Accept one" is the confirm/cancel it already had**, made informed: proceeding accepts the
scenario's value (later wins, §1), cancelling keeps the existing one. **"Author a third"** —
merging two conflicting scenarios into a new one — is a separate feature and is filed as
`personal-cfo-c3pq` rather than half-built.
