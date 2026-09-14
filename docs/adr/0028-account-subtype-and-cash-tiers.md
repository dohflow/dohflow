# ADR 0028: Account subtypes and type-based cash-tier rollups

- **Status:** Accepted
- **Date:** 2026-06-23
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-9dgg`](../../.beads/issues.jsonl)
- **Related plan sections:** §9.3 (account model), §18 (Future Cash)
- **Extends:** ADR 0007 (account/ledger model — adds an optional classification axis), ADR 0027 (additive balance — tiers sum assertion-anchored balances)

## Context

Dogfooding round 2 (feedback A7) asked for cash to be grouped into tiers — a
"money I can spend now" rollup distinct from "money set aside" — feeding the
per-account/per-group forecast (`personal-cfo-l8oh`), the spreadsheet table
(`ygjs`), the multi-series chart (`l916`), and a meaningful **net cash** total.

The blocker: the account model classifies accounts only by [`CashflowRole`]
(`LiquidCash`, `CreditFacility`, …). `LiquidCash` deliberately lumps **checking
and savings together** ("spendable now" — see `core-ledger/src/account.rs`), so the
schema cannot today tell a checking account from a savings account. There is no
finer axis: `AccountFlags` are orthogonal booleans (retirement, joint, …), not a
mutually-exclusive subtype.

We considered approximating tiers from existing roles (e.g. treat `InvestmentAsset`
as the "secondary" tier), but that conflates savings with investments and misleads
the net-cash total. The project owner chose to **add the missing axis properly** now
rather than approximate.

## Decision

### 1. Add an optional `AccountSubtype` classification axis

A new **optional** subtype refines an account within its role. It is nullable: an
account without a matching subtype (or in a role we do not subtype) simply has
`NULL`, and everything still works. Each subtype belongs to exactly one
`CashflowRole`; setting a subtype whose role does not match the account is rejected
in the kernel (domain validation, not a column constraint).

The taxonomy (storage tokens), by role:

| `CashflowRole`     | Subtypes                                          |
| ------------------ | ------------------------------------------------- |
| `LiquidCash`       | `checking`, `savings`, `money_market`, `cash`     |
| `CreditFacility`   | `credit_card`, `line_of_credit`                   |
| `LoanLiability`    | `mortgage`, `auto_loan`, `student_loan`           |
| `InvestmentAsset`  | `brokerage`, `retirement`                         |
| `RealAsset`, `ExternalClearing`, `IncomeExpenseVirtual` | *(none — always `NULL`)*     |

The full taxonomy ships in one pass (the project owner's choice) even though only
the liquid subtypes drive cash tiers today. The credit/loan/investment subtypes are
**foundation**: they unblock later debt grouping (`personal-cfo-xuer`) and
investment views without a second migration. The set is intentionally small;
real-world variants we do not list (a personal loan, an HSA) are represented by
`NULL` rather than forced into a bucket — `NULL` is a first-class "unspecified".

### 2. Cash tiers are derived from liquid subtypes (spendable vs reserve)

The cash-tier rollup partitions **`LiquidCash` accounts only**, by subtype, using
each account's **assertion-anchored balance** (ADR 0027 — not a raw posting sum):

- **Spendable** = `checking` + `cash` **+ any `LiquidCash` account with no subtype.**
- **Reserve** = `savings` + `money_market`.
- **Net cash** = Spendable + Reserve = every `LiquidCash` account.

Unclassified liquid (`NULL` subtype) folds into **Spendable** — the conservative
"treat accessible cash as spendable" default — which preserves the invariant
**`net cash = spendable + reserve`** (the bead's "net cash = configured tiers"
acceptance criterion). Non-liquid roles never contribute to cash tiers.

These rollups need **no user setup**: they derive from account type + subtype out of
the box. Custom membership that overrides the type-based default is a separate,
advanced layer (`personal-cfo-uy82`), explicitly out of scope here.

### 3. The rollup is a read-time computation, not stored state

`cash_tiers` is computed on read from the accounts + their assertion-anchored
balances (mirroring how the forecast's starting balance is derived). Nothing is
persisted beyond the per-account `subtype`; there is no tier table to keep in sync.
This is the foundation `l8oh` consumes for per-group projection.

## Consequences

- **Migration v14** adds a nullable `accounts.subtype TEXT` column with a `CHECK`
  constraining it to the token set above (NULL passes). The column `CHECK` cannot
  reference `cashflow_role` (a SQLite `ALTER … ADD COLUMN` limitation), so the
  **role↔subtype match is validated in the kernel**. Down-migration drops the
  column (SQLite ≥ 3.35, pinned at 3.45.3 per ADR/`7igv`).
- `core-ledger` gains an `AccountSubtype` enum (token ⇄ value, owning role, and the
  liquid `cash_tier()` mapping). `Account` gains an optional `subtype` field with a
  `#[serde(default)]` so existing serialized commands/oplog entries deserialize
  unchanged.
- `Account::new` keeps its signature (subtype defaults to `None`); a `with_subtype`
  builder sets it. This keeps the change additive across the many call sites.
- The IPC `AccountViewDto` / create / update inputs gain `subtype`, and a
  `cash_tiers` read command is added (PR B); the frontend add-account form offers a
  role-dependent subtype, and the UI surfaces spendable / reserve / net cash.
- No FX: cash tiers are single-currency like the forecast (`personal-cfo-4n3x`); a
  multi-currency rollup is deferred with the rest of multi-currency (`ipui`).

## Alternatives considered

- **Approximate tiers from existing roles** (savings ≈ `InvestmentAsset`): rejected
  — conflates savings with investments and corrupts net cash.
- **Boolean flags instead of a subtype enum** (`is_savings`): rejected — subtype is
  mutually exclusive within a role; a closed enum + `CHECK` is the honest shape and
  extends to credit/loan/investment cleanly.
- **Liquid-only taxonomy now**: viable and smaller, but the owner chose the full
  taxonomy to avoid a second migration when debt/investment grouping lands.

## Addendum (2026-07-11, personal-cfo-4d8.25.22): HSA + crypto investment subtypes

Owner dogfooding 2026-07-09: "We need other investment account types outside of retirement
and brokerage… HSA accounts which can be invested in stocks… crypto currency investments
like bitcoin, ethereum" — primarily so recurring transfers can target them (e.g. a monthly
HSA contribution) and they read correctly in Accounts.

**Decision.** Add two subtypes under the `InvestmentAsset` role: `hsa` and `crypto`.

- This **reverses** this ADR's original note (§ "roles and subtypes") that an HSA is
  represented by a `NULL` subtype. That was fine when the only distinction was liquid-vs-not;
  now the owner wants HSA and crypto as first-class, selectable investment kinds, so a `NULL`
  subtype is no longer expressive enough. `AccountFlags::tax_advantaged` remains an orthogonal
  boolean (an HSA is tax-advantaged *and* subtype `hsa`); the subtype names the account kind.
- **Cash tiers:** neither is liquid, so `cash_tier()` stays `None` — no effect on
  Spendable/Reserve/Net (they are investment assets, outside the liquid line).
- **Display:** both read as an investment **"Balance"** with an asset (positive) sign — the
  ADR 0044 value-vs-balance "Value" label stays reserved for `real_asset`. Manual updates use
  the existing balance-assertion path (ADR 0027); `BatchBalanceUpdate` already includes
  `investment_asset`.
- **Recurring transfers** into them already work (the DCA path verified in
  personal-cfo-4d8.22.8) — a new investment subtype is automatically a valid transfer
  destination; no transfer-engine change.
- **Explicitly out of scope (v1):** individual asset holdings, tickers, price refresh, and
  crypto volatility handling — those remain the long-term "Investments and net worth" epic
  (personal-cfo-9h0). Crypto gets **no** special valuation logic in v1; it is a
  manually-valued investment balance like any other.

**Migration.** The `accounts.subtype` `CHECK` is widened by the v36 column-swap pattern
(migration v43): SQLite can't ALTER a column CHECK, so a new column with the superset CHECK
is added, copied, and renamed. Every existing token is in the new superset, so no row is
rejected. Subtype tokens cross the IPC boundary as plain strings, so no bindings/enum change.
