# ADR 0043 — OSS license: AGPL-3.0-only, contributions under DCO (amended 2026-09-02: CLA)

- Status: Accepted
- Date: 2026-07-04
- Bead: personal-cfo-fkt5 (public OSS fork + repo hygiene); gate: personal-cfo-2owr (Launch)
- Related: ADR 0040 (MVP release criteria), the OSS Strategy epic, the planned paid
  cloud tier (future offering, not yet a bead)
- Decider: recommendation delegated by the owner; adopted as the working decision
  pending the fkt5 sign-off step

## Context

The Launch gate requires a public OSS repository, and fkt5 explicitly calls the
license "owner call: AGPL/MIT/Apache — affects the future cloud tier". The owner
delegated the recommendation. The relevant facts:

- The product is a **local-first, fully free desktop app**. There is no hosted
  component today.
- The owner plans a **paid cloud tier** later (sync/hosted convenience around the
  same core). The license choice must not hand that business to somebody else.
- The positioning is **genuine open source** — an OSI-approved license, not
  source-available theater.
- The owner is, today, the **sole copyright holder** of every line in the repo.
  That fact is an asset: whoever holds all the copyright can offer the same code
  under any additional license at any time.

## Decision

**License the public repository under AGPL-3.0-only.** Accept outside
contributions under the **Developer Certificate of Origin (DCO)** — a
`Signed-off-by:` line on commits — not a CLA.

### Why AGPL-3.0

- **The free app stays fully free.** AGPL is a strong-copyleft, OSI-approved
  license; users get the full four freedoms, and the local-first desktop app is
  unaffected in daily use — AGPL costs a *user* nothing.
- **The network clause is the moat.** AGPL §13 extends copyleft to network use:
  a third party offering a hosted/cloud version of Personal CFO must offer its
  modified source to those users. Nobody can take the code, bolt on a closed sync
  service, and sell the owner's own future product back to the market. MIT/Apache
  would allow exactly that.
- **Dual-licensing stays available to the owner.** As sole copyright holder today,
  the owner can ship his own cloud tier under a commercial license (or keep
  cloud-side additions proprietary) while the public repo remains AGPL. Copyleft
  binds licensees, not the copyright holder.

### Why `-only` (not `-or-later`)

`AGPL-3.0-only` pins the terms to a text that exists and has been read. The
`-or-later` variant delegates future terms to whatever the FSF publishes as v4;
for a finance app whose trust story is precision, unbounded future terms are the
wrong default. Relicensing to a hypothetical later version stays possible the
ordinary way (copyright-holder decision + contributor consent).

### Contributions: DCO, not CLA *(superseded by the 2026-09-02 addendum below)*

- Contributors add `Signed-off-by:` (DCO 1.1) attesting they may submit the work;
  a CI check enforces the line. No paperwork, no signature portal, no asymmetric
  rights-grab — consistent with the genuine-OSS positioning, and what the Linux
  kernel and most CNCF projects use.
- **Documented consequence, accepted deliberately:** with DCO, contributors keep
  their copyright. Once *significant* outside contributions land, relicensing the
  combined work (including future dual-licensing of *contributed* code into the
  proprietary cloud tier) requires those contributors' consent. That is
  acceptable: the near-term contributor pool is expected to be small, the owner's
  own code dominates, and the cloud tier can be built as owner-authored code.
  **Revisit a CLA only if dual-licensing pressure becomes real** (e.g. a partner
  wants a commercial license of the whole repo) — and weigh the chilling effect
  honestly at that point.

## Consequences

### Positive

- Hosted closed-source forks are off the table; the future paid cloud tier
  competes only with AGPL-compliant (source-published) offerings.
- Clean OSS credibility: OSI-approved license + DCO is a familiar, low-friction
  combination.
- The owner's dual-licensing option is preserved at zero process cost today.

### Negative

- AGPL scares away some corporate users/contributors (many companies ban AGPL
  dependencies outright). Accepted: this is an end-user desktop app, not a library
  hunting for embedding.
- DCO means future relicensing needs contributor consent once outside code is
  significant (see above — accepted, revisit-trigger documented).
- AGPL compliance questions (e.g. "does my internal fork count as network use?")
  generate support noise. Mitigation: a short LICENSE-FAQ in the fkt5 fork work.

## Rejected alternatives

- **MIT / Apache-2.0** — maximum adoption, zero cloud protection. Any vendor could
  ship a closed hosted Personal CFO tomorrow; directly undercuts the planned paid
  tier. Apache's patent grant is nice but doesn't address the actual risk.
- **BSL / FSL (source-available with delayed-open or use restrictions)** — not
  OSI-approved; conflicts with the genuine-OSS positioning fkt5 stakes out, and
  the eventual-conversion machinery is complexity this project doesn't need when
  AGPL already blocks the hosted-fork scenario.
- **GPL-3.0 (without the network clause)** — copyleft for distribution only; a
  hosted service never "distributes", so the cloud moat evaporates. AGPL exists
  precisely to close this gap.

## Addendum (2026-09-02): contributions under a CLA, not DCO

The "revisit if" trigger below fired **before the public fork**: the owner
committed to building a cloud-hosted subscription tier in parallel with the
OSS launch (2026-09-01 feedback, decision bead `personal-cfo-hbg4f`). With a
DCO, the first outside contribution to core would permanently foreclose
shipping that code in the cloud tier under proprietary terms; adopting a CLA
*now*, while the owner is still the sole author, costs nothing and closes
nothing.

**Decision (owner, 2026-09-02): AGPL-3.0-only stays; contributions are
accepted under a Contributor License Agreement instead of the DCO.**

- A standard individual CLA (license grant to the project owner, contributor
  keeps their own copyright and a broad license back) enforced by an
  automated signing bot (e.g. cla-assistant) — one click on a contributor's
  first PR, no paperwork after that.
- The cloud tier is **still** architected as a separate platform consuming
  the open core through its public interfaces — not because the CLA requires
  it, but for tenant isolation and because it keeps the fully-open-cloud
  (Plausible-style) option alive.
- The §"Contributions: DCO, not CLA" section above is retained for the
  record; its accepted-consequence paragraph is the reasoning this addendum
  reverses. Decision brief with case studies: bead `personal-cfo-hbg4f`.

Business model recorded in ADR 0066 (2026-09-05): free local app forever,
paid services around it; AGPL-3.0-only + CLA reaffirmed.

## Revisit if

- ~~Dual-licensing pressure becomes real (commercial license requests, or the cloud
  tier needs to absorb significant contributed code) → evaluate a CLA then.~~
  *(Fired 2026-09-02 — see addendum.)*
- The contributor base or an ecosystem partner makes AGPL a demonstrable adoption
  blocker for the *desktop app itself* (not for embedding — that's by design).

## Implementation notes

- **The LICENSE file lands in the fkt5 fork work, not this PR.** This ADR records
  the decision; fkt5 adds `LICENSE` (AGPL-3.0-only text), the CLA document +
  signing-bot setup (2026-09-02 addendum), and flips the package metadata
  (`license = "LicenseRef-Proprietary"` in the Cargo manifests, `package.json`)
  to `AGPL-3.0-only` at fork time.
- SPDX identifier to use everywhere: `AGPL-3.0-only`.
