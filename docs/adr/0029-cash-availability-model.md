# ADR 0029: Cash availability model

- **Status:** Accepted
- **Date:** 2026-06-24
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-fqbm`](../../.beads/issues.jsonl)
- **Related plan sections:** §13.2.1 (cash availability)
- **Builds on:** ADR 0027 (additive balance — the ledger number), ADR 0026 §12 (per-account projection — the committed number), ADR 0028 (liquid accounts)

## Context

Future Cash must not begin from a single ambiguous balance (§13.2.1). A raw
balance invites the classic errors: treating a credit-card purchase as immediate
cash out, treating a pending deposit as guaranteed cash, double-counting autopay.
The engine should retain the *components* even if the UI shows one number.

The `fqbm` bead predates the additive-balance and per-account work and its
acceptance criteria were imprecise (it named five numbers but its "floor"
definition and the `ledger ≥ available ≥ floor` property did not line up). This ADR
pins the formulas, ratified with the project owner.

## Decision

### 1. Five derived numbers per liquid account

Computed on read for each `LiquidCash` account, at an instant `as_of`:

| Number        | Definition                                                              |
| ------------- | ----------------------------------------------------------------------- |
| **ledger**    | The canonical balance — the assertion-anchored balance (ADR 0027).      |
| **pending**   | Sum of uncleared holds against the account (≥ 0).                       |
| **available** | `ledger − pending` — what is actually accessible right now.             |
| **committed** | Sum of projected **outflows** over the next **30 days** attributed to the account (from the ADR 0026 §12 per-account projection). |
| **headroom**  | `available − committed` — what is free after the next 30 days of bills. |

`pending`, `committed` ≥ 0, so the invariant **`ledger ≥ available ≥ headroom`**
holds for every account; it is asserted as a property test. (The bead's per-account
"floor" is this `headroom`.)

### 2. Manual mode: pending is first-class but zero

Manual entry produces **no** pending postings, so `pending = 0` and
`available = ledger`. `pending` / `available` remain first-class fields regardless,
so a future connector that delivers real holds (`personal-cfo-iwxc`) populates them
**without a model change** — only the data source differs.

### 3. The minimum-cash floor is a single household setting (ratified)

The floor — the buffer the user wants to keep — is **one household-wide setting**
(`minimum_cash_floor_minor`, default `0`), not a per-account value. The forward-looking
**below-floor** status fires when **net headroom** drops below the floor:

> `net_available − net_committed < floor`,

where `net_available = Σ available` and `net_committed` sums every account's committed
outflows **plus** un-attributable outflows (manual entries / non-liquid-paid bills —
the ADR 0026 §12 "Unallocated" series). So a manual one-off outflow still erodes the
household safe-to-spend even though it belongs to no single account.

### 4. Ratified parameters (2026-06-24)

- **Committed window = 30 days** (matches the dashboard's near-term window;
  predictable and always computable). A configurable window is a later option.
- **Floor scope = household-wide** (one buffer for total liquid cash).

### 5. Derived, single-currency, nothing persisted

The whole model is computed on read from canonical state (balances + the
projection); only the floor *setting* is persisted (the `settings` table, like
`reporting_currency`). Single-currency like the rest of the forecast
(`personal-cfo-4n3x`); a multi-currency rollup is deferred with `ipui`.

## Consequences

- A `CashAvailability` read in `db-worker` reuses `compute_by_account` (30-day
  horizon) for `committed`, and `assertion_anchored_balance` for `ledger`; exposed
  via a `cash_availability` IPC command. The floor is read/written through the
  existing settings commands.
- The UI shows a household "safe to spend" summary (available − committed) and the
  below-floor status, plus a floor input in Settings; per-account detail can follow.
- `pending` is wired as `0` today; the cross-cutting pending-deposit-with-holds test
  (`personal-cfo-iwxc`) lands when connectors deliver holds.

## Alternatives considered

- **Per-account floor** — rejected: an unusual mental model and more setup; the
  per-account property is still expressed via `headroom`.
- **Committed = full forecast horizon** — rejected: makes "committed" enormous for a
  6–12 month horizon, so headroom reads negative almost always.
- **Start the forecast from `available` instead of `ledger`** — rejected: the
  forecast stays anchored on the canonical ledger (ADR 0027); availability is a
  *lens* over it, not a replacement.
