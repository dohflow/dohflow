# ADR 0039: Credit-card cycle and statement-balance forecast (incl. bill-charged-to-card)

- **Status:** Accepted (ratified 2026-07-10, personal-cfo-4d8.25.1 — the cycle/statement model
  shipped as decided through the 2026-07-01/2026-07-06 addenda; the 2026-07-10 addendum below
  records the statement-accuracy contract the ratification is conditional on)
- **Date:** 2026-06-30
- **Deciders:** Project owner
- **Beads:** [`personal-cfo-6wk.7`](../../.beads/issues.jsonl) (this ADR); specifies/reconciles
  `xcq` (cycle + statement schema), `kqez` (cycle model), `4lhm` (statement/payment forecast),
  `llx5` (interest), `6wk.4` (payment outflow); introduces `6wk.8` (recurring bill
  charged-to-card). Consumes `pezm.1` (ordinary/extraordinary classifier) and `6wk.6`
  (`debt_terms` attributes).
- **Builds on:** ADR 0035 (debt-payment model — repayment philosophy §1, asymmetric transfer
  legs §3, finance charges §4, stored debt attributes §5), ADR 0026 (forecast architecture —
  §12 per-account projection, §14 transfers), ADR 0038 (ordinary/extraordinary spend
  classifier), ADR 0028 (account subtype + cashflow role), ADR 0018 (non-advice boundary).

## Context

ADR 0035 fixed the debt-**payment** model but deliberately deferred the cycle math: §4's
compounding loop is written as `next_opening = opening + new_charges + finance_charge −
payment`, and it explicitly leaves *"which base each token selects, not the cycle math (that
is `kqez`/`4lhm`/`40f`)"*. Two things are still undecided:

1. **Where `new_charges` comes from.** The per-cycle charges that build a statement balance
   are unspecified. They are partly *deterministic* (known recurring obligations the user
   puts on the card) and partly *projected* (ordinary variable card spend, now classifiable
   via ADR 0038 / `pezm.1`).
2. **Bills paid by credit card.** Today `collect_bill_events`
   ([`crates/db-worker/src/forecast.rs`](../../crates/db-worker/src/forecast.rs)) emits **every**
   recurring bill as a **liquid-cash outflow on its due date**, regardless of how it is paid.
   So a bill the user pays with a credit card wrongly depletes liquid cash on the bill date
   **and never lands on the card** — the opposite of reality, where the cash leaves later when
   the card statement is paid.

The project owner's framing names the fix exactly: *"recurring bills we set up can be marked
as being paid from credit cards, so we know every time on X date, Y amount will be getting
added to the CC balance."* That is the deterministic backbone of `new_charges`. This ADR
specifies the credit-card cycle + statement forecast and resolves the bill-charged-to-card
gap, **before** the cycle schema/feature is built (AGENTS.md §1A). Per ADR 0018 everything
here is descriptive — it states what follows from the user's own settings, never advises a
repayment or spending choice.

## Decision

### 1. Cycle boundaries are derived from `statement_close_day`

A card's billing cycle is delimited by its `statement_close_day` (stored in `debt_terms`,
`6wk.6` / ADR 0035 §5), with the documented **month-end clamp** for short months. Cycle *N*
covers `(close_{N−1}, close_N]`; the **statement** posts on `close_N`; the **payment** is due
on `payment_due_day` of the following period (clamp + `grace_period_days`). Cycles are derived
**deterministically** over the forecast horizon — there is no per-cycle user input, mirroring
how `PaySchedule` expands income/bill occurrences.

### 2. The projected statement balance composes three parts

For each projected cycle:

```
statement_balance = carried_balance + new_charges(cycle) + finance_charge(cycle)
```

- **`carried_balance`** = the prior cycle's `next_opening` (ADR 0035 §4 carry); `0` when the
  prior cycle was paid in full.
- **`new_charges(cycle)`** = the charges whose date falls in the cycle window:
  - **known recurring bills charged to the card** (§3) — *deterministic*;
  - **projected ordinary variable card spend** — the Layer-2 baseline (`9h1s`) over the
    **ordinary-classified** spend (ADR 0038 / `pezm.1`) attributed to the card; extraordinary
    one-offs go through the known-event / `l3id` path, never a naive rolling average;
  - **known one-offs** charged to the card (manual future entries `q6gh`).

  To avoid double-counting, the **projected ordinary variable card spend excludes the
  deterministic card-charged bills** for the same card — mirroring how
  `read_variable_spend_history` already excludes Layer-1 deterministic spend from the Layer-2
  band (ADR 0038). A recurring card-charged bill is counted **exactly once** in `new_charges`
  (as a known charge), keyed by its `recurring_event` id so the projected-spend component can
  subtract it. **v1 status (4lhm):** the projected component reads only `variable_regular` /
  `variable_lumpy` categories, so a *properly categorized* recurring bill (`deterministic`) is
  already excluded — the common case. The **precise** key-by-`recurring_event` dedup (for a
  bill mis-categorized as variable) needs the posting↔instance link and is deferred to the
  actualization seam (`6wk.9` / `46jq`).
- **`finance_charge(cycle)`** = the average-daily-balance interest (ADR 0035 §4, owned by
  `llx5`): zero for a zero balance or a paid-in-full cycle.

The **payment** on the due date is selected by the card's `repayment_philosophy` (ADR 0035
§1): `pay_statement_balance` / `pay_in_full` → the statement balance; `pay_current_balance` →
the current owed balance including post-statement charges; `pay_minimum` → the minimum-payment
rule (ADR 0035 §5); `pay_fixed_amount` → the stored fixed amount; `unknown` → the minimum
(ADR 0035 §1 default). That payment is an **asset→liability transfer leg** (ADR 0035 §3): a
real `−amount` outflow in the **aggregate liquid forecast** on the due date that **reduces the
card's owed balance** (`6wk.4`).

### 3. A recurring bill can be charged to a credit card — the deterministic `new_charges`

A recurring bill's pay-from account (`autopay_account_id`) **may be a liability account**
(`cashflow_role = credit_facility`), not only a liquid account.

> **Two distinct fields.** A bill's pay-from is `recurring_events.autopay_account_id` — *which
> account settles this bill*. It is **not** `debt_terms.paying_source_account_id`, which is
> *which liquid account pays a given card/loan* (ADR 0035 §2). ADR 0035 §2 unified the scattered
> debt-account **paying-source** field names; it did **not** rename the recurring-bill pay-from.
> This section extends that bill pay-from so a **liability** value means "charge this card."

When the pay-from is a liability:

- The bill does **not** emit a liquid-cash outflow on its due date (subject to the gate below).
  Instead it is a **charge that increases the card's owed balance** on the bill date — the
  liability-increase leg, with sign handling by `cashflow_role` per ADR 0035 §3.
- It is the **deterministic** component of that card's `new_charges` for the cycle its date
  falls in. *"On the 5th, $90 lands on the card"* is both the user's mental model and a known
  forecast input — no projection needed for it.
- The card's eventual **payment** (per `repayment_philosophy`, on the due date) is the real
  liquid outflow, modeled **once** at the card level (§2 / `6wk.4`). A card-charged bill is
  therefore **never double-counted** as both a bill outflow and a card payment.
- **Display** (ADR 0018, descriptive): such a bill is still an upcoming obligation, attributed
  to the card rather than a liquid account; the liquid forecast shows the card *payment*, not
  the individual charge.
- **Gating — never make cash silently vanish.** A card charge replaces the bill's liquid
  outflow **only when the full payment is projectable**, which v1 takes to mean both:
  (a) the card's `debt_terms` carry a `statement_close_day` **and** a `payment_due_day` (the
  minimum needed to derive a cycle + due date; a `NULL` `paying_source` lands the payment on
  the aggregate per ADR 0035 §2); **and** (b) a **full-payment** `repayment_philosophy`
  (`pay_in_full` / `pay_statement_balance` / `pay_current_balance`), where the payment equals
  the charges. A **revolving/partial** philosophy (`pay_minimum` / `pay_fixed_amount` /
  `unknown`) is **not** retimed in v1 — retiming the full charge onto the due date would
  mis-model a partial payment, so the bill keeps today's charge-date liquid outflow until
  revolving interest is modeled (`llx5`). **Whenever the bill is not retimed, it keeps today's
  behavior — a liquid-cash outflow on its charge date** — so the obligation can never disappear
  from the liquid forecast. *Invariant:* a card-charged bill always produces **some** liquid
  impact across the horizon (its eventual card payment, or — when not retimed — its own
  outflow); it is never modeled as zero net cash.
- **Validation:** the kernel keeps the existing currency-match check on the pay-from account;
  it may be `liquid_cash` (a liquid outflow) **or** `credit_facility`. A `credit_facility`
  pay-from is treated as a card charge **only when the card's cycle is derivable** (the gate
  above); otherwise it falls back to a liquid outflow.
- **Actualization (deferred, `6wk.9`).** A retimed row is dated on the *due* date, but its
  `recurring_event_instance` is dated on the *charge* date, and the actualization seam (`46jq`)
  joins them by exact date — so a retimed card-charged row is not yet actualizable against the
  bill instance (it should eventually actualize against the card *payment*). Until then, retimed
  bills are **excluded from the forecast-accuracy readiness denominator** so they do not silently
  cap that factor; closing the loop is `6wk.9`.

**Backward compatibility:** bills with a liquid (or unset) pay-from are **unchanged** — still a
liquid outflow on the due date. Only a *liability* pay-from with a derivable cycle changes
behavior.

### 4. The owed-balance trajectory is a non-liquid per-account series

A card's owed balance over the horizon (`carried + charges + interest − payments`) is a
per-account, **non-liquid** trajectory surfaced in the debt / net-worth views (`6wk.5`), not
in the liquid line. The ADR 0035 §3 / ADR 0026 §12 reconciliation invariant is preserved:
**Σ liquid per-account closings == the aggregate liquid forecast.** Only the payment's *liquid*
leg enters the liquid aggregate; the charges (including card-charged bills) and the interest
live in the card's own trajectory and are excluded from the liquid line by construction.

### 5. Scope and sequencing (v1)

The arc builds deterministic-first, then statistical:

1. **Cycle + statement schema** (`xcq`): `credit_card_cycles` + `credit_card_statements` +
   revolving-interest columns. (The per-account debt *attributes* — APR, days,
   `repayment_philosophy`, `paying_source` — already shipped in `debt_terms`, `6wk.6`; `xcq`
   no longer carries them.)
2. **Cycle derivation + statement balance from known recurring card charges** (`kqez` + the
   `6wk.8` bill-charged-to-card mechanism) — the first tangible value (the owner's ask).
3. **Projected ordinary variable card spend** into `new_charges` (`4lhm`, on `pezm.1`).
4. **Interest** (`llx5`) and the **payment outflow** into the aggregate liquid forecast
   (`6wk.4`).

**Non-goals (v1):** rewards / cashback accrual, multiple cards sharing one statement,
sub-month proration of projected variable spend (month-bucketed is enough), and
foreign-currency cards (currency-match is enforced, as today).

## Consequences

- Bills paid by card stop wrongly depleting liquid cash on the bill date; the liquid impact
  appears correctly — and once — as the card payment.
- The statement forecast has a **deterministic backbone** (known recurring charges) before any
  statistical projection, so it is explainable and testable from day one and degrades
  gracefully when variable-spend history is thin.
- **Behavioral change with a test blast radius.** The change lives in the **shared**
  `collect_bill_events` collector (it feeds both the aggregate `compute()` and the per-account
  projection), so the liquid-pay-from path must be **regression-tested** alongside the new
  card-charged path. A recurring bill with a *liability* pay-from **and a derivable cycle** no
  longer emits a liquid outflow on its due date — it raises the card's owed balance, and the
  liquid hit appears once, later, as the card payment. **Any existing `recurring_events` row
  already pointing `autopay_account_id` at a card changes forecast behavior on upgrade** — this
  is the intended bug-fix (such bills were draining liquid cash on the wrong date), not a
  regression.

## Alternatives considered

- **Keep bills always liquid; add a separate "card charge" entity.** Rejected — it duplicates
  the bill and splits the user's single mental model (*"this subscription is on my Amex"*) into
  two records to maintain.
- **Model both a card charge and a liquid outflow on the bill date.** Rejected — double-counts
  the cash; the liquid outflow is the card *payment*, which happens later and is modeled at the
  card level.
- **Per-cycle user entry of expected charges.** Rejected — the recurring schedule + the
  classifier derive them; explicit entry is the manual-future-entry escape hatch (`q6gh`), not
  the everyday path.
- **Fold this into ADR 0035.** Rejected — 0035 deliberately scoped itself to the payment model
  and deferred the cycle math; the cycle/statement model + the charge-source decision are
  substantial enough to own a dedicated ADR that `xcq`/`kqez`/`4lhm`/`llx5` anchor to.

## Addendum (2026-07-01, personal-cfo-6wk.10): the full card payment supersedes per-bill retiming

§2 always specified that the card's **payment** (per `repayment_philosophy`, on the due date) is
the single liquid outflow. The `6wk.8` **v1** approximated that by *retiming the individual
card-charged bills* to the due date, gated to full-payment philosophies (where payment ≈ the
bills); revolving cards kept charge-date outflows because interest wasn't yet modeled. Now that
`llx5` models revolving interest, `6wk.10` implements §2 in full:

- **The card payment is the liquid outflow.** `collect_card_payment_events` projects one
  `−forecast_payment_minor` per cycle on the due date, from the card's paying source, for **every**
  card with a derivable cycle (revolving included: the payment is the minimum, and the remainder
  revolves per `llx5`). Cycles are derived across the **whole horizon** (not the view's fixed 3).
  The statement projection is shared with the view via `project_card_cycles`.
- **Per-bill retiming is removed.** A bill on a card with a derivable cycle is **suppressed** in
  `collect_bill_events` (no per-charge outflow); the card payment carries it. A bill on a card
  **without** a cycle still keeps its charge-date outflow — the "cash never silently vanishes"
  invariant holds unchanged.
- **Card variable spend leaves the Layer-2 band.** The projected ordinary variable card spend is
  now part of the card *payment* (via the statement), so `read_variable_spend_history`'s
  **aggregate** call (`account = NULL`) is restricted to `liquid_cash` — card variable no longer
  widens the household band (nor the readiness spend-months count, kept consistent). The per-card
  call (`llx5`) is unrestricted. This removes the double-count the band otherwise carried.

The result: a card's money is counted **exactly once** in the liquid forecast — as its payment.

## Addendum (2026-07-06, personal-cfo-4d8.23.2): `statement_close_day` is optional — a naive due-date-only projection

Owner dogfooding (2026-07-06): a debt should appear on the future-expected-transactions forecast
whenever there is an expected statement amount **and** a due date — nothing else. `statement_close_day`
(§1), APR, credit limit, grace period, and repayment philosophy are **optional enrichments**: the more
that is filled in, the more accurate the projection; the less, the more naive — never *required* to
project the upcoming payment. §1's derived-cycle model previously made the close day a hard requirement
(`read_cards_with_cycle` dropped any card missing either day), so a card entered with only a due day
vanished from the cash forecast.

**Decision.** `payment_due_day` is the only field required to emit a card's upcoming payment.

- **Naive path (no `statement_close_day`).** A `credit_facility` with a `payment_due_day` but no close
  day is projected **exactly like a loan** (ADR 0035 §3): the `repayment_philosophy`'s payment computed
  on the current **owed balance** (the naive "expected statement amount"), on each `payment_due_day` in
  the horizon — defaulting to the ADR 0035 §5 minimum rule (1%-of-balance / $25) when no philosophy or
  minimum terms are set, so the payment is a realistic amount rather than `$0`. This reuses
  `loan_payment` + the loan due-date emission; there is **no cycle**, so no interest/charge bucketing
  and no statement-analytics row (the naive card appears in the cash-outflow forecast, not the richer
  `card_statement_forecast` view, which still requires a derivable cycle).
- **Rich path (with `statement_close_day`) is unchanged** — §1–§2's derived cycles, revolving interest,
  and charge bucketing apply, refining the same due-date payment.
- **The owed balance is the naive expected statement amount.** Recording a *distinct* expected statement
  amount for a close-day-less card (an override not keyed by a cycle-close date) is out of scope here and
  deferred; the `credit_card_statements` PK `(account_id, cycle_close)` is untouched, so no migration.

Consequence: a card with only a due day now contributes its payment to the liquid forecast (counted once,
via the naive path), and progressively enriching its terms only sharpens the same projection.

**Interaction with charged bills (known limitation).** A recurring bill charged to a close-day-less card
keeps its charge-date outflow (§3, unchanged — only cards with a *derivable cycle* suppress their charged
bills). For the common case the naive path targets — a card entered as a **balance + due date, with no
bills charged to it** — there is no interaction and no double-count: the naive payment is the card's only
outflow. But when a card is *both* cycle-less *and* has recurring bills auto-charged to it, the two paths
**over-count**: each future bill occurrence leaves cash at its charge date *and* the naive payment services
the standing balance, so that card's activity is modeled more conservatively (more cash out) than a single
statement payment would be. This errs on the safe side for a cash forecast (it never understates outflow),
and it is resolved exactly by adding a `statement_close_day`, which switches the card to the cycle model —
future charges fold into the statement and the per-bill outflow is suppressed, counting the money once.
Reconciling the cycle-less-card-with-charged-bills case without a close day is tracked as a follow-up
(personal-cfo-4d8.23.10). (Past charges are not part of this: they already sit in the owed balance but
precede `today`, so they emit no outflow, and the naive amount derives from the current owed balance.)

## Addendum (2026-07-10, personal-cfo-4d8.25.1 / .2 / .3 + 4d8.23.9 / 4d8.23.10): the statement-accuracy contract

Owner dogfooding 2026-07-09 ("probably priority number 1"): with 12+ months of OFX history a
card's future statements projected as the *prior actual repeated once* and then a *bills-only
alternation*. Root causes and the resulting contract:

### 1. Statement-override lifecycle

- **An actual statement can only exist for a cycle that has closed.** Writes reject
  `cycle_close > today` (household calendar day); the projection ignores any stored override on
  a not-yet-closed cycle. Rationale: the UI keys a recorded statement to `cycles[0].close_date`
  *at the moment of writing*, and that leading close shifts across the due-date boundary (and
  was mis-derived during the grace window before the 4d8.23.1 fix) — a stale row keyed to a
  later close silently replays the old statement into the next cycle (the owner's repeated
  13,873.08). With both guards a mis-keyed row can neither be created nor applied early.
- **An override re-anchors the fold but never swallows incurred charges.** The owed opening can
  exceed the recorded statement — the difference is charges incurred *after* the close, already
  on the card but not on that statement. The fold carries it forward instead of dropping it:
  `closing = max(0, statement − payment) + max(0, (opening + new_charges + finance_charge) − statement)`.
  (Owner numbers: owed 17,082.23, statement 13,873.08 paid in full → the 3,209.15 of post-close
  charges open the next cycle rather than vanishing.) Cycles without an override are unchanged
  (`estimate − statement = 0`). **The carried un-billed remainder stays grace-eligible**: it is
  new spend, not unpaid statement debt, so the interest fold treats it like the next cycle's new
  charges (average-daily-balance half-weight; zero under grace when the prior statement was paid
  in full) — a pay-in-full card never accrues phantom interest on its post-close charges (ADR
  0035 §4 grace invariant).
- **Every stored statement row is user-visible and clearable.** The card's statement section
  lists all recorded rows (a row keyed to a future close is flagged as inert); repair of rows
  recorded under earlier, buggier derivations is an explicit user action — never a silent
  delete.

### 2. Statement-balance prediction from history (the model decision)

`new_charges` gains a **per-card estimator** so future statements track the household's actual
spend rather than collapsing to carried-balance + recurring bills. Signal tiers, best-available
blended:

- **T1 — card transaction history, including uncategorized rows**, bucketed per derived cycle
  window. (Today only *categorized* `variable_regular`/`variable_lumpy` spend counts, so
  uncategorized OFX history contributes zero — the direct cause of the bills-only alternation.)
- **T2 — statement-balance history + balance deltas between assertions + recorded payments** —
  a card is projectable with **no transaction detail at all**.
- **T3 — Layer-2 per-category bands** (ADR 0038 / `9h1s` / `pezm`) once shipped, replacing the
  v1 whole-card average as the categorized-detail signal.

Deterministic card-charged bills (§3) are subtracted from whatever a statistical tier estimates
for the same cycle window, so a bill is counted exactly once. The estimator **re-fits
continually** as data arrives (ADR 0026 §15 daily-on-open run); spend-level change detection is
the Layer-3 seam (`6i2k`). Accuracy is a tracked metric on the backtest spine (ADR 0026
§17–18): per-cycle statement error `|projected − actual|`, with the tolerance recorded alongside
the backtest baseline (bead `4d8.25.6`). Everything stays descriptive (ADR 0018). The estimator
is a registered model in the ADR 0026 §7 registry; implementation beads: `4d8.25.4` (statement
history capture/backfill), `4d8.25.5` (estimator v1), `4d8.25.6` (backtest).

### 3. Cycle-less card with charged bills (resolves 4d8.23.10)

A `credit_facility` with a `payment_due_day`, **no** `statement_close_day`, and at least one
active forecast-included card-charged bill is projected on **pseudo-cycles anchored on the due
day** (`close == due`; window = due-to-due): its charged bills fold into the pseudo-statement
and the philosophy's payment — and stop emitting charge-date liquid outflows (the §3 suppression
set extends to these cards). A cycle-less card with **no** qualifying charged bills keeps the
naive path (2026-07-06 addendum) unchanged. Money is counted once; the "cash never silently
vanishes" invariant holds — every folded bill lands in exactly one pseudo-statement payment.
To keep that invariant under degenerate terms, the cycle/pseudo-cycle payment applies the
ADR 0035 §5 **default minimum rule** (1%-of-balance / $25) whenever a minimum-paying policy
(`pay_minimum`, or `pay_fixed_amount` with no positive amount) has neither minimum term
configured — matching the naive loan path — so a suppressed bill can never be "carried" by a
perpetual $0 payment.

### 4. Monotonic due dates (resolves 4d8.23.9)

Derived due dates are **strictly increasing** across a card's cycle sequence. When a month-end
clamp collides two cycles onto one due date (close 30 / due 31 across February: the Feb-close
statement is due Mar 31 and the Mar-30 close would also resolve to Mar 31), the later cycle's
due date advances to the next occurrence of the due day (Apr 30) — one payment per statement,
never two payments on one derived date.

## Relates to

ADR 0035 (debt-payment model this extends), ADR 0026 (§12 reconciliation, §14 transfers),
ADR 0038 (ordinary/extraordinary classifier feeding projected card spend), ADR 0028
(role/subtype tokens), ADR 0018 (non-advice). Beads: `xcq`, `kqez`, `4lhm`, `llx5`, `6wk.4`,
`6wk.5`, `6wk.8` (recurring bill charged-to-card), and `6wk.10` (the full card-payment unification).
