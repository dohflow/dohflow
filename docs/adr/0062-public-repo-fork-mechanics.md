# ADR 0062 — Public-repo fork mechanics: in-place flip via history rewrite, author identity, and pre-flip exposure

- **Status:** Accepted 2026-09-07; sections 1, 3 and 4 superseded by the
  2026-09-08 amendment below
- **Bead:** `personal-cfo-fkt5.2`
- **Related:** ADR 0064 (task tracking leaves the repo — forces the mechanics
  decision below), ADR 0067 (DohFlow rename policy — identifier and data
  directory unchanged, not re-litigated here), `personal-cfo-fkt5.11`
  (executes the rewrite this ADR authorizes; was blocking this bead, now
  depends on it per the sequencing decision below), `personal-cfo-fkt5.9`
  (fork-day hardening, including the email-privacy setting this ADR gates),
  `personal-cfo-fkt5.4` (owner-signed LICENSE-swap PR, blocked on this ADR
  being Accepted), `personal-cfo-o1nxk` item 8 (public-exposure sweep — the
  fuller version of this ADR's exposure decision), `personal-cfo-2owr`
  (Launch gate)

## Context

`personal-cfo-fkt5` (public OSS fork) needs answers to two questions before
the repository can go public: how does the private repo become the public
one (fork mechanics), and what identifier does the shipped app carry
(already answered — ADR 0067 keeps `ai.personalcfo.desktop` and the data
directory unchanged; this ADR does not revisit that). A third question
surfaced during preparation: what pre-flip content needs to be removed or
rewritten before the flip, since a visibility change on GitHub is permanent
and publishes the entire commit history at once.

**ADR 0064 already forces the mechanics answer.** `.beads/` — the committed
Beads export — is touched by 813 of 1,193 commits (68% of history) and
carries agent memories and financial detail. ADR 0064 decided to remove
`.beads/` from the repository and from history via `git filter-repo`. That
means "in-place flip" and "history rewrite" are the same operation on this
repository: there is no version of "flip in place" that leaves history
untouched, because the untouched history is exactly what carries the
exposure ADR 0064 was written to close.

**The rewrite turned out to need a second pass, discovered during dry
runs.** A path-based filter (removing `.beads/`) is necessary but not
sufficient. Scanning the rewritten history for the owner's real financial
values (from a screenshot of the running app, used to verify nothing
survived) found real dollar figures and initials in a frontend test file —
regression-test fixture data written against a real bug found through
dogfooding, not against `.beads/`. That leak was fixed at `HEAD` with
synthetic values (merged in PR #386) but the real values remained in five
historical blobs. **This ADR does not restate those values or initials —
they are recorded only in `fkt5.11`'s notes and its scratchpad replacement
rules, which are session-scoped and never committed.** The practical
consequence is architectural: the history rewrite must run a path filter
(`--path .beads/ --invert-paths`) and a value filter (`--replace-text` over
the specific real values) **in the same `git filter-repo` pass**, because a
second rewrite later costs a second force-push and a second window of risk.

The owner reviewed this in a 2026-09-07 planning session and made the
decisions below. They can be recorded without further owner input.

## Decision

### 1. Fork mechanics: in-place flip, executed as one combined rewrite

The repository stays the same GitHub repository — same identity, same
issues, same merged PRs (`#1`–`#386` as of this writing; GitHub addresses
issues and PRs by number, not by commit SHA, so a rewrite does not strand
them). What changes is its history: immediately before the visibility flip,
`fkt5.11` runs

```sh
git filter-repo --path .beads/ --invert-paths --replace-text <rules-file>
```

in one pass over `main`, after first deleting five stale remote branches
(§3) so fewer refs are rewritten and force-pushed. Every commit SHA changes
as a result. **Consequence for this and every other document:** a SHA
referenced in a bead note, a doc, or a commit message written before the
rewrite becomes a stale pointer afterward — true as history, not resolvable
against the rewritten repository. This is accepted as the cost of the
rewrite, not something to "fix" retroactively; new references written after
the rewrite point at post-rewrite SHAs.

**Rejected alternatives** (from the bead's original Q1): Option B (mailmap
+ a JSONL-rewrite into a fresh repository) is unnecessary — it would achieve
the same privacy outcome at the cost of abandoning the repository's
identity, issues, and PR history, which the combined-pass rewrite above
preserves without that cost. Option C (ship a snapshot, discard history
entirely) was rejected outright in the bead's own framing — it throws away
attribution and review history for no privacy benefit the combined pass
doesn't already provide.

**Timing is the whole point.** This is inexpensive only because the
repository is private with zero forks and zero stars and has never been
public — no clone exists, no SHA is referenced externally, and one
force-push to a remote only the owner uses costs nothing external. After the
flip, the identical operation breaks clones and permalinks and *publicly
documents what was removed*, which defeats the purpose. The rewrite must
complete before `fkt5.9` (the visibility flip).

### 2. Historical author identity: keep the Gmail author, do not rewrite it

As of 2026-09-08, `origin/main` carries 1,226 commits: 809 authored with
the owner's Gmail address, 396 authored by GitHub itself under the owner's
noreply identity (`chrisbustos@users.noreply.github.com` as author; GitHub
stamps its own `noreply@github.com` as committer on these — from
squash-merging PRs through the GitHub web UI/API, not commits the owner's
own machine produced), and 21 authored under a hostname-derived local
identity. This decision covers
the commits the owner's own machine produces: **every locally-authored
commit** (Gmail and `.local`) **keeps its author identity as-is.** No
mailmap, no rewrite to a GitHub noreply address, on this pass or any future
one. Future locally-authored commits also continue under the same Gmail
identity unless a separate decision changes that.

**Direct consequence, binding on `fkt5.9`:** GitHub's account setting
"Block command line pushes that expose my email" **must stay OFF.** That
setting rejects any push whose commits carry a non-noreply author address
with error `GH007` — since every locally-authored commit in this
repository's history, and every one going forward, uses the Gmail address
(the GitHub-generated noreply commits are unaffected; they already carry
the noreply identity by construction), turning it on would reject the very
next push, including the next agent PR. "Keep my email
addresses private" (a different, compatible setting — it only affects the
web UI's display and GitHub-generated commits) may still be enabled; that
one does not touch command-line pushes.

### 3. Branch cleanup precedes the rewrite

Five remote branches are deleted before the rewrite, so fewer refs need
force-pushing. Verified read-only against `origin` on 2026-09-07 — nothing
of value is lost:

- Three unmerged agent branches whose net effect is zero real change:
  `agent/4d8.25-recurring-ux-batch4` (2 commits, only
  `.beads/issues.jsonl`), `agent/4d8.25-statement-estimator-batch2` (1
  commit, only `.beads/issues.jsonl`), `agent/4d8.25.27-interest-dedupe` (3
  commits: a forecast fix, its own revert after adversarial review found it
  anti-conservative, and a beads export — net source change zero).
- Two already-merged branches that are stale refs only:
  `agent/4d8.24.7.1-txn-page-recurring-filter`, `agent/867.1-staple-dmg`.

`fkt5.11` runs the deletion (`git push origin --delete <branch>...` for the
remote refs, `git branch -D` locally) as its first step, ahead of the
rewrite.

### 4. Memories and beads leave git entirely — enacted here, decided in ADR 0064

This ADR does not re-decide ADR 0064; it enacts the mechanics ADR 0064
requires. `.beads/` is added to `.gitignore` and removed from history in the
same `filter-repo` pass as §1. The Dolt database stays the source of truth,
backed by two independent mechanisms per ADR 0064 §3 — `bd dolt push` to a
private Dolt remote, and a separate private git mirror of `.beads/` — both
of which must exist and be verified to restore *before* the force-push, not
assumed to work. That verification is `fkt5.11`'s gate, not this ADR's.

### 5. Bundle identifier and vault data directory: unchanged

Already decided in ADR 0067: `ai.personalcfo.desktop` stays the bundle
identifier, and the vault data directory it derives does not move. Restated
here only because `fkt5.11` bundles it into the same "ADR 0062 decisions"
checklist — the substantive decision and its rationale live in ADR 0067.

### 6. Pre-flip exposure sweep: recommendation for the docs/agent kickoff files

The full sweep is `personal-cfo-o1nxk` item 8 (public-exposure sweep across
`docs/`, `scripts/`, `.github/`, `README.md`, `SECURITY.md`,
`CONTRIBUTING.md`), which is still open. This ADR records the one
disposition already decided, for the specific set of files `o1nxk` item 8
named as input to this bead:

- **Remove from the public tree:** `docs/agent/HANDOFF.md`,
  `docs/agent/HANDOFF-2026-08-01.md`, `docs/agent/HANDOFF-2026-08-02.md`,
  `docs/agent/SESSION_KICKOFF.md`, `docs/agent/PLAN_KICKOFF.md`,
  `docs/agent/BEAD_REVIEW_KICKOFF.md`.
- **Keep in the public tree:** `docs/agent/PROJECT_PROFILE.md`,
  `docs/agent/WORKFLOW_ROLES.md`, `docs/agent/FRONTEND.md`,
  `docs/agent/demo-vault.md`, `docs/agent/TAURI_RUST_REACT_FINANCE_APPENDIX.md`.

**Rationale:** the removed set is session-kickoff and handoff operational
material — internal working notes with no value to an external contributor,
and `HANDOFF.md` specifically names an account unrelated to the project
(one non-project email address, confirmed present, content not repeated
here). The kept set documents durable project conventions (architecture,
the four-role workflow, frontend conventions, the demo vault, the
Tauri/Rust/React reference) that a contributor genuinely needs and that
carry no comparable exposure. This is a recommendation the owner reviews in
this ADR's PR, not a final sweep — `o1nxk` item 8's broader pass over
`scripts/`, `.github/`, and the root-level files may surface more, and its
findings take precedence if they conflict with this list.

### Sequencing

This ADR must be **Accepted before `fkt5.11`'s force-push**, not after —
`fkt5.11` now depends on this bead (reversed from the bead's original
ordering) per `AGENTS.md` §1A: the decision is recorded and reviewable
before the irreversible action executes, not reconstructed from what the
rewrite happened to do.

## Consequences

### Positive

- The repository keeps its identity, issues, and merged-PR history; the fork
  is not a fresh repository with severed provenance.
- One combined `filter-repo` pass closes both exposure problems found
  (paths and values) instead of two separate rewrites, each with its own
  window of risk and its own force-push.
- The no-Block-pushes rule is recorded where `fkt5.9` (which executes the
  GitHub settings) can find it, preventing a self-inflicted `GH007` outage
  on the next push after the flip.
- Branch cleanup shrinks the force-push blast radius to `main` only.

### Negative

- **Every commit SHA becomes stale the moment the rewrite runs.** Anything
  written before the rewrite that cites a specific SHA (this document
  included, where it cites PR numbers rather than SHAs for that reason)
  needs to be read as historical description, not a resolvable pointer,
  after the rewrite lands.
- **The owner's personal Gmail address remains permanently visible** across
  809 commits in public git history (as of 2026-09-08 — this count grows
  with every future locally-authored commit). This is a deliberate
  tradeoff, not a claim that history is already uniform: `origin/main` also
  carries 396 commits authored under the owner's GitHub noreply identity
  (from squash-merges through the GitHub UI/API) and 21 under a leaked
  hostname. Keeping the Gmail author avoids the complexity and risk of a
  mailmap rewrite across all three identities — accepted explicitly rather
  than by default.
- The exposure recommendation in §6 is scoped to the files `o1nxk` item 8
  already flagged, not an exhaustive sweep. Treat it as provisional until
  `o1nxk` item 8 closes.

## Revisit if

- `o1nxk` item 8's full sweep finds exposure that changes the file
  disposition in §6, or finds content requiring another value-based scrub
  — fold any new replacement rules into the **same** `fkt5.11` rewrite pass
  if the window (private, zero forks, zero stars) is still open; do not run
  a second rewrite after the flip.
- The owner later decides to adopt the noreply commit identity on every
  machine — at that point, and only then, "Block command line pushes that
  expose my email" may be enabled (§2).

## Implementation notes

- `fkt5.11` executes §1, §3, and §4: branch cleanup, a re-run dry run against
  current `main` (inputs drift over time), the combined
  `--path`/`--replace-text` pass, backup verification, and the owner's
  quoted force-push approval — gated on this ADR's Accepted status.
- `fkt5.9` executes the GitHub settings pass after the flip, including the
  email-privacy settings this ADR's §2 constrains.
- `fkt5.4` (the LICENSE-swap PR) is blocked on this ADR by bead dependency
  and needs no further action from this ADR beyond being Accepted.
- This ADR does not itself move or delete the `docs/agent/` files listed in
  §6 — that removal happens in the pre-flip PR that actually ships history
  changes (`fkt5.11` or its companion PR), once the owner has reviewed the
  recommendation here.

## Amendment (2026-09-08): snapshot supersedes the rewrite

Sections 1, 3 and 4 above described an in-place history rewrite
(`git filter-repo` + force-push) as the fork mechanism. The owner reversed
that decision the following day, before the rewrite ran. This amendment
records the reversal; it does not restate the superseded sections, which
stay as written above for the historical record of why the rewrite was
considered and what it would have done.

**(a) Decision.** `dohflow/dohflow` is created as a **new, empty, private**
repository under the org (owner action, `personal-cfo-uxev1` step 0 — not
this bead). On go-live day, the launch tree — tracked files only, as of the
private repo's `main` at that moment — is pushed to it as **one initial
commit** (`personal-cfo-uxev1`, procedure in
`docs/operations/public-launch-snapshot.md`). The new repository is then
flipped to Public (`personal-cfo-fkt5.9`).

**(b) The private repository (`chrisbustos/personal-cfo`) is retained as
the archive.** It is never rewritten and never deleted. It may be archived
read-only on GitHub, but only after development has fully moved to the
public repository — that transition is a separate, future bead, not part of
this one.

**(c) Rationale.** The rewrite's privacy outcome depended on GitHub Support
purging old commits that stay reachable through pull-request refs
(`refs/pull/N/head`) after a force-push — a force-push alone does not make
old commits unfetchable by SHA, since GitHub keeps PR refs independent of
branch history. The snapshot depends on nothing: the old history never
leaves the private repository at all, so there is nothing on the public
side that a purge could need to catch up on. Strictly better privacy, and
substantially less work — no `--replace-text` rules file, no blast-radius
branch cleanup, no force-push window.

**(d) What is given up, and why it is accepted.** Pre-launch `git
blame`/`bisect` history and the merged-PR record (`#1`–`#407` and beyond)
stay in the private archive, not the public repository. This is accepted
because: the project has a single author, so attribution loss has no
collaborator to lose it for; the 67+ dated ADRs and the CHANGELOG already
carry the development narrative independent of commit history; and the
public repository's own README and CHANGELOG will state plainly that
history was squashed at public release specifically to keep personal
dogfooding data out of the public record — an intentional, disclosed
choice, not a gap.

**(e) Unchanged from the sections above.** Section 2 (keep the Gmail
author identity; GitHub's "Block command line pushes that expose my email"
setting stays OFF) still governs every future commit in the private
repository — nothing about the snapshot changes who authors commits or how
they're identified going forward. Section 5 (bundle identifier and vault
data directory, unchanged) and section 6 (the `docs/agent/` pre-flip
exposure disposition) are unaffected by this amendment; both describe
content decisions independent of the fork mechanism.

**(f) The five stale remote branches are left alone.** Section 3's branch
cleanup existed to shrink a force-push's blast radius. A snapshot has no
force-push and touches no branch history at all, so there is nothing to
clean up before it — those five branches remain exactly as they are,
open questions for whenever (if ever) someone gets to them, unrelated to
this bead.
