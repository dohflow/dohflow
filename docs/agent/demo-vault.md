# The Polish Demo vault

The **Polish Demo** vault is the fixture every DohFlow screenshot, design
walkthrough, and dogfooding-by-proxy session runs against. It is seeded by
`apps/desktop/src-tauri/tests/seed_polish_vault.rs` (beads `personal-cfo-2pcx`,
refreshed in `personal-cfo-4d8.28.4`) and lives in the app-data directory,
whose path keeps the frozen bundle identifier (ADR 0067):
`~/Library/Application Support/ai.personalcfo.desktop`.

## Invocation

The seeding body is one function, `seed_demo_vault(root, anchor)`, with two
entry points.

**Seed the real app-data root** (ignored test, double-gated on `PCFO_SEED_ROOT`):

```sh
PCFO_SEED_ROOT="$HOME/Library/Application Support/ai.personalcfo.desktop" \
  cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml \
  --test seed_polish_vault -- --ignored --nocapture
```

It registers a **new** "Polish Demo" entry in that root's `vaults.json` and
writes `vaults/<id>/vault.db`. Existing vaults' files are never touched; the
new vault becomes the active one, so the next launch opens it. Run it again to
get a second, fresh copy (old copies are deleted from inside the app).

- **Passphrase:** `polish-demo`. It is a published fixture, not a secret — the
  vault holds invented data only.
- **Anchor:** every seeded date is an offset from one anchor, the household's
  "today". The manual seed defaults to the local date, so today-relative
  surfaces (past-due obligations, the stale balance, upcoming bills, the next
  paycheck) look right the day you take the screenshot. Pin it for a
  reproducible run with `PCFO_SEED_ANCHOR=2026-09-01` (`YYYY-MM-DD`).

**CI guard** (not ignored): `seeded_demo_vault_populates_every_screenshot_surface`
seeds a unique temp root with a fixed anchor (2026-09-01) and asserts every
screenshot surface has content. It runs with the rest of the desktop crate:

```sh
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --test seed_polish_vault
```

Never point `PCFO_SEED_ROOT` at a root you do not intend to add a vault to,
and never at someone else's `~/Library`.

## What gets seeded, per screenshot surface (`n76x.13`)

| Surface | Content |
| --- | --- |
| **Dashboard / forecast** | Cash floor $5,000 and comfort band up to $12,000; two income sources (semi-monthly and biweekly); nine recurring bills, mixed autopay and manual, categorized; a monthly DCA transfer into the brokerage; two one-time future entries (property tax, quarterly bonus); a year of imported checking history auto-reconciled onto the bills' and paychecks' occurrences; one manual bill (the piano studio) left **past due and unconfirmed** so Cash Flow shows exactly one ADR 0058 obligation; the current daycare occurrence confirmed early by hand. |
| **Scenarios** | Three scenarios that compose (ADR 0059): *Parental leave (fall)* — two income windows on one partner's pay, marked active; *Kitchen remodel* — two one-time outflows with an expiry date; *Daycare rate increase* — a bill-amount override. |
| **Money Inbox** | One flagged suspected duplicate from the CSV import (with its committed twin for the review panel); a stale-balance nudge (the savings balance is three weeks old); a short unreviewed-transaction queue (the newest three imported rows). The connection is healthy, so there is deliberately no connector-error item. |
| **Accounts and debt** | Eleven accounts across every cashflow role and subtype: checking, savings, two credit cards, an auto loan, a mortgage, a brokerage, a 401(k), an HSA, and two real assets (home, car) linked to the liabilities that finance them (ADR 0044). Debt terms on both cards and the auto loan (fixed payment, original principal), one recorded card statement, so the payoff comparison and card-cycle views are full. Cash tiers show both spendable and reserve money (ADR 0028). |
| **Import** | One generic-CSV batch (`saltmarsh-checking.csv`, about 180 rows): a year of paychecks and bill payments with bank-style spellings and reference numbers, a side gig and a gym the detectors surface as *suggested income* and *suggested recurring* (the onboarding suggestions), recent everyday debits, and one repeated row that the import flags. |
| **Vault / backup** | The vault is registered and active in the picker, reports healthy, exports a backup that restores into a fresh location, and reopens with the demo passphrase. |
| **Settings › connections** | One linked connection, both external accounts mapped, synced once, no error. It uses the deterministic mock adapter (accounts-only, so no fixture rows land in the ledger) under the schema token `other`; no real token or network is involved. Because no production adapter carries that id, the app's unlock auto-sync skips it and the row stays as seeded. Its display hint reads "Mock bridge, 2 accounts" — the one place the fixture shows its seams. |

## Data rules

- **Every name is invented**: institutions (Saltmarsh CU, Kestrel, Copperleaf,
  Tidepool, Quillfeather, Foxglove, Larkspur), employers (Ledgerline Systems,
  Northhollow Health), merchants (Lantern Grocery, Glassmoor Utilities, Maple
  Street Piano Studio, and so on), and people (Rowan). No real brand, bank,
  product, or anything traceable to the owner. Keep it that way when adding
  rows.
- **Amounts are plausible and not round**; none come from any real ledger.
- **Dates are offsets from the anchor**, never literals, so the stored data is a
  pure function of the anchor and the CI run is deterministic.
- **Descriptions in the CSV must normalize to the bill or income name** they
  pay (uppercase, reference numbers stripped) so the recurring matcher links
  them and the detectors do not re-suggest what is already modeled.
- **The imported history must cover the whole instance window.** The
  recurring-instance projection looks back 365 days from today, and every bill
  occurrence in that window without a linked posting is a past-due obligation.
  The seed imports 370 days of paychecks and bill payments for exactly this
  reason; a shorter history puts months of spurious past-due rows on Cash Flow.
- **Mind the linker's amount-only fallback** (±7 days, within max(5%, $5) of
  the expected amount, same account): a posting with no payee match still links
  by amount alone. An unrelated row that lands near a bill's due date with a
  similar amount steals its link — the first draft's $86.43 grocery row silently
  "paid" the $91.33 piano bill. Keep the deliberately unpaid bill's amount clear
  of every other seeded amount.

## The rule that keeps this useful

**Whenever a screenshot surface gains a new empty state, the seed is updated in
the same change** — seed the data that fills it and extend
`seeded_demo_vault_populates_every_screenshot_surface` with an assertion that
proves it. A surface that can render "nothing here yet" against this vault is
a screenshot we cannot take, and the CI guard is what makes that visible before
a screenshot session rather than during one.

## Known limits

- There is no Money Inbox item kind for an upcoming (not yet due) bill; ADR 0058
  places upcoming bills in a Money Inbox insight (bead `92x7`) that has not
  shipped. The seed guarantees a past-due unconfirmed occurrence on Cash Flow
  and manual bills with upcoming due dates on the Bills view instead.
- Low-confidence-category inbox items need merchant-memory rules; none are
  seeded.
- Fixed (`personal-cfo-5ie.11`): the kernel's "today" used to be the raw UTC
  date, so late in the day a bill due *today* could already show as one day
  past due for any household west of UTC. It now resolves through
  `vault_metadata.household_timezone` (ADR 0021 §1), matching the policy
  every other calendar-boundary computation already followed.
- This vault's timezone is `America/Los_Angeles` (`personal-cfo-q329`),
  deliberately non-UTC — exercising the same household-timezone-aware code
  path a real, configured household hits, rather than the untouched `UTC`
  default that let the launch-evening symptom above ship unnoticed against
  an all-UTC test fleet in the first place. Set via `set_household_timezone`
  right after vault creation, before any bill or transaction is seeded; the
  fixed anchor (2026-09-01) is far enough in the past that the LA/UTC
  day-boundary shift never flips any past-due/current assertion the CI guard
  makes.
- Vault creation uses the everyday Argon2id profile (the same one every
  integration test pays); the CI guard derives the key three times (create,
  restore, reopen). The whole guard runs in about seven seconds.
