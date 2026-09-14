# ADR 0048: Interval frequencies — every N days / weeks / months

- **Status:** Accepted
- **Date:** 2026-07-11
- **Deciders:** Project owner
- **Beads:** [`personal-cfo-4d8.25.13`](../../.beads/issues.jsonl) (this ADR), gates
  `4d8.25.14` (implementation). Owner feedback 2026-07-09: "For Bill Frequency we need more
  granular options. Every X days, Every X weeks, Every X months."
- **Builds on:** ADR 0021 (date/timezone policy), ADR 0026 (§16 recurring-instance
  projection), ADR 0047 (§3 anchor semantics), the `pay-schedule` crate contract.

## Context

`pay_schedule::Frequency` is a closed six-token enum (`weekly` / `biweekly` / `semi_monthly`
/ `monthly` / `quarterly` / `annual`) shared by recurring bills, income sources, recurring
transfers, forecast expansion, deterministic v5 instance ids, and the card-cycle estimator.
Real bills recur on cadences the closed set cannot express (every 6 weeks, every 2 months,
every 45 days). The enum is a schedule-model seam with wide blast radius, so growing it is an
architecturally-significant decision (AGENTS.md §1A) — this ADR records it before the build.

## Decision

### 1. Parameterized token family, same flat wire shape

Three new `Frequency` variants — `EveryNDays(n)`, `EveryNWeeks(n)`, `EveryNMonths(n)` — with
the stable snake_case token form **`every_<n>_days` / `every_<n>_weeks` / `every_<n>_months`**
(e.g. `every_6_weeks`). Everywhere a frequency travels — DB TEXT columns, the schedule JSON in
`bill_contracts.due_rule_json`, DTO strings, `AssumptionBasis::RecurringSchedule` — it stays a
**flat string token**; serde for the enum is a custom to/from-token round-trip so the six
classic tokens keep their exact serialized form (backward compatible, no migration: frequency
columns carry no CHECK constraint and the typed boundary is the validator).

### 2. Expansion semantics reuse the two existing generators

- `every_n_days` → the fixed-day-interval generator with step `n` (exactly how `weekly`/
  `biweekly` are steps 7/14 today): interval-exact from the anchor, bidirectional alignment
  via floor division.
- `every_n_weeks` → fixed-day-interval with step `7·n`.
- `every_n_months` → the month-stepping generator with step `n` (exactly how `monthly`/
  `quarterly`/`annual` are steps 1/3/12): the **anchor's day-of-month, month-end clamped**,
  each occurrence recomputed from the anchor so a day-31 schedule keeps trying the 31st
  (ADR 0021 discipline; no drift-after-clamp).

No new generator code paths means the ADR 0047 §3 anchor contract carries over unchanged: any
**unclamped** occurrence is an equivalent anchor (the shift-invariance sweep extends to the
new variants), and the month-end-clamp caveat applies to `every_n_months` exactly as to
`monthly`.

### 3. Bounds and validation

`n == 0` is invalid everywhere. Upper bounds keep horizons meaningful and the UI honest:
`days ≤ 366`, `weeks ≤ 52`, `months ≤ 36`. `Frequency::from_token` rejects out-of-bounds or
malformed parameterized tokens (`every_0_days`, `every__weeks`, `every_400_days` → parse
failure), which the existing typed boundaries surface as the standard invalid-frequency
error. Degenerate aliases are permitted, not normalized (`every_1_months` ≠ rewritten to
`monthly`): the stored token preserves what the user chose; display labels may still say
"Monthly".

### 4. Determinism and instance identity

Instance ids remain v5 over `(recurring_event_id, scheduled_date)` — unaffected by token
shape. The six classic tokens expand **byte-identically** to today (regression-tested), so
existing schedules, persisted forecasts, and actualization pairs are untouched.

### 5. Scope (v1)

Bills and recurring transfers gain the custom cadence in their forms ("Every ⟨N⟩
⟨days/weeks/months⟩"). Income sources accept the tokens structurally (shared enum) but the
income form keeps its curated list until a need appears. **Detection is unchanged**: the
detector's cadence bands still emit only the classic tokens; interval cadences are a
manual-entry expressiveness feature, not a detection target (a follow-up may add e.g. a
42-day band if real candidates surface). `semi_monthly` remains the anchor-independent
special case (ADR 0047 §3).

## Consequences

- `Frequency::as_token() -> &'static str` becomes `token() -> String` (parameterized tokens
  are dynamic); `AssumptionBasis::RecurringSchedule.frequency` widens to `String`. Mechanical
  call-site migration, no behavior change for classic tokens.
- Every schedule consumer (forecast events, card cycles, instances, estimator windows,
  commitments) gains interval support for free by parsing through `from_token`.
- The UI needs a token→label formatter that handles both families ("Every 6 weeks").

## Alternatives considered

- **A `(frequency, interval)` column pair.** Rejected: touches every table, DTO, and form for
  a property only custom cadences need; the token family is self-describing and
  backward-compatible.
- **Full RRULE adoption.** Rejected (again — `apso` originally scoped "custom-rrule" and shipped
  without it): RRULE's expressiveness (BYDAY, BYSETPOS, …) far exceeds the owner's ask and
  would put a parser/serializer of a complex grammar on the critical forecast path.
- **Normalizing degenerate aliases at parse time.** Rejected: silently rewriting stored user
  input crosses the non-destructive default; labels can present the friendly name without
  mutating the token.
