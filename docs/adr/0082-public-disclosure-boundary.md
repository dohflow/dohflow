# ADR 0082 — The public-disclosure boundary

- **Status:** Accepted (2026-09-16)
- **Tier:** Public — contributors must know these rules; that an internal
  repository exists is not itself a secret.
- **Bead:** `personal-cfo-0uxwt`
- **Decider:** Owner, 2026-09-16, in chat with the planning session
- **Related:** ADR 0062 (public-repo fork mechanics; the snapshot mechanism
  and export-ignore list this ADR builds on), ADR 0064 (task tracking leaves
  the repo — beads never public, restated here), ADR 0066 (business model —
  already public, **not reopened** by this ADR), ADR 0071 (public-repo
  contribution model — complements this one; ADR 0071 references this ADR
  when it is written)

## Context

`personal-cfo-qvn0x` and `personal-cfo-r36ck` move real development into the
now-public `dohflow/dohflow`. The bead graph itself stays private (ADR 0064:
`.beads/` never enters git tracking, and the public `.gitignore` already
proves it), but three other channels were left open and would leak roadmap
and business detail the moment normal work resumed in the public repo:

1. **`docs/adr/` is public.** Several ADR beads already queued for this
   program plan batch (0075 platform pricing, 0076 connector economics, 0078
   Sync server licensing/business terms, 0081 App Store legal posture) would
   publish per-subscriber cost tables, a revenue floor, affiliate commission
   terms, and entity-timing decisions the day their PR merges — not because
   anyone intended to publish them, but because nothing distinguished an
   engineering ADR from a business one.
2. **PR titles and bodies.** A bead ID (`personal-cfo-xxxxx`) reveals
   nothing on its own, but a bead's **title** or **acceptance criteria** can —
   and nothing before this ADR forbade pasting either into a public PR.
3. **The seventeen export-ignore'd paths** (`scripts/backup-beads.sh`,
   `backup-dolt.sh`, `mirror-backup.sh`, `restore-beads-local-files.sh`, their
   tests, the launchd plist, `docs/operations/beads-backup-and-restore.md`,
   `docs/operations/repo-mirrors.md`, and the `HANDOFF*`/`*_KICKOFF` docs) are
   hidden only from a `git archive` snapshot. Once development happens
   directly in the public clone rather than through periodic snapshots,
   anything tracked there is public — and `personal-cfo-r36ck`'s original
   plan assumed `backup-beads.sh` keeps living in the tree. Also found in the
   same pass: `CONTRIBUTING.md` told contributors to run `bd ready` and claim
   beads, then admitted `.beads/` would not exist in their clone and linked
   `docs/operations/beads-backup-and-restore.md` for "Bootstrap" — itself one
   of the seventeen paths, so a dead link in the public repo. No CI check
   enforces a bead ID anywhere (verified directly against `.github/`).

## Why not just gitignore every ADR

`README.md` cites ADRs six times and `SECURITY.md` four; the trust boundary
(0003), the CSP (0010), the id strategy (0013), the backup format (0024), the
license (0043), and the release channel (0068) are the *why* a contributor or
a security reviewer needs in order to change the code correctly, and CI
carries checks derived from several of them. Hiding the whole directory would
strip the design record from the code it governs and break those references.
Two tiers keep the engineering record public and the business record private
instead of losing either.

## Decisions

**1. Beads never public** (restated from ADR 0064). `personal-cfo-r36ck`'s
acceptance criteria gain an explicit check that the public clone tracks
nothing under `.beads/`. Bead titles and acceptance criteria never appear in
a public artifact (PR body, commit message, issue comment) — see decision 5.

**2. Two tiers.**

- **Public** — architecture, engineering, security, and process ADRs:
  0001–0074 as they stand today, plus 0069, 0071, 0072, 0073, 0077, 0079,
  0080, 0082 (this one), and sub-numbered amendments 0013-A, 0024-A, 0066-A.
- **Internal** — business ADRs: pricing, charging, billing, the per-subscriber
  cost model, the revenue floor, entity timing, affiliate economics, license
  deliberations before they are decided, App Store contingencies, and legal
  posture. Concretely: 0075 (the DohFlow Platform), 0078 (Sync server
  license/business terms), 0081 (App Store business posture). 0076 (connector
  strategy) **splits**: the public ADR carries the multi-provider decision and
  the D15 affiliate stance with its FTC-disclosure sentence; an internal
  companion carries commission terms and demand-test numbers. The LunchFlow
  feasibility research doc splits the same way — public except its
  affiliate-terms section.

**3. Where internal lives.** A new **private** repository, `dohflow/internal`
(the owner creates it — `gh repo create dohflow/internal --private`, and
verifies visibility with the GitHub API before anything is pushed, per
AGENTS.md §2). It holds `docs/adr/` for internal-tier ADRs, `docs/research/`
for internal research, and the destination layout for the seventeen relocated
operations/agent files (the files themselves move during
`personal-cfo-r36ck`, not this bead — see decision 6). **One ADR number
sequence spans both repositories.** The public `docs/adr/README.md` (created
by this bead) lists every assigned number with its tier; an internal number
appears as `0075 — internal` with no title. When an internal decision ships
as a user-visible feature, a **public stub ADR** is written at that same
number, stating the accepted decision in the words the site already uses
("Sync is a paid subscription; current price on `/pricing`") and nothing
more — never the internal reasoning, numbers, or alternatives considered.

**4. The tier check is mechanical, but as an index, not per-file churn.**
Every *new* ADR's front matter states its tier as one of the header fields
(see the template this ADR itself uses above: `- **Tier:** Public` /
`Internal`). The sixty-three ADRs that predate this one are **not**
retroactively stamped with a tier line — every one of them is already public
tier (nothing business-shaped has ever been written into this repository's
`docs/adr/`; the four business-tier numbers assigned so far, 0075/0076
(split)/0078/0081, are all still unwritten beads), so a header on each would
say the same uniform thing sixty-three times. `docs/adr/README.md`'s tiered
index is the single, machine-readable source of truth for tier, both for the
existing files and for the numbers not yet written; a future ADR's own
front-matter `Tier:` field is what an agent checks *before* deciding which
repository to write it into (decision 3 above), and the index is updated in
the same PR/commit that adds the ADR. `docs/architecture/definition-of-done.md`
and the ADR-bead acceptance template both gain "tier decided and recorded
before the file is written." `bd remember` carries the rule for agent
sessions (decision 9 of this bead's acceptance criteria).

A CI tripwire (`scripts/adr-tier-check.sh`, wired into the existing
`security-scan` job so it rides on an already-required status check rather
than needing a new one added to the branch-protection ruleset) fails the
build on a fixed phrase list — `$/month`, `per month`, `Stripe`, `price`,
`pricing`, `commission`, `revenue`, `cost table`, `subscriber`, `invoice` —
appearing anywhere under `docs/adr/` or `docs/research/` in the public repo,
with a documented allowlist for ADR 0066 (already public, already discusses
the business model in the abstract) and any public stub ADR.

**5. The PR and commit rule.** Agent and owner PRs carry the bead ID in the
title, followed by a description of the **change** — never a bead's title
verbatim when that title names an unshipped product, price, or plan. No
acceptance criteria or bead notes are pasted into a public PR body; a review
PASS comment cites the bead **by ID only**. Outside contributors need **no**
bead ID at all — they cite the GitHub issue instead (ADR 0071's D8). No CI
check demands a bead ID anywhere (verified; none is added by this ADR
either). At merge, the maintainer's bead gains a note with the PR URL, and
the merge commit itself carries the bead ID — the linkage ADR 0071 already
established. `.github/PULL_REQUEST_TEMPLATE.md` and `CONTRIBUTING.md` are
rewritten so the contributor path no longer tells people to run `bd ready` or
links the now-relocated bootstrap doc; the `implement-bead` and `review-pr`
skills gain the same rule.

**6. The seventeen export-ignore'd paths relocate during `personal-cfo-r36ck`,
not this bead.** They move to `dohflow/internal` as that bead's first commit
in the new public clone. `scripts/git-hooks/pre-push` changes now, in this
ADR's own bead, to call the mirror-backup script from a **configurable**
local checkout of `dohflow/internal` (`DOHFLOW_INTERNAL_DIR`, defaulting to
the documented sibling path `../dohflow-internal`) instead of the in-tree
`./scripts/backup-beads.sh`, and to skip with **one** warning line — not
silently — when that checkout isn't found, so a maintainer notices a
misconfigured path while a contributor clone (which will never have it) sees
the same single line once per push rather than nothing. `.gitattributes`
drops the seventeen `export-ignore` entries in the same relocation commit
that moves the files — not in this bead, since the files haven't moved yet.

**7. The site is unaffected.** `dohflow-site` stays public; `check-claims`
remains its gate; the site never carries internal detail before a feature
ships (decision 3's public-stub mechanism is exactly how a shipped decision
reaches the site's copy).

**8. D8 is unchanged.** GitHub Issues remain the public tracker for
community bugs and features; a curated `/roadmap` page
(`personal-cfo-sygn`) is the later, separate vehicle for surfacing selected
roadmap items. *How* that page is curated is not decided here.

## Rejected alternatives

- **Gitignore every ADR.** Breaks `README.md`/`SECURITY.md`'s own references
  and CI's ADR-derived checks, and strips the "why" a contributor needs to
  change the code correctly.
- **One public repo with redacted business ADRs.** Redaction drifts silently
  over time, and a CI tripwire scanning for business phrases cannot see
  *intent* — it can only catch phrases, which a redacted-in-place document
  would by construction have already removed, leaving the tripwire unable to
  prove the redaction stayed complete.
- **Keep business ADRs only in beads/memories, never as a document.** No
  reviewable record — violates the foundation-first discipline (AGENTS.md
  §1A): an architecturally significant decision needs a written, Accepted
  ADR, not just a bead description or a `bd remember` entry.

## Revisit if

- A business decision becomes load-bearing for contributors trying to build
  or extend the app — the fix is a public stub ADR at that point, never
  publishing the internal document's actual text.
- `dohflow/internal` needs a second maintainer — the access model for that
  repository isn't decided here.
- The CI tripwire's phrase list produces false positives that block a
  legitimate engineering ADR — tune the list; never remove the check itself.
