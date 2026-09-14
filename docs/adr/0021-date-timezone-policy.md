# ADR 0021: Date and timezone policy

- **Status:** Accepted
- **Date:** 2026-06-20
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-z3kx`](../../.beads/issues.jsonl)
- **Related plan sections:** §9.1.1, §13.3
- **Supersedes:** None

## Context

Dates and times are a recurring source of off-by-one and DST bugs in financial
software. Several date/time decisions are **already implemented** in the
`pay-schedule` and `forecast-engine` crates but were never recorded as a
decision. This ADR captures the policy so it is enforceable and so the next
date-handling code path doesn't re-litigate it. It pairs with implementation bead
`personal-cfo-rr0` and the cross-cutting test suite `personal-cfo-rp1r`.

## Decision

### 1. The household timezone is authoritative for calendar boundaries

"Today", "this month", and the forecast horizon are computed in the **household
timezone** (an IANA zone stored in `vault_metadata.household_timezone`, default
`UTC`) — **never** the machine's local zone. A user travelling, or running on a
laptop set to a different zone, still sees boundaries in their household zone.

### 2. Schedules are timezone-free calendar dates

Pay dates and bill due dates are **`NaiveDate`** — calendar dates with no
time-of-day and no zone. "Due June 5" is the same date in every timezone. The
`pay-schedule` crate is pure `NaiveDate`; DST and zones never enter it.

### 3. The forecast resolves "today" from an instant, then folds calendar days

The deterministic forecast (`forecast-engine`, ADR/plan §13.3) takes an **as-of
instant** supplied by the caller (the impure side reads the clock once),
resolves the **household-local start date** via the household tz
(`Horizon::window`), and folds over **local calendar days**. Bucketing on the
local calendar date makes the fold **DST-immune by construction**.

### 4. DST invariant

An event whose local date falls on a spring-forward or fall-back day still lands
on **that local date**. (Converting each event to a UTC instant and bucketing by
UTC day would reintroduce this bug — we deliberately do not.)

### 5. Storage formats

- Calendar dates: ISO **`YYYY-MM-DD`** TEXT.
- Instants/timestamps: **RFC 3339 UTC**.
- Ordering across instants uses the hybrid-logical-clock fields (ADR 0011 / §9.1.2).

### 6. Display formatting

The frontend renders a bare `YYYY-MM-DD` via `lib/format.formatIsoDate`, which
constructs a **local** `Date` from the components so a calendar date is never
shifted to the previous day in negative-UTC-offset zones (where
`new Date("2026-06-05")` would land on June 4). Instants render via locale
formatting (`formatDate`). Per-locale number/date display is `personal-cfo-me79`.

### 7. Purity: the clock is read only at impure boundaries

`Utc::now()` is called **only** in adapters / Tauri commands, never inside the
pure engines (`forecast-engine`, `pay-schedule`). Engines take instants/dates as
inputs, which is what makes them deterministic and snapshot-testable.

### 8. Ingestion normalization (forward rule — not yet built)

When import/connector ingestion lands, provider **UTC timestamps normalize to the
household tz** on ingestion (§9.1.1); a provider's calendar-only date is taken as
household-local. Recorded here so ingestion inherits the policy instead of
inventing its own.

### 9. Household-timezone change

Changing the household tz leaves calendar-date schedules **unaffected** (they are
tz-free); only boundary resolution ("today"/horizon) shifts. Covered by the
`personal-cfo-v53f` cross-cutting test.

### Libraries

`chrono` (`NaiveDate`, `DateTime<Utc>`) for dates/instants; `chrono-tz` for the
IANA zone database. No wall-clock-reading types in pure crates.

## Consequences

### Positive

- Deterministic, DST-safe schedules and forecasts; no "laptop zone" bugs.
- Pure engines stay clock-free → snapshot- and property-testable.
- One recorded policy that ingestion, display, and future date code inherit.

### Negative

- Engines must be handed an as-of instant rather than calling `now()` — a small,
  intentional threading cost.
- The household tz must be set at vault creation and is a migration concern if its
  semantics change.

## Rejected alternatives

- **Use the machine's local zone for boundaries.** ✗ Wrong when the laptop zone
  differs from where the household actually budgets.
- **Store schedules as UTC instants.** ✗ Reintroduces DST/midnight bugs — the
  exact failure this policy avoids.
- **Read the clock inside the engines.** ✗ Destroys determinism and testability.

## Revisit if

- Multi-household or per-account timezones become a requirement.
- Sub-daily (intraday) forecasting is added — that would need explicit
  within-day time-of-day + zone handling beyond calendar-date bucketing.

## Test coverage

`personal-cfo-rp1r` exercises `America/Los_Angeles`, `Europe/London`,
`Asia/Kolkata` (UTC+5:30), and `Australia/Lord_Howe` (30-minute DST). The
`forecast-engine` crate already carries zone tests for start-date resolution and
the DST invariant against those zones.

## Linked beads

- `personal-cfo-z3kx` (this ADR)
- `personal-cfo-rr0` (implementation: date + timezone policy)
- `personal-cfo-rp1r` (cross-cutting IANA tz + DST test suite)
- `personal-cfo-164u` (forecast engine — implements §3 / §4 / §7)
- `personal-cfo-82q9` (pay-schedule — implements §2)
- `personal-cfo-v53f` (cross-cutting test: household timezone change)
- `personal-cfo-me79` (per-locale number/date display formatting)
- `personal-cfo-2lm` (vault_metadata schema — `household_timezone`)
- `personal-cfo-5ie.11` / `personal-cfo-ku2hn` (2026-09-08 addendum below)
- `personal-cfo-m8x2r` (follow-up: remaining non-compliant call sites)
- `personal-cfo-q329` (2026-09-08 addendum: the setter + initial-default decisions)

## Addendum (2026-09-08): drift found and fixed — the past-due queue and the confirm-early guard were reading raw UTC (`personal-cfo-5ie.11`, `personal-cfo-ku2hn`)

§1 already settled this — "never UTC, never the machine's local zone" — but several call
sites written before `read_household_tz` existed (or added since without following it)
never got updated to match. The user-visible symptom: after roughly 5pm Pacific, a bill due
*today* already read as past due on Cash Flow, because `DbWorker::unconfirmed_past_due` (and
its siblings sharing the same `today`) computed it as `Utc::now().date_naive()` — the raw
UTC calendar date — rather than resolving it through the household timezone. The
`ConfirmObligationEarly` future-date guard (`apply/transactions.rs`) had the identical bug on
its `today` comparison, which mattered specifically because the frontend's paid-date input
(`MarkObligationPaid.tsx`'s `todayIso()`) has no household-timezone concept at all (there is
no frontend accessor for it — see `ScenariosView.tsx`'s `today()`, which documents the same
gap for its own display-only badge) and was *also* computing UTC (`toISOString()`) despite a
doc comment claiming "household-local."

**The decision this addendum records, since neither symptom is itself a new architectural
question — §1 already answered it — is the *scope* of the fix and where the impure/pure
split lives:**

1. **Kernel side (`crates/db-worker/src/forecast/events.rs`):** `read_household_tz` gained two
   siblings — `household_today(conn)` (impure: reads `Utc::now()`, matching the debt.rs /
   `lib.rs:2823` sites that were already correct) and `household_today_at(conn, as_of)` (pure:
   resolves an explicit instant, mirroring `Horizon`'s `as_of` — ADR §3/§7's pattern applied
   one level further so the resolution itself is testable against a fixed instant, not only
   real wall-clock). Fixed to use one of these: `unconfirmed_past_due`,
   `rebuild_recurring_instances`, `recurring_bill_history`, `actualize_forecasts`,
   `backtest_forecasts` (all in `lib.rs`, all part of the past-due/actualization chain
   5ie.11's bug report named), and `apply_confirm_obligation_early`'s future-date guard
   (`apply/transactions.rs`, load-bearing for ku2hn: it is what the frontend's `max` input
   guard needs to agree with).
2. **Frontend side (`MarkObligationPaid.tsx`):** `todayIso()` now uses local `Date` getters
   (`getFullYear`/`getMonth`/`getDate`), matching the already-established convention in
   `FutureCashEntries.tsx`, `ScenarioEvents.tsx`, and `useSpendByCategory.ts` — browser-local,
   not UTC, and not attempting household-timezone resolution client-side (no accessor exists
   for it yet). The doc comment states this caveat honestly, the same way
   `ScenariosView.tsx`'s already does, instead of the false "household-local" claim it had.
3. **Deliberately out of scope, tracked separately (`personal-cfo-m8x2r`):** the same
   `Utc::now().date_naive()` bug also exists in `recurring_candidates` / `income_candidates`
   (recurring-detection suggestions), `money_inbox_list`'s snooze filter, and the "next
   due/pay date" projections in `read_income_source_views` / `read_recurring_bill_views` /
   `read_recurring_transfer_views` — the latter's doc comments literally say "UTC calendar,"
   an honest label for a bug rather than a policy exception. None of these were named by
   either bead's acceptance criteria (they surface a bill as due *tomorrow* one day early,
   not *past due*, a materially smaller symptom), and fixing them is mechanically identical
   to what this addendum already did five times over — deferred to avoid inflating one bug-fix
   PR into a repo-wide sweep, not because the fix differs.

No new architectural decision was made here — this addendum documents where the
already-accepted §1 policy was not yet followed and records the fix, per `AGENTS.md` §1A /
§12.

## Addendum (2026-09-08): the household timezone becomes settable — the setter, and the initial-default policy (`personal-cfo-q329`)

The addendum above fixed the KERNEL side of the launch-evening symptom: `household_today()`
correctly resolves through `vault_metadata.household_timezone`. But that column was written
only once, at vault creation, hardcoded to `DEFAULT_HOUSEHOLD_TIMEZONE = "UTC"` — there was
no setter anywhere (no `UPDATE` outside tests, no IPC, no Settings UI). So every vault that
existed was permanently pinned to UTC, and 5ie.11's fix held only in theory until a
non-UTC household could actually configure itself. This addendum closes that gap and
records the three decisions the owner made to do it (2026-09-08), settling questions this
ADR's original text left open rather than making a new architectural call.

**A. Existing vaults get no migration prompt and no automatic change.** The Settings
"Household" timezone picker introduced by this bead (`HouseholdCard.tsx`) *is* the migration
path — a household on UTC simply opens Settings and sets its real zone once, same as it
would set any other preference. No first-launch-after-upgrade prompt, no silent
auto-detection overwrite of a value someone might have already set deliberately.

**B. New vaults capture the machine's IANA zone as the INITIAL default, not `UTC`.** §1's
"never the machine's local zone" governs the *authoritative* value — what every calendar
boundary resolves against for the vault's whole lifetime — and that rule is unchanged: the
authoritative value is always `vault_metadata.household_timezone`, read fresh on every
resolution, never re-derived from the machine. Capturing the machine's zone *once*, at the
impure vault-creation boundary (`create_vault_impl` / `create_vault_named_impl` in
`apps/desktop/src-tauri/src/ipc/commands.rs`, both call the same
`set_machine_timezone_on_create` helper via `iana_time_zone::get_timezone()`), to seed that
column's *first* value is a different thing — a one-time best-effort convenience so a fresh
user never sees the launch-evening symptom on day one, not a standing exception to §1. A
failed or unavailable capture falls back to the `UTC` default silently (logged, never fails
vault creation) — fixable any time from Settings, same as decision A's migration path.

**C. The picker lives in a new, dedicated Settings card — `HouseholdCard.tsx`, first among
the cards** (`apps/desktop/src/settings/SettingsView.tsx`) — not folded into an existing
Settings PR (`personal-cfo-7oj4`'s currency/locale section, `personal-cfo-gkjy`'s
privacy-mode surface) to avoid two in-flight PRs building two Settings surfaces. It holds
only the timezone today and is the intended anchor both of those extend later, along with
the owner's household-members roadmap idea (schema under `personal-cfo-339`) — this bead
does not build any of that, only leaves the anchor in place.

**Mechanics**, for completeness (the "which mechanism" question was already §1's, not new):
`crates/db-worker/src/forecast/events.rs::write_household_tz` validates the IANA name via
`chrono_tz::Tz::from_str` — the same parser `read_household_tz` already used on read —
*before* writing, since every downstream reader (the whole Future Cash forecast surface,
transaction posting) fails loudly on a bad stored value; an invalid name never reaches the
column. `DbWorker::set_household_timezone` / `Kernel::set_household_timezone` are a plain
`UPDATE vault_metadata SET household_timezone = ?1 WHERE singleton = 1` against the
single-writer connection — configuration on the singleton row, not a ledger mutation, so
(like `set_setting`) it bypasses `WriteCommand`/the operation log rather than inventing one.
The IPC pair (`household_timezone` / `set_household_timezone`) follows the
`set_comfort_band_upper` precedent exactly: an `*_impl` free function plus a thin
`#[tauri::command]` wrapper, registered in `app-commands.toml`'s non-destructive grant list.

Also carried in this bead, closing a gap the first addendum named but didn't fix: the
frontend gained its first accessor for the household timezone
(`apps/desktop/src/settings/useHouseholdTimezone.ts`), and `MarkObligationPaid.tsx` /
`ScenariosView.tsx` — both of which had documented "there is no frontend accessor for the
household timezone today" caveats — now use it instead of the browser's local date.

`personal-cfo-m8x2r` (the remaining `Utc::now().date_naive()` call sites named in the first
addendum) is unaffected by this bead and remains open.

## Addendum (2026-09-09): the deferred call sites fixed — `personal-cfo-m8x2r`

The first addendum's item 3 deliberately deferred six sites carrying the same
`Utc::now().date_naive()` bug (a materially smaller symptom — a bill or occurrence
one day early, not falsely past-due) rather than inflate `5ie.11`'s PR into a
repo-wide sweep. This bead closes that gap. No new architectural decision — §1
already settled it; this records where it was applied and how it was tested.

**Fixed, all in `crates/db-worker/src/lib.rs`, all via the existing
`forecast::household_today(conn)`:**

1. `money_inbox_list`'s snooze-expiry filter (`snoozed_until <= today`).
2. `recurring_candidates` / `income_candidates` — the connection is now hoisted to
   a `let conn` binding so `household_today(&conn)` and the detector call can both
   borrow it; the detected candidate *set* was never `today`-sensitive (see below),
   only the `next_date` field projected onto each candidate.
3. `read_income_source_views`, `read_recurring_bill_views`,
   `read_recurring_transfer_views` — the "next pay/due/occurrence date"
   projections. The first two's doc comments said "UTC calendar" outright; both
   corrected to say household-local.

**Deliberately left alone:** the connector-sync balance clamp (`lib.rs`, inside
the ingestion path) already resolves through `read_household_tz` on its primary
path, falling back to raw UTC only if that read itself fails — matching §1's
intent already, not an instance of this bug. Changing the fallback branch to also
fail rather than degrade would be a behavior change with no bug behind it, so it
was left as-is.

**Testing decision — timezone selection, not an as-of seam.** All six call sites
are impure entry points with no `as_of` parameter (`household_today` resolves
`Utc::now()` internally), unlike `household_today_at`, which `5ie.11` built and
pinned to a fixed instant precisely so its *own* resolution logic could be
unit-tested deterministically. Threading an `as_of` seam through all six would
have been the more "by the book" match to §3/§7's as-of-instant discipline, but
it widens the diff considerably (new parameters on every call site, an unblocked
`#[cfg(test)]` re-export) for behavior none of the six actually need pinned —
none of them are part of the deterministic forecast fold §3 governs. Instead,
`crates/db-worker/tests/household_local_today_remaining_sites.rs` drives the
*household timezone*: `Pacific/Niue` (UTC−11) and `Pacific/Kiritimati` (UTC+14)
between them guarantee a local date that disagrees with UTC's at any instant the
test happens to run (their local-midnight thresholds, UTC 11:00 and UTC 10:00
respectively, jointly cover all 24 hours with no gap — see the helper's own doc
comment), which is exactly the discipline the AC asked for ("proves the household
timezone is actually consulted, not just that the code compiles") without
widening the six sites' own signatures. Each of the six behavioral tests was
verified to fail against the pre-fix code (`git stash` the `lib.rs` change, rerun)
before being accepted, so none of them can be vacuous.

One further finding while investigating sites 5–6: `categorization::detect_recurring`
uses `today` in exactly one place — `roll_forward`, which advances a stale
`next_date` projection forward by whole cadence intervals when it has fallen
behind `today`. The candidate *set itself* (which merchants surface, at what
confidence) never depended on `today` at all — only `next_date` does. The two new
tests target that field directly (a three-observation biweekly series positioned
so the naive projection lands exactly on the earlier of the two candidate
"todays," making the household-local vs. UTC choice flip the result by a full
14-day cadence step) rather than asserting on set membership, which would not
have regressed if the fix were reverted.

Linked: `personal-cfo-m8x2r`.
