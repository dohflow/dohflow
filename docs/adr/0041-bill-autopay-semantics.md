# ADR 0041 — Bill autopay is intent/label, not a forecast-projection change

- Status: Accepted
- Date: 2026-07-02
- Bead: personal-cfo-mc7f (deferred enhancement: personal-cfo-mc7f.1)
- Related: ADR 0027 (additive balance model), ADR 0018 (forecast language / non-advice), 5ie.9 (ConfirmObligationEarly)

## Context

A recurring bill can be **autopay** (it debits itself from a linked account on the due date) or
**manual** (the user pays it). The MLP pay-and-confirm loop needs to distinguish the two so the
daily driver shows which bills need action.

The model already had *implicit* autopay state: `bill_contracts.autopay_enabled` was written as
`autopay_account_id.is_some()` (a synthetic mirror of "an account is linked"), `autopay_status`
(`enabled`/`disabled`/`unknown`) was never written, and nothing read either for behavior. There was
no way to record the user's intent ("this bill autopays") independent of picking an account.

When scoping this, two independent analyses proposed making the **forecast projection itself**
autopay-aware — either (a) proactively *suppressing* upcoming autopay occurrences, or (b) projecting
*past-due* autopay occurrences as assumed-paid. Both are hazardous:

- (a) is **wrong**: an autopay bill due next week is a real future outflow. Suppressing it would
  overstate liquid cash — the money still leaves on the due date whether or not a human triggers it.
- (b) is **useful but unsafe without care**: under the additive balance model (ADR 0027) a posting
  is only counted if dated after the paying account's latest manual balance assertion. If the balance
  was asserted on/after the due date it *already* reflects the autopay; assuming it again double-counts.

## Decision

**Autopay is an intent flag on the bill. It does not change what the forecast projects.**

- Every bill — autopay or manual — projects as an outflow on its due date, exactly as today. The
  cash projection stays correct; nothing is hidden or assumed.
- The flag is authoritative on `recurring_events.autopay_enabled` (nullable: `1` autopay, `0`
  manual, `NULL` legacy/unknown), set from an explicit `autopay` on Create/Update, and mirrored to
  `bill_contracts` so the commitments read model reflects intent rather than account presence.
- The distinction is **presentational**: the bill list badges autopay bills, and the Future Cash
  mark-paid control reads "Confirm it cleared" for an autopay occurrence (it pays itself; you're
  recording that it did) vs "Mark paid" for a manual one. Both drive the same
  `ConfirmObligationEarly` (5ie.9) — autopay does not get a distinct posting path.

## Consequences

- Safe and simple: no projection-hiding, no double-count risk; the flag is opt-in metadata.
- The genuinely useful "assume a recently-passed autopay occurrence cleared so a **stale** balance
  reflects it" behavior is **deferred** to `personal-cfo-mc7f.1`, which must first define — in an ADR
  — how it guards against the additive-model double-count (consult the paying account's latest
  assertion date; bound by a look-back window; never double-count a real confirm).
- `autopay_account_id` remains "which account", orthogonal to `autopay_enabled` "does it pay itself".
