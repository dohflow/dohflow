# ADR 0056 — An archived liquid account leaves the forecast

- **Status:** Accepted
- **Date:** 2026-08-06
- **Bead:** `personal-cfo-4d8.27.5.6`
- **Governs:** every read that sums `cashflow_role = 'liquid_cash'`
- **Related:** ADR 0044 (account model), ADR 0026 §13 (forecast readiness)

## Context

Archiving an account sets `active = 0` and nothing else. **Six** forecast-facing reads
select liquid accounts with no `active` filter, so an archived account keeps anchoring the
household's projection:

| Read | What it feeds |
| --- | --- |
| `forecast/events.rs` `liquid_starting_balance` | the projection's opening cash |
| `forecast/account_series.rs` `read_liquid_accounts` | the per-account series |
| `lib.rs` `cash_tier_rollups` | Spendable / Reserve / Net |
| `lib.rs` post-observation posting sums | balances since the last assertion |
| `forecast/readiness.rs` freshness | the "balance freshness" score |
| `forecast/readiness.rs` history unlock | whether history is offerable |

The freshness query already carries an explicit contract from a previous author:

> This account set MUST match the accounts the forecast's starting balance sums … so an
> archived-but-still-projected account drags freshness exactly as it anchors the forecast
> … **Do NOT add `active = 1` here without also excluding archived accounts from the
> forecast.**

That contract is right, and it is what makes this a single decision rather than six.

## Decision

### 1. An archived liquid account is excluded from the forecast

All six reads filter `active = 1`, **together**.

The deciding argument is direction of error. This app's wedge is *"how much cash will this
household probably have"*. An archived account is one the user has retired — closed,
moved away from, replaced. Continuing to count its balance **overstates** available cash,
and overstating cash is the error that produces an overdraft. Understating is the survivable
direction.

It also matches what the rest of the app already says: archived accounts are hidden from
the Accounts list behind a "Show archived" toggle. Only the forecast disagreed.

And it removes a defect the user cannot escape: today, an archived account drags the
**Balance freshness** score, so the app reports stale data because of an account the
household deliberately retired — with no action available short of un-archiving it.

### 2. The forecast set and the freshness set move together, permanently

The previous author's contract is preserved, only in the other position: freshness must
cover exactly the accounts the forecast rests on. Adding an `active` filter to one of these
reads without the others reintroduces the failure it was written to prevent — readiness
reporting "current" over a projection resting on a stale balance, or the reverse.

The reconciliation invariant depends on the same thing: `read_liquid_accounts` (per-account
series) and `liquid_starting_balance` (the aggregate) must sum the same set, or the
per-account cones stop adding up to the aggregate band.

### 3. History-unlock counts archived history

The history-unlock check asks *"is there enough past to draw?"* — a question about the
data, not about which accounts are current. Postings made while an account was active are
real history and remain so after archiving. This read filters `active = 1` for consistency
of the *account set*, but the practical effect is nil in the common case and the honest
reading is that it should not resurrect a retired account to unlock a feature.

## Consequences

- **A household that archived a liquid account will see its forecast starting balance
  drop** on the next run. That is the point, and it is the correction: the previous number
  included money in an account they had retired.
- **Archiving an account that still holds a balance now understates cash.** This is the
  accepted cost of choosing the safe direction — but it must not be silent. Archiving an
  account with a non-zero balance should say what happens to the forecast. Tracked as
  `personal-cfo-tiqf`.
- Balance freshness stops being dragged by retired accounts, so the readiness score can
  reach "current" again for a household that has archived anything.
- The six reads are now coupled by an invariant that no test can express locally. The
  test that protects it is the reconciliation assertion — per-account series sum to the
  aggregate — plus a test that an archived account drops from *both* the projection and
  freshness in the same fixture.
- Nothing here changes ledger history. An archived account's transactions, postings and
  past balances are untouched; only the forward projection stops counting it. Reinstating
  restores it.
