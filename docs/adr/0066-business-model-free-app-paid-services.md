# ADR 0066 — Business model: free local app forever + paid services around it (the Obsidian model)

- **Status:** Accepted (2026-09-05)
- **Bead:** `personal-cfo-915.4`
- **Decider:** Owner, 2026-09-05, in chat, after a full closed-source vs
  source-available vs AGPL license-strategy review
- **Related:** ADR 0043 (OSS license: AGPL-3.0-only + CLA), bead `personal-cfo-hbg4f`
  (the license/cloud-tier decision this ADR's business-model half completes),
  `TRADEMARK.md`, the DohFlow site pricing/index/contribute pages (site copy
  bead `personal-cfo-n76x.23` depends on this ADR; owner approval in `n76x.6`
  reviews the reconciled copy)

## Context

`personal-cfo-hbg4f` settled the license question: AGPL-3.0-only stays, and
contributions are accepted under a CLA rather than a DCO so the owner keeps the
unilateral right to ship a future proprietary cloud tier (ADR 0043's 2026-09-02
addendum). That decision fixed the *license*. It did not yet fix the *business
model* — what is free, what is paid, and how the site is allowed to talk about
either. Two things forced that second decision:

- The site copy (`index.astro`, `pricing.astro`, `contribute.astro`) had
  drifted into promising "no paid tier" and "no pro build" outright, which
  contradicts the planned paid cloud tier and would have to be walked back
  after launch — the one scenario ADR 0043 was written specifically to avoid.
- Nothing has shipped publicly yet, so this is the only point in the project's
  life where the business model is free to state cleanly. Once the app is
  public, changing the free/paid boundary is the kind of move that reliably
  spawns hostile forks (Elastic, HashiCorp, Redis all did this after
  launch, not before).

## Decision

**DohFlow follows the Obsidian model.** The local desktop app is free forever,
with no feature gating and no donation nag screens. Revenue comes from paid
*services* built around that free core, shipped as a separate platform.

### 1. The free-forever promise, verbatim

> The self-hosted local app, its full feature set at the time of each release,
> and the AGPL-3.0 license on its source, are free forever. There is no
> feature gating and no donation checks in the local app.

**Boundary — what this does not promise:** it does not promise that every
*future* capability is free. Sync, a hosted/cloud vault, mobile, and any future
tier are separate products this promise does not cover. "Free forever" binds
the local app as it exists at each release; it does not bind products that do
not exist yet.

### 2. Paid offerings are services around the core, on a separate platform

Sync, cloud vault, mobile, hosted access, and future tiers are built as a
**separate platform** that consumes the free local core through its public
interfaces — not features unlocked inside the local app. This was already the
architectural direction from `hbg4f` (kept "for tenant isolation and because it
keeps the fully-open-cloud option alive," independent of what the license
requires); this ADR makes it the business-model default, not just an
architecture preference. Practical effect: nothing in the local app's code
paths checks a license key, a subscription, or an account to unlock behavior.
Paid functionality lives in a product the local app talks to, not in a branch
the local app executes.

### 3. AGPL-3.0-only + CLA, reaffirmed — no relitigation

The license question is **closed**. AGPL-3.0-only stays, contributions arrive
under the CLA (ADR 0043, as amended 2026-09-02). This ADR adds a
**no-relitigation rule**: absent one of the "Revisit if" triggers already
recorded in ADR 0043, this decision is not reopened after launch.

**Rationale for the rule, not just the decision:** nothing has been
distributed yet, so changing the license or the free/paid boundary today costs
nothing — there are no users to disappoint and no forks to provoke. A license
or business-model walk-back *after* launch is the one event that reliably
spawns a hostile fork of a previously-open project; Elastic, HashiCorp, and
Redis all relicensed post-launch and all got a community fork in response
(OpenSearch, OpenTofu, Valkey). AGPL's network clause already makes a
well-funded competitor's cheapest path "build your own" rather than "fork
DohFlow and out-compete it," which is why funded companies routinely do the
former — the license is doing its job. There is no new information a
relitigation would surface; there is only the cost of looking indecisive to a
community that was told the terms up front.

### 4. Marketing rule: license placement

This rule binds the site and the README.

- **Lead with:** local-first, your data stays on your device, free forever,
  paid sync when you want it.
- **The license is a footer badge, one pricing-FAQ line, and the `/license`
  page.** It is never a hero section, never the headline pitch.
- **"Fork it, modify it, sell it, host it" language lives only on `/license`
  and in `TRADEMARK.md`.** It does not appear on the home page, the pricing
  page, or the contribute page as an invitation — those pages sell the product
  and the trust story, not the license terms. (`TRADEMARK.md`'s existing "Fork
  it, modify it, sell it, host it." language is correct and unaffected — it
  is exactly where this rule says that language belongs.)
- Site pages must not promise "no paid tier" as a blanket statement. The
  accurate claim is narrower: *the local app* has no paid tier. A future
  hosted/sync product is allowed to be paid, and the copy should say so rather
  than implying otherwise and correcting it later.

### 5. The Obsidian precedent, and why finance needs a stronger trust story

**Precedent:** Obsidian ships a fully free, fully capable local-first note app
with no feature gating, and sells Sync and Publish as separate paid services
around it. Users trust the local app because nothing about using it depends on
paying, and the company's revenue depends on the paid services being good
enough to want, not on withholding local functionality. That alignment — free
app competes on merit, paid services compete on merit, neither subsidizes
the other by crippling it — is what this ADR adopts.

**Why finance needs more than that:** a notes app losing your trust costs you
inconvenience. A finance app losing your trust costs you your transaction
history, your categorization work, or your confidence that the numbers are
right. Obsidian's trust lever is that notes are plain Markdown files a user
can read and move without the app. DohFlow's vault is an encrypted SQLite
database, not plain files, so the equivalent lever does not exist by
accident — it has to be built deliberately. That work is scoped as its own
bead, **`personal-cfo-klr.4`** ("documented vault on-disk format +
first-class Export Everything — the Obsidian trust lever"): a public,
versioned spec of the on-disk format (encryption envelope, schema, backup
format) so a user with their passphrase can recover their data without the
DohFlow binary, plus an Export Everything action reachable in one click that
produces a documented, round-trippable bundle. This ADR does not restate
klr.4's design; it names the commitment the business model depends on and
points at the bead that delivers it.

Community labor is explicitly **not** the reason DohFlow is open source —
`hbg4f` and ADR 0043 already establish that the reasons are distribution
(Show HN, GitHub, r/selfhosted) and trust. This ADR extends that trust
argument: for a finance app specifically, trust is carried by data
portability (klr.4) and license terms together, not by the license alone.
Community energy is directed at a future plugin ecosystem instead of core
contributions to the ledger/vault/forecast engine — Obsidian's plugin
community built thousands of plugins and never touched the app itself, which
is the model. Whether and how DohFlow exposes extension points is a separate,
not-yet-decided question tracked in **`personal-cfo-915.5`** (post-1.0); this
ADR only fixes where community energy is *directed*, not the plugin API's
design.

## Consequences

### Positive

- **Binds `n76x.23` (site copy), `n76x.6` (owner copy approval), and the
  README** to the marketing rule in §4: one pricing-FAQ line, a footer badge,
  no "no paid tier" headline promises. `n76x.23` implements this directly
  against this ADR.
- **The local app never checks for payment or donation.** No license key, no
  subscription check, no donation nag, in any code path a fully offline user
  hits. This is the literal, testable form of "no feature gating," and the
  free/paid boundary cannot drift feature-by-feature because of it — paid
  functionality is architecturally required to live in a separate platform.
- **A future paid service must not degrade the local app.** Shipping sync,
  cloud vault, mobile, or any other paid product must not remove, slow, or
  hold back a capability the free local app already has — the free app does
  not become a funnel for the paid one.
- The no-relitigation rule removes an entire category of future
  bikeshedding: once shipped, "should we change the license" or "should we
  paywall a local feature" is off the table absent a named trigger.

### Negative

- Building every paid capability as a genuinely separate platform (rather than
  a flag in the local app) is more engineering work up front than gating a
  feature behind a license check would be. Accepted: it is also what makes the
  free-forever promise credible rather than a marketing claim that erodes over
  time.
- A strict reading of "no feature gating" means the team cannot later relieve
  local-app pressure by moving a shipped local feature behind a paywall — that
  would break the literal promise in §1. Any such move requires walking back
  this ADR explicitly, with the community-trust cost that implies, not a
  quiet feature relocation.
- **What changes if the owner ever wants to relicense: nothing is technically
  blocked.** As sole copyright holder plus the CLA (ADR 0043), the owner
  retains the unilateral right to relicense at any time. This ADR's
  no-relitigation rule (§3) is a policy commitment the owner made to future
  users, not a legal constraint — the rule says *don't*, absent one of the
  named triggers, precisely because the capability to do it anyway is what
  makes the restraint meaningful rather than automatic.

## Revisit if

- One of ADR 0043's own "Revisit if" triggers fires (AGPL becomes a
  demonstrable adoption blocker for the desktop app itself, or dual-licensing
  pressure requires a CLA change beyond what was already decided).
- A capability originally planned as a local-app feature turns out to require
  ongoing server-side cost to operate (e.g. it depends on a paid third-party
  API) — that is a "which side of the platform boundary does this live on"
  question, not a reason to gate it locally.

## Implementation notes

- See ADR 0074 for the Sync architecture; this ADR's free-local-app and separate-service boundary remains unchanged.
- ADR 0043 gains a one-line pointer to this ADR, appended after its
  2026-09-02 CLA addendum.
- Site copy: `personal-cfo-n76x.23` reconciles `index.astro`, `pricing.astro`,
  and `contribute.astro` in the `dohflow-site` repo against §4 of this ADR;
  `personal-cfo-n76x.6` is the owner approval pass over the reconciled copy.
- No code changes follow from this ADR by itself — it is a documentation and
  copy decision. The architectural consequence (paid features live in a
  separate platform) is already the direction `hbg4f` set; this ADR does not
  ask for new backend work.
