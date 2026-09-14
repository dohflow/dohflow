# ADR 0035: Debt-payment forecast model — repayment philosophy, source mapping, asymmetric legs, finance charges

- **Status:** Accepted (ratified 2026-07-10, personal-cfo-4d8.25.1 — the model shipped as
  decided across `6wk.2`/`6wk.4`/`6wk.10`/`llx5`; no code divergence found)
- **Date:** 2026-06-30
- **Deciders:** Project owner
- **Beads:** [`personal-cfo-6wk.1`](../../.beads/issues.jsonl) (this ADR), gates `6wk.2`
  (debt-attribute entry), `6wk.4` (debt-payment outflow forecast), `6wk.5` (debt viz),
  `9h0.1` (investment DCA transfers), and reconciles `xcq`/`kqez`/`4lhm`/`llx5`
- **Builds on:** ADR 0007 (ledger/posting model), ADR 0026 (forecast architecture — §12
  per-account projection, §14 recurring transfers), ADR 0028 (account subtype + cashflow
  role), ADR 0018 (non-advice boundary)

## Context

The forecast today models money **coming in** (income) and **going out** (bills) of liquid
cash, and account-to-account **transfers** between two liquid accounts. It does **not** model
debt: a credit-card or loan **payment** is an outflow of liquid cash whose size depends on a
repayment choice the user makes, and a revolving card that is not paid in full **accrues
interest that compounds**. ADR 0026 §14 explicitly **deferred** this: transfers there are
liquid↔liquid, where the two legs net to zero and the aggregate forecast omits them; it noted
that a mixed-role transfer (asset→liability) "needs normal-balance sign handling and would not
appear in the liquid-only forecast."

That deferral is now the blocker for a whole arc of features (debt-attribute entry, the
credit-card bill forecast, interest projection, debt-paydown scenarios, the cash comfort band)
and for recurring investment contributions (asset→investment, the same shape). Five existing
beads (`xcq`, `kqez`, `4lhm`, `llx5`, `od07`) and the new `6wk.*`/`9h0.1` features all assume a
debt model that has never been decided. They also drifted apart — a credit-card-only
`pay_behavior` enum (`xcq`) competes with a general `repayment_philosophy`, and the
paying-account has been named five different ways across beads. This ADR makes the one
foundational decision they share, **before** any debt schema or feature is built (AGENTS.md
§1A). Per ADR 0018, every setting and projection here is **descriptive, not advisory** — the
mechanisms below never imply the user *should* pay a debt a particular way.

## Decision

### 1. A per-liability `repayment_philosophy` — one enum, replacing the scattered ones

Every liability account (subtype `credit_card`/`line_of_credit` → role `credit_facility`;
`mortgage`/`auto_loan`/`student_loan` → role `loan_liability`) carries a single
`repayment_philosophy` token. It is the **one** model; the credit-card-only `pay_behavior` of
`xcq` is reconciled into it (no two competing enums).

| `repayment_philosophy` | Forecasted payment amount on the due date |
|---|---|
| `pay_in_full` / `pay_statement_balance` | the projected **statement balance** (revolving) / current owed balance (installment) |
| `pay_current_balance` | the current owed balance as of the run (includes post-statement charges) |
| `pay_minimum` | the computed **minimum payment** (§5) |
| `pay_fixed_amount` | a stored `fixed_amount_minor` (a loan's scheduled amortization payment, or a user-set figure) |
| `unknown` | **defaults to `pay_minimum`** — the contractual floor, the realistic conservative assumption — and the readiness surface nudges (descriptively) that setting it sharpens the forecast |

`pay_statement_balance` and `pay_in_full` differ only for cards with post-statement activity;
they are distinct tokens so the projection (the card-cycle model, `kqez`/`4lhm`) can compute the
right base. The amount each maps to is produced by the credit-card cycle model for cards and by
the loan amortization schedule for installment debt — this ADR fixes *which* base each token
selects, not the cycle math (that is `kqez`/`4lhm`/`40f`).

**Why `unknown → pay_minimum` and not "no payment" or "pay in full":** a forecast that omits a
debt payment understates cash depletion (false comfort); one that assumes pay-in-full when the
user revolves overstates it (false alarms). The contractual minimum is the realistic floor of
what *will* leave the account, so it is the safe neutral default. It is a default, never a
recommendation (ADR 0018).

### 2. One authoritative debt → paying-account mapping

Each liability account carries a single nullable `paying_source_account_id` — the **liquid**
account its payments come from. This is **the** attribution key, superseding the scattered
**debt-account** paying-source field names (`payment_source_account_id` /
`payment_method_account_id`) that drifted across beads. (It does **not** rename
`recurring_events.autopay_account_id`, a *recurring bill's* pay-from — a distinct concept; ADR
0039 §3 extends that field so a liability value there means "charge this card.") The kernel validates it on write, mirroring the existing
role↔subtype check: the **target** must be a liability-role account and the **source** must be
`liquid_cash`. A `NULL` mapping means "unattributed" — the payment still reduces the owed
balance in the debt account's own projection, but cannot be attributed to a specific liquid
account, so it lands against the **aggregate** liquid forecast only (it must still deplete cash;
see §3). Setting the mapping is descriptive data entry, not a prompt to consolidate payments.

### 3. Asymmetric transfer legs — the liquid leg counts in the aggregate (extends ADR 0026 §14)

A debt payment is an **asset→liability** transfer; a contribution is an **asset→investment**
transfer. Both are *asymmetric*: the two legs are different `cashflow_role`s, so — unlike the
liquid↔liquid case — they **do not net to zero** in the liquid pool. This ADR resolves §14's
deferral for both shapes with one rule:

> For a transfer whose **source** role is `liquid_cash` and whose **destination** role is in
> {`credit_facility`, `loan_liability`, `investment_asset`, `real_asset`}: the source (liquid)
> leg is a real **−amount outflow** that appears in **both** the per-account series
> (`compute_by_account`) **and the aggregate liquid forecast** (`compute`). The destination leg
> is handled in the destination account's **own** projection and is **excluded** from the
> liquid aggregate.

- **Asset→liability (debt payment).** The destination (liability) leg **reduces the owed
  balance** — a liability's normal balance falls when paid (`owed += charges + interest −
  payment`, §4). It belongs to the debt account's balance trajectory, not the liquid forecast.
- **Asset→investment (DCA, `9h0.1`).** The destination (investment) leg **increases the
  investment balance** / net worth; it is not liquid cash, so it stays out of the liquid
  forecast. Net worth is flat on the transfer day (cash down == investments up); liquid cash
  is genuinely lower.

**Sign handling is by `cashflow_role`, not by sign of the stored amount**, so a single
`recurring_transfers`-style two-leg record serves all shapes. The existing liquid↔liquid path is
unchanged (legs still cancel and are still omitted from the aggregate).

**Reconciliation invariant (revises ADR 0026 §12).** The invariant becomes: **Σ per-account
closings over the liquid accounts == the aggregate liquid forecast.** Liquid↔liquid transfer
legs net to zero within that sum (unchanged); an asymmetric transfer contributes its **liquid
leg only** to the sum (the non-liquid leg lives in a non-liquid account's series and is excluded
from the liquid aggregate by construction). Non-liquid accounts (liabilities, investments) carry
their own balance trajectories, surfaced separately (the debt/net-worth views, `6wk.5`).

### 4. Finance charges are a derived projection, not a user assumption event

When a card is not paid in full, the carried balance accrues interest that **compounds** across
cycles. A projected finance charge is a **consequence** of the model, not an input, so it is a
**credit-cycle-derived** projection (owned by the card-cycle model `kqez`/`4lhm`/`llx5`), **not**
a first-class `forecast_assumption_event` (those are user inputs, ADR 0026 §4). Per cycle:

```
daily_periodic_rate = apr_bps / 10_000 / 365
finance_charge      = avg_daily_balance(cycle) * daily_periodic_rate * days_in_cycle
next_opening        = opening + new_charges + finance_charge − projected_payment
```

`next_opening` carries forward as the following cycle's opening balance and re-accrues — the
**compounding loop**, run out to the forecast **horizon cap** (the 365-day daily-persist
horizon). A projected finance charge increases the **owed balance** (the liability trajectory),
and — because `pay_statement_balance`/`pay_current_balance` pay that balance — flows into the
**next payment's** liquid outflow. Interest thus affects liquid cash *indirectly*, through larger
future payments, never as a direct liquid-cash line. Property invariants the implementers must
hold: zero balance ⇒ zero interest every cycle; paid-in-full each cycle ⇒ zero interest.

### 5. Stored debt attributes + the minimum-payment rule

The debt foundation (the schema bead, reconciling `xcq`/`40f`) persists, per liability account:
`apr_bps` (nullable; basis points, integer — no f64), `statement_close_day` + `payment_due_day`
(1–31, with a documented **month-end clamp** for short months), `grace_period_days`,
`credit_limit_minor`, `repayment_philosophy` + `fixed_amount_minor`, and `paying_source_account_id`
(§2). The **minimum-payment rule** is a small typed rule — `greater_of(percent_bps_of_balance,
floor_minor)` with a sensible default (e.g. 1%-of-balance-or-$25) — so a minimum can be projected
*before* a statement posts. APR/terms are **user-entered** in v1; **inferring** APR from realized
interest postings (`zc8q`) is a later enhancement, not part of this decision. Every projected
debt event carries an `assumption_basis`/provenance per ADR 0026 §1 so the row is explainable.

Of these attributes, only `payment_due_day` is **required** to project an upcoming payment: it dates
the outflow, and the owed balance under the minimum-payment rule supplies the amount. `apr_bps`,
`statement_close_day`, `grace_period_days`, `credit_limit_minor`, and `repayment_philosophy` are
**optional enrichments** — the more that is filled in, the more accurate the projection; the less, the
more naive (personal-cfo-4d8.23.2, ADR 0039 addendum 2026-07-06). A card with a due day but no close
day is projected naively, like a loan, rather than dropped from the forecast.

### 6. Non-advice framing (ADR 0018)

All of the above is **descriptive**. The settings are neutral data entry (no nudged default
philosophy — `unknown` is a neutral state, not "you should pay in full"). The projections state
what follows from the user's own settings — *"Based on your full-statement autopay, $1,240 is
projected on Jun 28"* — and never prescribe a repayment strategy, a paydown order, or a "you
should pay X" directive. The cash-band drift signals that consume this model (`5ie.8`) are
governed by ADR 0035's sibling decision on the recommendation boundary (`915.1`).

## Consequences

- **Unblocks the arc.** The debt schema (`6wk.2`/schema), the debt-payment outflow forecast
  (`6wk.4`), interest projection (`llx5`), debt-paydown scenarios (`od07`), the comfort band
  (`5ie.8`), and investment DCA (`9h0.1`) can now build against a fixed model. `6wk.3` reconciles
  `xcq`'s `pay_behavior` into §1's enum and the five source-account field names into §2's one.
- **The aggregate forecast changes shape.** `compute()` must now include the liquid leg of
  asymmetric transfers (today it omits all transfers). This is the one behavioral change with a
  test blast radius — the per-account/aggregate reconciliation tests must be extended to the
  asymmetric case.
- **Liability/investment balances become first-class forecast outputs**, surfaced in the debt
  and net-worth views (`6wk.5`) rather than the liquid line.
- **Irreducible complexity:** the card-cycle + compounding math is genuinely intricate; this ADR
  contains it to the cycle model and keeps the liquid forecast's contract simple (debt payments
  are just attributed outflows).

## Alternatives considered

- **Finance charges as user assumption events.** Rejected — interest is a derived consequence;
  modeling it as an input would let edits desync it from the balance and duplicate the cycle
  math.
- **Liability leg in the liquid aggregate (signed).** Rejected — a liability balance is not
  liquid cash; mixing it into the liquid line breaks the "cash I can spend" meaning and the
  reconciliation invariant.
- **`unknown` ⇒ no projected payment.** Rejected — understates cash depletion (false comfort).
- **A separate ADR for asset→investment legs.** Folded in here — asset→liability and
  asset→investment share the exact normal-balance sign-handling problem; one rule (§3) serves
  both, avoiding a second near-identical ADR.

## Relates to

ADR 0026 (extends §12 reconciliation + §14 transfers), ADR 0028 (the role/subtype tokens this
keys on), ADR 0018 (non-advice), ADR 0030 (categorization feeds the spend that drives the
card-statement projection). Decision sibling: `915.1` (cash-band recommendation boundary).
