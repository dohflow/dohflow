# ADR 0085 — The open-core boundary

- **Status:** Accepted (owner decision, 2026-10-18)
- **Tier:** Public
- **Date:** 2026-10-18
- **Bead:** `personal-cfo-915.8` (this document); decision recorded on
  `personal-cfo-915.6`
- **Decider:** Owner, 2026-10-18, in the decisions session
- **Related:** [ADR 0043](0043-oss-license.md) (AGPL-3.0-only plus the CLA —
  the CLA is what lets the copyright holder ship proprietary code built on the
  core), [ADR 0066](0066-business-model-free-app-paid-services.md) §1–§2
  (free local app, paid services on a separate platform),
  [ADR 0066-A](0066-A-dohflow-endpoints-pinned-credential-not-unlock.md) §4
  and §6 (a credential, never a local unlock),
  [ADR 0074](0074-dohflow-sync-architecture.md) (Sync architecture),
  [ADR 0077](0077-wire-formats.md) (the published wire format),
  [ADR 0082](0082-public-disclosure-boundary.md) (public/internal tiers)

This ADR does not decide the license of the DohFlow Sync server; that is ADR
0078 (`personal-cfo-quhmv`) and stays an open owner decision.

## Context

ADR 0043 and ADR 0066 fix the license (AGPL-3.0-only plus the CLA) and the
promise (the local app is free forever, paid offerings are services on a
separate platform). Neither says which *components* are open and which are
not, where the non-open code and documents live, or how the Sync client
reaches a desktop user. Four pieces of work were waiting on those answers: the
Sync client (`personal-cfo-elciw`), the iOS cross-compile CI guard
(`personal-cfo-md8h2.7`), the mobile architecture decision
(`personal-cfo-iuxhq`) and the Sync server license (`personal-cfo-quhmv`).

Two things made the question concrete on 2026-10-18. A proprietary iOS app is
planned, and it need not be open source. And a document about that app
(the iOS kernel spike write-up) was about to land in this public repository,
which showed that ADR 0082 sorted *business* decisions into the internal tier
but said nothing about *engineering* documents for a product that is itself
proprietary.

## Decision

### 1. Component map

**Open, under AGPL-3.0-only (this repository):**

- the desktop app and the Rust kernel;
- the *core extensions* shipped with the app;
- the Sync foundations that live in the kernel: the operation log (v2),
  deterministic replay, snapshots and envelope crypto;
- the published wire format (ADR 0077);
- the desktop Sync client (see decision 3).

**Proprietary:**

- the iOS app and its documentation;
- the DohFlow Sync server, its accounts and billing;
- the marketplace bridge;
- extensions that are not core extensions.

The line is "what runs on the user's machine as part of the free local app is
open; the operated service and the separate products around it are not." This
restates ADR 0066 §1–§2 as a component list and adds nothing to the free-forever
promise or to its stated boundary.

### 2. Repository map

- **Proprietary documents** live in the private `dohflow/internal` repository
  (ADR 0082, decision 3), under `docs/adr/` or `docs/research/`.
- **Proprietary code** lives in private repositories. Their names are not
  fixed by this ADR.
- **The private iOS app consumes the public kernel as a git dependency pinned
  to an exact commit.** The copyright holder ships it under the holder's own
  rights; the CLA (ADR 0043 addendum) is what makes that true for
  contributed code. Moving the pin is a deliberate commit in the private
  repository, never a floating reference.
- **This repository never contains** proprietary source, proprietary
  documentation, or a build step that needs either.

### 3. The desktop Sync client stays open

The Sync client is part of the AGPL desktop app. It is **off** until the user
signs in to a DohFlow Sync account. What is sold is the operated service:
the server, the account and support. A payment is an endpoint credential
presented to a service, **never a local unlock** (ADR 0066 §2, ADR 0066-A §4):
the app runs the same local code whether an account is active, lapsed or
absent. This matches ADR 0066-A as written, so no addendum to it is needed.

Consequences stated here so downstream beads do not re-derive them:

- There is **no extension host on the Sync critical path**. The extension
  host is built later, around leaf features such as budgeting, and Sync does not wait for it.
- **On iOS, Sync is compiled into the proprietary app.** The open client and
  the proprietary iOS client both speak the published wire format (ADR 0077).
- Third parties may build interoperable servers or unofficial sync
  (for example through git) against the published format. ADR 0066-A §7
  already says openness is not a server-publication decision; this ADR does
  not change that.

### 4. ADR 0082's tier rule is extended

ADR 0082 decision 2 put *business* ADRs in the internal tier. This ADR adds
a second trigger: **documentation about a proprietary product is internal
tier even when its content is engineering.** The proprietary products are
those listed in decision 1: the iOS app, the Sync server, accounts and
billing, the marketplace bridge, and non-core extensions. An ADR or research
document about one of them is written in `dohflow/internal`, and a public
stub ADR at the same number is written only when the decision ships and only
as ADR 0082 decision 3 describes.

Documentation about the open pieces they touch stays public: the kernel's
iOS cross-compile property is carried here by a public CI check
(`personal-cfo-md8h2.7`), not by iOS app documents. Under this rule the
mobile architecture decision (ADR 0080) is internal tier; the index in
`docs/adr/README.md` is updated to match, and ADR 0082's accepted text, which
listed 0080 as public before this rule existed, is not rewritten.

### 5. Forward constraint for the marketplace ADR

The premium marketplace is designed in a later milestone ADR, not here. The
owner's constraint on that ADR: no code or architecture that lets someone
technically bypass access to paid extensions may land in this repository. ADR
0066 §2 and ADR 0066-A §6 already point the way: the paid gate sits
**server-side**. The registry hands a paid package only to an entitled
account credential, and an acquired extension runs with no local payment
check. The open code then holds no unlock logic to work around. The
AGPL extension exception third-party extensions will need is also that ADR's
work, not this one's.

## Consequences

**Positive**

- Contributors, packagers and reviewers can tell which code is open, and no
  open component depends on a proprietary one.
- The Sync client is reviewable and forkable like the rest of the app, which
  keeps the AGPL desktop download whole: the official build carries no closed
  module.
- The Sync work (`personal-cfo-elciw`) and the iOS CI guard
  (`personal-cfo-md8h2.7`) have a settled location: both in this repository.
- The iOS app's documents no longer have a public home to leak into.

**Negative**

- A third party can build an alternative Sync server or client against the
  published format. That is intended; the operated service is the product,
  not exclusive access to the protocol.
- Two repositories carry related decisions. A contributor reading a public ADR
  may find a number marked internal in the index with no file. That is the
  ADR 0082 trade, accepted there.
- The private iOS app trails the public kernel by whatever its pinned commit
  is. A kernel change that the iOS app needs is a public commit first.

## Rejected alternatives

- **A proprietary Sync module compiled only into the official signed build**
  (the pattern of editions that ship a closed superset). It is the fastest
  route, but the official download would no longer be the AGPL source, the
  open-source claim would need a footnote, and a local build could differ from
  a released one in exactly the code that handles the user's data. Rejected:
  it conflicts with ADR 0066's free-forever local app and with ADR 0066-A's
  rule that payment is a credential, never a local unlock.
- **Sync as the first extension on a signed-extension host.** Cleanest in the
  abstract, but it puts the extension host on Sync's critical path, so Sync
  could not ship until the host existed, and the host's design would be
  decided by Sync's needs. Rejected: the extension host is built later around
  leaf features.
- **Putting the proprietary iOS documents here with a "do not publish" label.**
  A label is not a control. ADR 0082 rejected redaction in place for the same
  reason.

## Revisit if

- A component moves between open and proprietary. That is a new ADR (and, for
  a move toward proprietary, one the free-forever promise must be checked
  against); it is not an edit to the map above.
- The extension host or marketplace ADR needs an exception to decision 3's "no
  extension host on the Sync critical path".
- ADR 0078 decides the server license in a way that changes decision 2's
  repository map.
- A private code repository is created or named: record its name here or in a
  dohflow/internal index.

## Linked beads

- `personal-cfo-915.6` — the decision this records (closes when this merges)
- `personal-cfo-915.8` — this document
- `personal-cfo-elciw` — Sync client engine (stays in this repository)
- `personal-cfo-md8h2.7` — iOS cross-compile CI check of the kernel (public CI)
- `personal-cfo-iuxhq` — mobile architecture (internal tier under decision 4)
- `personal-cfo-quhmv` — Sync server license (ADR 0078; not decided here)
