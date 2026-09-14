# ADR 0038: Spend classification — ordinary vs extraordinary (v1 deterministic)

- **Status:** Accepted (ratified 2026-07-10, personal-cfo-4d8.25.1 — the v1 classifier shipped
  as decided; the override-learning loop remains split out to `pezm.2`)
- **Date:** 2026-06-30
- **Deciders:** Project owner
- **Beads:** [`personal-cfo-pezm.1`](../../.beads/issues.jsonl) (this ADR + the v1
  classifier), under the `pezm` flagship; refines the Layer-2 baseline
  (`read_variable_spend_history` / `9h1s`); consumed later by `4lhm` (credit-card
  statement forecast) and `5ie.8` (cash comfort-band drift). The persisted
  override-learning loop + merchant-key wiring are split out to `pezm.2`.
- **Builds on:** ADR 0026 (forecast architecture — §7 Layer-2 spend band, §13a readiness),
  ADR 0030 (categorization — `forecast_behavior` taxonomy, merchant identity `zrpg`),
  ADR 0018 (forecast language / non-advice boundary).

## Context

Layer-2 of the forecast (`personal-cfo-9h1s`, ADR 0026 §7) learns a per-category
variable-spend **band** from the household's own history: `read_variable_spend_history`
([`crates/db-worker/src/forecast.rs`](../../crates/db-worker/src/forecast.rs)) reads every
`variable_regular` / `variable_lumpy` posting in a 24-month window and feeds it to
`SpendModel::fit`.

It feeds in **every** such posting — including one-off spikes. A single $3,000 vacation, a
furniture purchase, or a medical bill lands in the same per-category sample set as the
weekly groceries, so the fitted band says the household spends like that *every* month. The
baseline is overstated and the band is noisily wide. The function's own comment already
flags this: *"Exact recurring-vs-discretionary matching tightens this later."*

Three shipped-or-imminent features need the same missing primitive first:

- the Layer-2 baseline itself (exclude one-offs so the band reflects ordinary behavior),
- the credit-card statement/payment forecast (`4lhm`) — projecting next month's card bill
  from *ordinary* card spend, not a vacation that won't repeat,
- the cash comfort-band drift signal (`5ie.8`) — distinguishing a real downward drift from
  a single large discretionary charge.

That primitive is a per-posting label: **ordinary** (recurring / baseline behavior) vs
**extraordinary** (one-off). The full `pezm` vision — a continuous statistical model that
deduces seasonality and learns from user overrides — is research-shaped and gated on the
`r6o5` classifier + `pt7k` eval harness. This ADR scopes the **first deterministic
increment** that those features can build on now.

## Decision

### 1. The label is a per-posting classification with a confidence

A pure, deterministic classifier produces, for each spend posting:

- `SpendClass` ∈ { `Ordinary`, `Extraordinary` },
- a `confidence_bps` in `0..=10_000`,
- a machine `reason` token (e.g. `amount_outlier`, `recurring_merchant`, `below_floor`) for
  inspection/explainability. The reason is descriptive, never advisory (ADR 0018).

It lives in the **`categorization` crate** — pure, no `rusqlite` / `tokio` / clock / RNG,
exactly like `normalize_merchant`. `db-worker` assembles the per-category history and calls
it (the forecast-engine pattern: pure core + a DB adapter). Identical inputs → byte-identical
labels (sorted inputs, integer-cent math), so the persisted-forecast pipeline stays
reproducible.

### 2. v1 signal — robust amount-vs-own-category-history, with a recurring-merchant guard

A posting is **extraordinary** when *all three* amount tests hold and the merchant guard
does not veto:

1. **Outlier above the category's robust upper fence:** `amount > median + K·MAD`, where
   `median` and `MAD` (median absolute deviation) are computed over the *individual posting
   magnitudes* in that posting's category across the window. Median + MAD is used rather than
   mean + standard deviation because a single huge outlier inflates the mean/σ and masks
   itself; the median/MAD pair is robust to exactly the spikes we are hunting.
2. **At least `RATIO`× the median:** guards a tightly-clustered category whose `MAD` is near
   zero from flagging everything modestly above median.
3. **At least `ABSOLUTE_FLOOR`:** a cheap category's noise never yields an extraordinary
   label.

**Recurring-merchant guard:** a posting whose merchant recurs `≥ RECUR_MIN` times in the
window is **ordinary** regardless of amount — a regular large grocery run or a monthly bill
is baseline behavior, not a surprise. This is the merchant-identity (`zrpg`) signal.

> **v1 wiring caveat (intentional, documented):** there is no reliably-populated merchant key
> on *committed* ledger transactions yet (`normalized_merchant` exists only on import
> staging). So the v1 `db-worker` adapter supplies "no recurrence information" and the guard
> is a no-op in production today. The classifier is written and **tested** with the guard, so
> lighting it up later (`pezm.2`) is a pure data-plumbing change, not a logic change.

Constants live as documented crate consts and are tunable: `K` (MAD multiplier), `RATIO`
(multiple-of-median), `ABSOLUTE_FLOOR` (minor units), `RECUR_MIN`. `confidence_bps` is
monotone in the distance above the fence (clamped to `10_000`) so consumers can threshold.

**Scope — regular categories only.** The classifier is applied to `variable_regular`
categories (groceries, gas, utilities — where steady behaviour is expected and a single big
charge pollutes the baseline). `variable_lumpy` categories (property tax, maintenance,
rideshare …) are the taxonomy's *explicit* bucket for irregular/large spend: their big
charges are the signal, and `SpendModel`'s per-category empirical quantiles already model
that lumpiness. So lumpy postings are **kept whole** — never classified or excluded. A
deeper model of lumpy spend is `personal-cfo-l3id`'s concern, not this classifier's.

### 3. Consumer contract

- **Layer-2 baseline (this PR):** `read_variable_spend_history` classifies each
  `variable_regular` posting per category and **drops the extraordinary ones** before
  `SpendModel::fit`, so the band models ordinary spend; `variable_lumpy` postings pass
  through untouched. A `variable_regular` category with fewer than `MIN_SAMPLES` (4) postings
  in the window is too thin to judge an outlier, so it is kept whole (unclassified) rather
  than risk excluding ordinary spend — a consumer-side gate alongside the classifier's own
  `K` / `RATIO` / `ABSOLUTE_FLOOR` / `RECUR_MIN`. The maturity gate (`distinct_spend_months`)
  still counts months from the postings that remain.
- **Card forecast (`4lhm`) and comfort band (`5ie.8`):** consume the same `classify` seam
  when they land — they do not re-derive "what is a one-off."

### 4. User overrides are training signal — split to `pezm.2`

A user marking a posting ordinary/extraordinary (which the model must then honor and learn
from) needs a persistence table, a kernel command, and a UI — a separate concern. v1 ships
the deterministic classifier + the baseline exclusion; `pezm.2` adds override persistence,
the committed-transaction merchant-key wiring (lighting up the guard above), and — later —
the statistical model. Captured here so nothing is lost.

### 5. Non-goals (v1)

- No seasonality deduction (that is `SpendModel`'s calendar-month bands + later `pezm` work).
- No ML / statistical classifier (`r6o5`).
- No override-learning loop or label-persistence store (`pezm.2`).
- No UI surface — in v1 the labels are an internal modeling input; surfacing them is later.

## Consequences

- The Layer-2 baseline stops being skewed by one-off spikes → a tighter, more honest band on
  real households with lumpy history. This improves the already-shipped Future Cash band.
- A single tested seam (`classify`) that the card-bill forecast and comfort band reuse,
  rather than each re-inventing one-off detection.
- The merchant signal is built and tested but **dormant** until a committed-transaction
  merchant key exists (`pezm.2`) — a documented, intentional defer, not a silent gap.
- v1 is conservative by construction (three conjunctive amount tests + a guard): it favors
  **precision** (only clear one-offs are excluded) over recall, which is the safe bias for a
  modeling input — wrongly excluding ordinary spend would understate the baseline.

## Alternatives considered

- **Mean + standard deviation outliers** — rejected: non-robust; one large outlier inflates
  the statistics and hides itself.
- **Reuse `SpendModel`'s empirical category quantiles** — rejected for v1: needs more samples
  to be stable and couples the classifier to the band model. Median + MAD with the ratio +
  floor guards degrades gracefully on thin history.
- **Frequency-only (a never-before-seen merchant ⇒ extraordinary)** — rejected as the primary
  signal: the committed-txn merchant key is not populated yet, and a first-time-but-ordinary
  purchase should not be extraordinary. Kept instead as the *recurring-merchant guard*.
- **Build the full statistical `pezm` model now** — rejected: research-shaped, gated on
  `r6o5` / `pt7k`. This deterministic increment unblocks the card/band features immediately
  and is a defensible baseline the statistical model can later be evaluated against.
