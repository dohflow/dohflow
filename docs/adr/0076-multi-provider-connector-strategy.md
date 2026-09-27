# ADR 0076 — Multi-provider connector strategy

- **Status:** Accepted (2026-09-27)
- **Tier:** Public + Internal split (ADR 0082). This file carries the
  multi-provider decisions and the affiliate stance with its FTC disclosure
  sentence, which is public by law. The affiliate program's payout terms live
  in the private `dohflow/internal` repository
  (`docs/research/lunchflow-affiliate.md`), never here.
- **Bead:** `personal-cfo-m0kgx`
- **Decider:** Owner. D15 (affiliate stance) was decided 2026-09-18 on
  `personal-cfo-hdk50`; D28 (Plaid) on 2026-09-18 on this bead; picker order
  and the `source_type` policy on 2026-09-27, in the implementation session.
- **Builds on:** ADR 0004 §2–§3 (credential tiers and relay modes), ADR 0060
  and its 2026-09-02 addendum (SimpleFIN first; one link/map surface; the
  disclosure panel), ADR 0066 (free local app, paid services around it),
  ADR 0015 (the connector registry), ADR 0010 (window capabilities and the
  scoped opener grant), ADR 0082 (public/internal disclosure boundary), and
  the program plan's invariant 6.
- **Research:** `docs/research/simplefin-feasibility.md`,
  `docs/research/lunchflow-feasibility.md`

## Context

ADR 0060 chose SimpleFIN as the first bank connection. Its 2026-09-02
addendum fixed two things every later provider inherits. First, the app has
exactly one link/map surface: the Settings Connections card, which onboarding
reuses. Second, a disclosure panel appears **before** any token is pasted. It
says the provider is an independent third party, that it handles the user's
bank credentials under its own terms, that it costs money paid to it, and
that it is optional. ADR 0004 defines the credential tiers: user-token and
bring-your-own credential (§2), and the relay modes (manual, self-hosted,
managed; §3). ADR 0066 fixes the business model: the local app is free
forever, and paid offerings are services around it. ADR 0015 fixes the
registry every adapter registers with.

None of them decides how a **second** provider enters the free app, who may
pay whom, or whether DohFlow may earn from a referral. LunchFlow
(`docs/research/lunchflow-feasibility.md`, GO) is the forcing case. It is a
user-token provider like SimpleFIN, it serves countries SimpleFIN doesn't,
and it runs an affiliate program.

The adapter (`personal-cfo-r2pow`) and the picker (`personal-cfo-dto2j`) are
both blocked on this ADR. Neither may settle these questions implicitly
(AGENTS.md §1A).

## Decision

### 1. Invariant 6: the free app ships only providers the user pays directly

The free app ships only providers the user pays directly, under the user's
own account. That means ADR 0004 §2's tiers: **user-token** and
**BYO-credential**. The free app has no project secret, no relay, and no
DohFlow-brokered provider. This is the program plan's invariant 6, and this
ADR accepts it.

Server-secret aggregators (Plaid, MX, Mastercard Open Banking) need ADR 0004
§3's relay. For **Plaid** specifically, the owner decided on 2026-09-18 (D28)
that Plaid is offered **only** as part of the paid DohFlow Sync subscription,
through the managed relay (ADR 0004 §3 mode 3), once Sync can be bought. It is
never in the free app. Entitlement is an active Sync subscription, checked at
the relay, never an unlock inside the local app (ADR 0066 §2). The build bead
is `personal-cfo-tpz9q`.

**BYO-Plaid is rejected.** Plaid Link needs a `client_secret` held by a
confidential server. Plaid's Limited Production access has no Item-count
trial, and its production review evaluates the *developer*, not each end
user. A "bring your own Plaid keys" path would put a server secret on the
desktop or make each user a Plaid developer. ADR 0004 §2 already declined to
commit to it; this ADR closes it.

**The self-hosted relay (`personal-cfo-lb0x`) stays open.** Per ADR 0004 §3,
the managed relay must never be the only path to a relay-tier provider. The
self-hosted relay ships alongside it. That promise only holds if the relay can
route to at least one provider a self-hoster can actually use (Teller BYO
today).

### 2. Provider entry gate: no adapter without a registry entry

An adapter ships only with its ADR 0015 registry entry (`ConnectorMetadata`,
implemented by `personal-cfo-5jjz`, merged as `2b68c3c`). The entry records:

- the credential tier;
- account types and regions;
- the cost model: payer, base cost and period, included connections, and the
  per-extra-connection cost with its own period;
- the cost-review date;
- the terms URL and its review date;
- the four-point disclosure text, written per provider;
- history-depth expectations;
- the `enabled` flag.

`register_connector!` will not accept an adapter without its entry. **New
adapters ship `enabled: false`.** `connector_link` refuses a disabled provider
before its `link` runs (ADR 0015 §6). A release bead flips the flag once
decision 3's conditions hold.

### 3. Ship conditions for a second (or later) provider

A provider's `enabled` flag may flip only when all of these hold:

- **(a) Multi-provider dedupe.** The same account reached through two
  providers never double-counts. This is `personal-cfo-6evt`'s provider-id
  layer on the `personal-cfo-yl5` dedupe pipeline, with "one account, two
  providers" as a test fixture. LunchFlow makes this concrete: it can also
  act as a SimpleFIN Bridge destination
  (`docs/research/lunchflow-feasibility.md` §f). A user could therefore reach
  one bank through the LunchFlow adapter and through the SimpleFIN adapter at
  the same time.
- **(b) The currency rule** (decision 7).
- **(c) Help and disclosure.** A help page for the provider exists. Where
  decision 5 applies, the affiliate disclosure is in place at every placement
  it names (`personal-cfo-wk0iv` for the site).
- **(d) Threat model.** `docs/security/threat-model.md` TB3 (sanctioned
  egress paths) and its threats table gain the provider's pinned endpoint. At
  the time of writing they name SimpleFIN only. This file is changed once per
  provider, by that provider's adapter bead.

### 4. The picker

The Connections card opens a **registry-fed provider picker** listing enabled
entries only. For each entry it shows the name, what the provider fetches,
what it costs and to whom, the terms link and last-reviewed date, and the
countries covered. Choosing a provider shows its four-point disclosure panel,
with text from the registry, then that provider's link form. With a single
enabled provider, the picker is skipped. Onboarding's connected branch reuses
the same picker and panel, so there is still exactly one link surface.
Vocabulary follows `personal-cfo-p7qc6`: "Connections" and "refresh", never
"sync". Implemented by `personal-cfo-dto2j`.

**Order rule (owner decision, 2026-09-27):** SimpleFIN first as the launch
provider, then every other enabled provider alphabetically by display name.
The registry carries no ordering or weighting field. Order is a fixed rule
over provider names, so no referral relationship can influence it, and
`dto2j` asserts the rule in a test.

### 5. D15: the affiliate stance is take and disclose

The owner decided on 2026-09-18 (D15, recorded on `personal-cfo-hdk50`) that
DohFlow **takes and discloses** a referral fee where a user-pay provider
offers one. LunchFlow is the first such provider. The program's terms stay in
the internal tier (see the Tier line above).

Because a fee may be earned, FTC 16 CFR Part 255 requires a clear and
conspicuous disclosure next to every referral link. The sentence, confirmed
final by the owner:

> DohFlow may earn a commission if you sign up for LunchFlow through the link above — this does not affect what LunchFlow charges you, and DohFlow works the same whether or not you use it.

It appears, adjacent to the referral URL, in each of these **placements**:

1. **The in-app picker.** It sits in that provider's disclosure panel, in the
   slot `dto2j` builds. That slot renders the sentence only when the provider's
   registry entry carries a referral, and nothing otherwise.
2. **The site's "Connecting a bank" help page** (`/help`), via
   `personal-cfo-wk0iv`.
3. **The provider's features page** on the site, via `wk0iv`. The site's
   existing SimpleFIN features page says DohFlow receives nothing from that
   provider. That stays true for SimpleFIN and must not be copied onto a
   provider with a referral.

**Rules that bind every placement:**

- **Referral URLs render as selectable text in the app, never as links.**
  ADR 0010's only opener grant is scoped to `https://dohflow.app/*` (its
  2026-09-06 addendum). Widening it to a provider host would be a new egress
  class. Routing a referral through a `dohflow.app` redirect to make it
  clickable is also rejected: that would launder a provider link through the
  scope granted for the project's own pages.
- **The referral relationship never changes picker order, defaults, or copy
  tone.** Order is decision 4's fixed rule. No provider is preselected. The
  referral is framed as an option (for LunchFlow: coverage outside the US),
  never as a nudge away from another provider.
- **Where the referral lives.** ADR 0015's `ConnectorMetadata` gains one
  optional field for this: the referral URL, plus the provider-specific FTC
  sentence above, written per provider for the same reason ADR 0015 §5 keeps
  every disclosure per provider. It is added by the first bead that registers
  a provider with a referral (`r2pow`). SimpleFIN's entry carries none.

### 6. The OFX/CSV zero-adapter path stays first-class

Manual entry and file import stay first-class (program-plan invariant 5; ADR
0060 §3). For any aggregator that exports files, the OFX/CSV route is the
first-class fallback. It is documented on `/help`, and how often it's used is
the demand signal that sizes future adapters. **An adapter is never required
to use a provider whose exports DohFlow can import.** LunchFlow's CSV and OFX
exports already round-trip through the existing importers
(`docs/research/lunchflow-feasibility.md` §f).

### 7. The currency rule

Multi-currency is a later milestone (`personal-cfo-d63`, `personal-cfo-rlx`,
`personal-cfo-k2u3` and `personal-cfo-il6n`). Until it ships, the mapping surface **refuses** to map a
provider account whose currency differs from the household's reporting
currency. The refusal copy names the limitation. Until then, the site does not
claim international coverage. Implemented by `personal-cfo-049p6`.

### 8. `source_type` tokens: one reviewed migration per adapter

`source_batches.source_type` is constrained by a database `CHECK` to a fixed
token set (`crates/db-worker/src/migrations.rs`, migration 16).
`ConnectorAdapter::id` must be one of those tokens.

**Owner decision, 2026-09-27: keep the `CHECK`.** Each new adapter's bead
ships an explicit, reviewed migration that widens it. SQLite can't alter a
`CHECK` in place, so the migration uses the column-swap pattern migration 36
already used for `accounts.subtype`: add a new column with the wider `CHECK`,
copy, drop the old column, rename. `source_type` has no index and is only
ever read by name, so the swap is safe. The migration carries the usual
down-migration and migration tests.

Rationale: the schema stays self-describing, the database itself rejects an
unknown token even if a code path bypasses the registry, and each widening is
a visible, reviewable diff. **Registry-only validation (dropping the `CHECK`)
is rejected** because it trades that database-level guarantee for saving one
small migration per provider, and providers are added rarely.

### 9. Identifiers are one name, forever

**adapter id == registry key == `source_type` token == crate directory
suffix** (`crates/connectors/<id>-adapter`). The id is stable forever: every
`source_batch` and `parser_run` row records it as provenance, so renaming a
provider's id would orphan its history.

## Rejected alternatives

- **Plaid first, or Plaid in the free app.** Plaid Link needs a
  `client_secret` on a confidential server, which the free app never has
  (invariant 6, ADR 0004 §1). BYO-Plaid is rejected for the same reason
  (decision 1).
- **In-house aggregation.** Building a DohFlow aggregator means per-bank
  integrations and screen-scraping liability. The FDX / CFPB §1033 track is
  watched instead (`personal-cfo-gqmx`); if it yields an individual-access
  path, that is a new tier, not an in-house aggregator.
- **Hardcoding a second provider into the Connections card** the way
  SimpleFIN is today. It duplicates the disclosure, forks the link surface
  ADR 0060's addendum made singular, and doesn't scale to a third provider.
  The registry-fed picker (decision 4) replaces it.
- **Folding provider costs into a DohFlow charge.** That would make DohFlow
  a broker of a provider it does not operate (invariant 6), and it would hide
  from the user what they actually pay each provider.
- **Undisclosed referrals.** Illegal under FTC 16 CFR Part 255, and
  corrosive to the trust story ADR 0066 depends on.
- **Registry-only `source_type` validation.** See decision 8.
- **Ordering the picker by anything a referral could influence.** See
  decision 4.

## Consequences

- `personal-cfo-5jjz` (merged) already implements decision 2's entry gate and
  the `enabled` refusal. Decision 5 adds one optional referral field to
  `ConnectorMetadata` when `r2pow` first needs it.
- `personal-cfo-dto2j` builds decision 4: the picker, the order rule with its
  test, the per-provider disclosure panel, the referral slot, and SimpleFIN
  de-hardcoded from the Connections card.
- `personal-cfo-r2pow` follows decisions 8–9. It adds the `lunchflow` token
  migration and its crate as `crates/connectors/lunchflow-adapter`, adds its
  TB3 threat-model row (decision 3d), and ships `enabled: false`.
- `personal-cfo-6evt` (on `yl5`) and `personal-cfo-049p6` are ship conditions
  for any second provider (decision 3).
- The site's SimpleFIN features page is generalized into a per-provider
  template, and each page states that provider's own relationship to DohFlow
  (`wk0iv`).

## Revisit if

- A provider critical to users offers neither a user-token/BYO flow nor an
  FDX individual-access path → ADR 0004's own revisit clause.
- CFPB §1033 / FDX yields an individual-access path → consider a direct FDX
  tier.
- An affiliate program's terms change materially, or forbid open-source
  referrers → re-decide D15 for that provider.
- LunchFlow removes API access from its consumer plan or changes its cost
  materially → fall back to the OFX/CSV path, following ADR 0060's own
  pattern for SimpleFIN.
