# ADR 0027: Additive balance model — assertions and progressive reconciliation

- **Status:** Accepted
- **Date:** 2026-06-23
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-mkq1`](../../.beads/issues.jsonl)
- **Related plan sections:** §9.3, §13.2.1
- **Amends:** ADR 0007 (ledger/posting model — balance derivation)
- **Supersedes:** the reconciliation-v0 "drift → Money Inbox" behaviour described in `personal-cfo-xmc`

## Context

Dogfooding round 2 surfaced a core-model problem (feedback A5): the app forces
transaction-tracking. To change an account balance today you must enter
reconciling transactions until the delta is zero (the `4d8.5` modal), because the
ledger is the only source of a balance (ADR 0007: balance = sum of postings; the
DB-side balance trigger enforces it). Many households won't track every
transaction in a manual-entry tool — they want to **set a balance, then see Future
Cash react**, recording detail only when they choose to.

The "manual mode must work" principle (§1.5) says this should be first-class, not
an afterthought. And the infrastructure to express it already half-exists:

- **`balance_observations`** (`personal-cfo-xmc`, §9.3): "evidence (balance_type:
  ledger/available/statement/connector/**manual**), does NOT mutate the ledger;
  reconciliation v0 compares the latest observation against the ledger-derived
  balance and surfaces drift in the Money Inbox."
- **Cash availability model** (`personal-cfo-fqbm`, §13.2.1): per-account
  ledger / available / pending / committed / floor.

A **manual** balance_observation *is* a user-stated balance, and the
**observation-vs-ledger drift** *is* the "unexplained" amount we want to track. So
this is not new infrastructure — it is a reinterpretation of an existing design:
turn the drift from a Money-Inbox nag into a balance the user owns and that
explains itself over time.

## Decision

### 1. Balance assertions are the user primitive

A user states **"account = B as of date D"** with no transaction. An assertion is
persisted as a `balance_observations` row with `source = manual`. The opening
balance at account creation is simply the **first assertion**. Setting a balance is
frictionless — no modal, no reconciling entries.

### 2. The balance is anchored to the latest assertion (amends ADR 0007)

An account's current balance is:

> **latest assertion `B` (at date `D`)  +  Σ real postings dated strictly after `D`.**

The assertion is a **checkpoint**; the double-entry ledger continues from it. This
**amends ADR 0007**: a balance is no longer purely the sum of all postings — the
most recent assertion overrides history before it. Postings that exist remain
rigorous double-entry; they are simply not *required* to fully reconstruct the
balance.

### 3. The auto-reconciling adjustment (the "plug") is **derived, not a stored posting**

For the latest assertion, the **unexplained amount** is:

> **`B` − ( prior assertion's balance  +  Σ real postings in `(prior_D, D]` ).**

It is **computed on read** and surfaced as *"unexplained since your last balance
update,"* never written as a ledger posting. It **auto-shrinks toward zero** as
real postings in the window are added or imported — that is the *progressive
reconciliation*. Example: assert $3,000 on Mar 5, assert $5,500 on Mar 20 →
unexplained $2,500; later import a card statement whose lines net to +$2,500 in
that window → the derived unexplained recomputes to $0 with no extra step.

A **derived** plug (rather than a stored adjustment posting to a suspense account)
is chosen deliberately: it avoids write amplification (no re-posting on every
imported transaction), keeps the ledger free of synthetic entries, and makes the
"unexplained" a single honest read-time quantity.

### 4. Progressive reconciliation and manual resolution

Adding or importing real postings inside an assertion's window shrinks the derived
unexplained automatically. A residual can be **converted or split** into real
transactions by the user — each conversion is an ordinary posting that further
reduces the unexplained. Nothing is ever forced; "unexplained → explained" happens
on the user's schedule.

### 5. Transactions stay optional; the ledger stays rigorous

Recorded transactions (manual entries, transfers, imports) remain **double-entry
(ADR 0007)** and accrue on top of the latest assertion. You never have to record
them. When you do, they explain the plug and sharpen the per-account history.

### 6. Future Cash starts from the asserted balance

The forecast's starting balance per liquid account / cash rollup (`personal-cfo-9dgg`)
is the assertion-anchored balance (§2). Manual mode therefore produces a full
forecast with **zero transactions** recorded.

### 7. Relationship to the cash-availability model and account close-out

The asserted balance feeds the `ledger` / `available` components of the cash
availability model (`fqbm`); `committed` and `floor` continue to come from the
forecast. Archiving an account that still holds a balance prompts a **disposition**
— transfer to/from another account, keep as cash, or write off — so net cash stays
correct (detailed in `personal-cfo-ynkr`).

### 8. Progressive reconciliation (addendum 2026-06-27, `dyy4`)

The unexplained plug is a **derived** value (§3: `asserted − (prior assertion + postings
dated in the window)`), so it **auto-shrinks** with no extra machinery: every real posting
that lands in the assertion window — a manual entry, a transfer leg, an imported statement
row — raises the explained sum, so the next read of the plug is smaller. Nothing is stored
or recomputed eagerly; the shrink is a property of the formula.

What remains is **converting the residual into a real transaction** when the user knows
what it was. **`ConvertUnexplainedToTransaction`** is an op-logged kernel command that reads
the account's current plug and the latest assertion's `observed_at`, then records one
balanced ledger transaction (account posting + system income/expense counter, routed by the
plug's sign) **dated at that assertion date**. Because the posting lands *inside* the
window (`posting_date <= observed_at`), it raises the explained sum by exactly the residual
— the plug goes to zero, and the transaction is now a first-class ledger entry the forecast
and history see. A no-op when the plug is already zero.

This is the one-click case; the existing `ReconcileBalanceModal` (4d8.5) remains the
**split** path — record several reconciling transactions, watching the remaining delta count
down — for when the residual is actually several forgotten postings. Convert keeps the
ledger honest without forcing the user to itemize.

### Addendum (2026-09-02): connector-observed balances join the model (`personal-cfo-yl53`)

Bank sync (ADR 0060) delivers a provider-stated balance with every walk. These
now enter `balance_observations` at sync time — the commit path the importer
arc deferred:

- **Write.** `stage_sync_batch` promotes each staged balance for a *mapped*
  account inside the same staging transaction: any prior `connector_sync`
  observation for the same `(account_id, observed_at)` is replaced
  (last-wins per day), the row lands with `source = 'connector_sync'`,
  provenance via `source_record_id`, and `observed_at` date-only (the same
  `YYYY-MM-DD` convention as manual assertions). The staged row is then
  consumed. Unmapped balances stay staged, exactly as before. Balance
  promotion is independent of per-transaction commit outcomes: the provider's
  balance is true regardless of how its transactions triage.
- **Read.** The anchor queries — `assertion_anchored_balance`, the derived
  plug (`unexplained_adjustment`), the batched forecast anchor, and the
  `stale_balance` inbox generator — widen from `source = 'manual'` to
  `source IN ('manual', 'connector_sync')`. The latest observation across
  both sources anchors the displayed balance; the plug is computed against
  that anchor and keeps auto-shrinking as postings explain it; an account a
  connector refreshes never goes stale.
- **Baseline rule.** A FIRST-ever observation that arrived from a connector
  yields **no plug** (`unexplained_adjustment` returns `None`): it is the
  provider's starting truth, not a user-visible discrepancy — otherwise every
  freshly-linked account would carry a plug equal to its pre-history balance,
  tanking Forecast Readiness with an ask no user action can satisfy. A first
  *manual* assertion keeps its existing plug semantics (the user explicitly
  asserted against their own entered history).
- **Cross-day windows.** The plug's *prior* observation is the newest one on
  a strictly **earlier day**. Two observations on the same day are a
  last-wins pair, not a window: postings are date-granular, so a same-day
  window is degenerate and nothing could ever explain it. The
  convert-to-transaction correction is dated at the latest observation
  (either source) so it always lands inside the window it zeroes.
- **Currency guard.** A provider balance in a different currency than the
  mapped account is **not** promoted (it stays staged) — parity with
  `record_balance_assertion` and the transaction commit path.
- **Local-day clamp.** The provider's epoch collapses to a UTC date upstream;
  an evening sync west of UTC would land "tomorrow". Promotion clamps
  `observed_at` to the household-local today, so a manual assertion recorded
  later the same local evening still wins its tie on `created_at`.
- **Same-day tie.** `ORDER BY observed_at DESC, created_at DESC` already
  resolves a manual assertion and a sync on the same day: the most recently
  *recorded* wins. A user overriding right after a sync sees their number; the
  next sync re-observes. This is deliberate — for a linked account the
  provider is authoritative at rest, the user is authoritative in the moment.
- **Deferred, with rationale.** The file-import path (an OFX `LEDGERBAL`)
  still stages without committing: imports flow through review gates and a
  silently committed balance from a months-old export would *regress* the
  anchor semantics (an old observation with a newer `created_at` still loses
  to nothing if it is the only one). Import-time balance commit needs its own
  decision about age cutoffs; tracked on the bead.

## Consequences

### Positive

- Manual mode is first-class: set a balance, get a forecast, record detail only by
  choice. Directly answers the A5 feedback.
- Reconciliation becomes something that happens *for free* as data is added, not a
  chore — and it surfaces inline, not as a Money-Inbox backlog.
- No parallel infrastructure: assertions reuse `balance_observations`; the
  unexplained reuses the existing drift concept; the ledger and `fqbm` are unchanged.
- The double-entry ledger stays intact and authoritative for the transactions that
  exist (transfers, imports, splits) — additive, not a teardown.

### Negative

- Balance derivation is no longer the pure ADR-0007 posting sum; assertions are
  checkpoints. This is a deliberate, documented amendment and the account
  read-model + balance trigger must implement it (`personal-cfo-ueg6`).
- The derived "unexplained" must be computed and presented clearly, or users won't
  trust the number. Its definition (this ADR) is the single source of truth.

## Rejected alternatives

- **Pure double-entry, reconcile-as-you-go** (the `4d8.5` reconcile-via-transactions
  modal). ✗ This *is* the friction A5 rejects — you must enter transactions to set
  a balance. Kept only as an optional power-user path.
- **A plain editable balance field, ledger dropped.** ✗ Abandons double-entry
  guarantees for the transactions that do exist and discards the import /
  reconciliation story entirely.
- **A stored adjustment posting to a suspense account, recomputed on change.** ✗
  Write amplification (re-post on every imported line), clutters the ledger with
  synthetic entries, and needs careful ordering — the derived plug is cleaner and
  equally correct.

## Revisit if

- Per-account assertions in currencies different from the account/base currency are
  needed — that requires conversion (see `personal-cfo-ipui`, deferred).
- An audit or regulatory requirement demands a fully posting-derived balance with
  no checkpoints — then a materialized adjustment posting per closed period would
  be reintroduced for the closed range.

## Linked beads

- `personal-cfo-mkq1` (this ADR)
- `personal-cfo-ueg6` (backend: balance assertions + the derived adjustment; depends on `xmc`)
- `personal-cfo-hxjj` (set-balance UI)
- `personal-cfo-dyy4` (progressive reconciliation: shrink + convert the plug)
- `personal-cfo-ynkr` (account close-out / archive disposition)
- `personal-cfo-xmc` (Schema: balance_observations — the assertion store)
- `personal-cfo-fqbm` (Cash availability model)
- ADR 0007 (ledger/posting model — amended here)
