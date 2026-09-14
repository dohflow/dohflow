# ADR 0044 — Account-model extensions: real-asset subtypes, value-vs-balance, account notes, real-asset↔liability linking

- Status: Accepted
- Date: 2026-07-05
- Bead: personal-cfo-4d8.22.1 (foundation for the 4d8.22 accounts + setup UX overhaul)
- Related: ADR 0028 (account subtypes + cash tiers), ADR 0037 (Accounts IA / Debt sub-view), ADR 0027 (additive balance model), ADR 0035 (debt model)

## Context

The Accounts surface is being reshaped from owner dogfooding feedback (2026-07-05) plus a Claude
Design layout (`Accounts.dc.html`): a two-column **Assets | Liabilities** view with a net-worth
summary, a single **one-form editor** that captures debt terms *at account entry*, and first-class
**real assets** (a house, a car) shown as an estimated *value* and linkable to the loan that
finances them.

The current model (ADR 0028) has the `RealAsset` cashflow role but **no real-asset subtypes**, no
way to present a real asset as a *value* rather than a *balance*, no free-text notes on accounts,
and no linkage between a real asset and its financing liability. These are architecturally
significant (schema + read-model + display semantics), so this ADR records the decisions before the
code lands.

## Decision

**1. Real-asset subtypes.** Add to `core_ledger::AccountSubtype`, all mapping to
`CashflowRole::RealAsset`:
- `Property` (`"property"`) — a home, land, real estate.
- `Vehicle` (`"vehicle"`) — a car, boat, etc.
- `OtherRealAsset` (`"other_real_asset"`) — anything else owned that carries value.

They have **no cash tier** (`cash_tier() == None`): real assets are not liquid and never enter the
spendable/reserve cash rollups. The `accounts.subtype` CHECK-constraint token set is extended to
include them (migration v36).

**2. Value vs. balance is a DISPLAY decision, not a model change.** A real asset still stores its
worth as an ordinary balance assertion (ADR 0027) — the ledger, the additive-balance plug, and
net-worth math are all unchanged. The UI simply **labels** a real-asset account's figure "Value"
(its current estimated valuation) instead of "Balance," and the set-figure control reads "Update
value." No new column, no new command: presentation is role-aware in the frontend. Net worth =
Σ assets − Σ liabilities as today; real-asset values are assets like any other.

**3. Account notes.** Add a nullable `accounts.notes TEXT` column and a `SetAccountNote` command
(mirroring the transaction-note pattern), surfaced on `AccountViewDto.notes` and edited in the
account editor's Notes section. Free-text, no semantics.

**4. Original principal on debt terms.** Add a nullable `debt_terms.original_principal_minor
INTEGER` — the loan's starting principal, useful for payoff/amortization context. It does **not**
change the cash forecast (which projects from the current owed balance + the payment amount); it is
descriptive metadata carried through `DebtTermsDto`.

**5. Real-asset ↔ liability linking (model decided; implementation deferred).** A real asset may be
**linked** to the single liability that finances it (a property → its mortgage, a vehicle → its
auto-loan). The model:
- A nullable **one-to-one** association, stored as `accounts.linked_account_id BLOB REFERENCES
  accounts(id)` on the **real-asset** row (the asset points at its liability). One asset links one
  liability; the reverse (a liability's linked asset) is found by query. Multi-liability cases (a
  house with a mortgage *and* a HELOC) are out of scope for v1 — a documented limitation.
- **Display-only.** Linking changes no math: net worth already nets assets against liabilities
  regardless of links. The link exists to *show the relationship* — both the asset (Assets column)
  and the liability (Liabilities column) surface "Linked to <name>," and details show the pair.
- Validation: a link is only allowed asset(real_asset) → **`loan_liability`** (narrowed
  2026-07-06, personal-cfo-4d8.23.4 — a home/vehicle is financed by a *loan* or *mortgage*, not a
  revolving `credit_facility`; a credit card is never a sensible financing target, so it is not
  offered in the picker and is rejected on write). Self-links and asset↔asset / liability↔liability
  links are rejected. **One-to-one is enforced on write**: linking a loan that is already the target
  of a *different* asset is rejected, so a liability resolves to exactly one asset (the reverse
  lookup is then deterministic). A "Linked to …" chip surfaces only when the **partner is active** —
  archiving either side hides the chip rather than leaving a stale reference on the live account.

The linking **implementation** (schema column, `SetAccountLink` command, editor Link section, view
display) ships as a **fast-follow PR** (bead 4d8.22.3), *after* the accounts redesign lands, per the
owner's scope decision. Deciding the model here keeps that follow-up pure implementation.

## Consequences

- Adding `RealAsset` subtypes makes the earlier "RealAsset role with no subtypes" state whole; the
  role↔subtype validation (db-worker apply) now covers real assets like every other role.
- The value-vs-balance choice keeps the ledger honest (no special-casing in the double-entry core);
  only the presentation layer branches on `real_asset`.
- Notes and original-principal are additive, nullable, and rebuild-safe (they are canonical columns,
  not derived read-model fields).
- The linking column is additive and nullable; because it is display-only, deferring it does not
  block the redesign — the view simply omits link chips until the fast-follow.
- Migration v36 bundles the three additive schema changes for this PR (subtype tokens, `notes`,
  `original_principal_minor`); the `linked_account_id` column lands in the fast-follow's own
  migration.
