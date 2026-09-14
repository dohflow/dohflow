# ADR 0064 — Task tracking leaves the product repository

- **Status:** Accepted (owner decision, 2026-09-05)
- **Date:** 2026-09-05
- **Amends:** `AGENTS.md` §9.1 ("the JSONL export is committed to git as the durable artifact")
- **Forces a choice in:** ADR 0062 / `personal-cfo-fkt5.2` (fork mechanics)
- **Replaces the approach in:** `personal-cfo-fkt5.11` (pre-flip privacy review)

## Context

`.beads/issues.jsonl` is a generated export of the local Dolt bead database,
committed to git so that the task graph has a durable artifact and so a git merge
driver can regenerate it on conflict. That was the right call for a private,
single-developer repository.

It stops being the right call the moment the repository goes public
(`personal-cfo-fkt5.9`), and the reason is not a judgement call — it is arithmetic:

| Measured 2026-09-05 | |
|---|---|
| Commits touching `.beads/` | **813** |
| Total commits in the repository | **1,193** |
| Share of history | **68%** |
| Current export: rows naming a bank or broker | 21 |
| Current export: rows carrying a `$` figure | 23 |
| Agent memories in the export | 136 |

**Redacting the current file does nothing.** The content lives in 68% of the
commit history, and an in-place visibility flip publishes all of it permanently.

There are also two reasons to remove it that have nothing to do with privacy:

1. **It is the wrong shape for a public repository.** 1,298 internal issues and 136
   agent memories are not useful to a contributor; they make the repository read
   as a personal workspace rather than a project.
2. **The tracker is expected to change.** The owner intends to move to a tracker
   usable by both humans and coding agents (Linear, Jira, or similar) once other
   people are involved. Whichever tracker wins, it should not live inside the
   product repository.

## Decision

### 1. `.beads/` leaves the product repository

Added to `.gitignore` and removed from tracking. The Dolt database remains the
source of truth, as it always was; the JSONL stays a local export.

### 2. The history is rewritten before the flip, not after

```sh
git filter-repo --path .beads/ --invert-paths
```

**This is cheap here specifically, and the window closes at the flip.** The
repository is private, has **zero forks and zero stars**, and has never been
public: no one has cloned it, no commit SHA is referenced anywhere external, and
no link can break. A rewrite costs one force-push to a remote only the owner uses.

After the repository is public, the same operation costs broken clones, dead
permalinks, and a public record of what was removed — which advertises the thing
the rewrite was meant to bury.

`git-filter-repo` is not currently installed (`brew install git-filter-repo`).

### 3. Durability is replaced, not abandoned

Removing the file from git removes the only off-machine copy of the task graph.
`bd dolt remote list` currently reports **"No remotes configured"** — the Dolt
database on one Mac is, today, the single point of failure.

Two independent backups replace what git was providing:

- **`bd dolt push` to a private Dolt remote.** This backs up the actual source of
  truth, including history, not a flattened export.
- **`.beads/` mirrored to a separate private git repository.** Independent of
  Dolt entirely, so a Dolt-side problem cannot take both.

They must fail independently. Neither alone is sufficient.

### 4. `AGENTS.md` §9.1 is amended

The line "the JSONL export is committed to git as the durable artifact" becomes
false on adoption and must be corrected in the same change, or the next agent
will faithfully re-commit the file.

## Consequences

**Good.** The privacy exposure is closed permanently rather than snapshot by
snapshot — no future bead becomes a privacy decision, and there is no ongoing
tax whose failure mode is "someone forgets once." `fkt5.11` collapses from a
per-row redaction review to a verification step. The public repository presents
as a project rather than a workspace. The eventual tracker migration gets easier,
because the tracker is already decoupled from the product.

**Costs.** A history rewrite invalidates every existing commit SHA — harmless
here, but it must happen while that is still true. Two backup mechanisms need
standing up and, more importantly, **need periodic proof that they restore**; an
unverified backup is a belief, not a backup. Contributors lose the ability to
read the task graph from the repository, which is the intended outcome but is
still a loss.

**This forces the `fkt5.2` decision.** "In-place flip" and "history rewrite" are
the same operation on this repository. ADR 0062 cannot describe a flip that
preserves history untouched *and* adopt this ADR; it must describe the rewrite,
including that PR numbers #1–#382 survive on GitHub while their commit SHAs do not.

**Not decided here:** which tracker replaces beads, or when. That is a separate
decision for when other people are actually involved. This ADR only removes the
tracker from the product repository; beads continues to work exactly as it does
today, just backed up elsewhere.

## Addendum (2026-09-08): snapshot supersedes the rewrite mechanism

Section 2 above ("The history is rewritten before the flip, not after")
described `git filter-repo --path .beads/ --invert-paths` as how `.beads/`
would leave the repository's history before the public flip. That mechanism
is superseded by [ADR 0062's 2026-09-08
amendment](0062-public-repo-fork-mechanics.md#amendment-2026-09-08-snapshot-supersedes-the-rewrite):
the public repository is a new, empty repository seeded by a squashed
snapshot commit, not the private repository with its history rewritten. The
rest of this ADR — the decision to remove `.beads/` from git tracking at
all (§1), the durability requirement (§3), and the `AGENTS.md` §9.1
amendment (§4) — is unaffected and still stands.

**How `.beads/` actually leaves the repository now, without a rewrite:**

- **In the private repository (`chrisbustos/personal-cfo`), now:**
  `.beads/` is removed from git tracking (`git rm -r --cached .beads`,
  files stay on disk) and added to `.gitignore`
  (`personal-cfo-fkt5.11`). No history rewrite is needed for this — future
  commits simply stop touching the path; the 813 historical commits that
  already touched it remain in the private repository's history, which per
  ADR 0062's amendment is retained as a permanently-private archive, never
  published.
- **In the public repository (`dohflow/dohflow`), from day one:** `.beads/`
  never enters it in the first place. The go-live snapshot
  (`docs/operations/public-launch-snapshot.md`) is built with `git archive`
  over the private repository's tracked files at the go-live commit — since
  `.beads/` is untracked by that point, it is structurally absent from the
  archive, not filtered out of it. There is no `.beads/` to remove from the
  public repository's history because it is never in the public
  repository's history.

**The two-backup requirement (§3) stands, unchanged and unaffected by
this addendum:**

- **JSONL mirror** — `scripts/backup-beads.sh`, run automatically on every
  `git push` via `.beads/hooks/pre-push`, pushing to a private mirror
  repository. Working, per `docs/operations/beads-backup-and-restore.md`.
- **Dolt remote** — still blocked (`bd dolt push` does not reach the
  remote, `personal-cfo-es9ew`); the file `file://` mirror of
  `.beads/embeddeddolt/` remains the practical stand-in until that bug is
  resolved or worked around.

Both must still fail independently, and both still need periodic proof
that they restore — an unverified backup is a belief, not a backup,
whether or not a history rewrite is ever in the picture.

**Addendum (2026-09-09):** The hand-written git hooks live in
`scripts/git-hooks/` (tracked, `personal-cfo-apesm`); `.beads/hooks/` holds
only bd's managed sections.
