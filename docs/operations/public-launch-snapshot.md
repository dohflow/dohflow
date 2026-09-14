# Public launch snapshot (owner-run, go-live day)

- Beads: `personal-cfo-uxev1` (executes this procedure), `personal-cfo-867.1.3`
  (tags and builds `v0.1.0` from the resulting clone), `personal-cfo-fkt5.10`
  (the broader public-launch runbook — link here for the go-live-day snapshot
  mechanics rather than re-deriving them)
- Decision this implements: [ADR 0062's 2026-09-08
  amendment](../adr/0062-public-repo-fork-mechanics.md#amendment-2026-09-08-snapshot-supersedes-the-rewrite)
  — `dohflow/dohflow` is a **new, empty** repository seeded by **one initial
  commit**, a snapshot of the private repository's tracked files at the
  go-live moment. The private repository (`chrisbustos/personal-cfo`) is
  **never rewritten and never deleted** — it stays private forever as the
  archive. There is no `git filter-repo`, no force-push, no branch deletion,
  no GitHub Support purge, and no resetting of local checkouts anywhere in
  this procedure.
- **Owner-run.** Creating the new repository, pushing to it, and flipping
  visibility are all outward-facing, largely irreversible actions
  (`AGENTS.md` §1) — this is a sitting-with-the-owner procedure, not
  something to run unattended.

## Before you start — preconditions

All of the following must be true before the snapshot is taken:

1. **`personal-cfo-fkt5.4`** (the owner-signed LICENSE-swap PR) is merged.
2. **`personal-cfo-o1nxk`** (pre-release security review) is closed.
3. **Zero open PRs** against `chrisbustos/personal-cfo` — `gh pr list --state open`
   returns nothing, or every open PR has been deliberately deferred with the
   owner's sign-off.
4. **`git status --short` is clean** in the private repository's `main`
   worktree, apart from the untracked `.agents/` and `.codex/` directories
   (pre-existing, unrelated to this project).
5. `.beads/` is untracked (`git ls-files .beads/` returns nothing) —
   `personal-cfo-fkt5.11`.

If any precondition is not met, stop and resolve it first — do not snapshot
a tree with in-flight work.

## 1. Record the go-live commit

From the private repository's `main`, at the exact commit being snapshotted:

```sh
git rev-parse HEAD
```

Record this SHA in the `uxev1` bead note. **No tag is created on the private
repository for this** (`git tag v0.1.0-source <sha>` is not needed) — the SHA
recorded in the bead note is sufficient provenance, and tagging the private
repo would add a permanent, irrelevant marker to an archive that is never
published.

## 2. Create the new, empty public repository

Owner action (`uxev1` step 0, not automated here): create `dohflow/dohflow`
under the org as a **new, empty, private** repository — private at creation
so the snapshot can be pushed and verified before the visibility flip
(`personal-cfo-fkt5.9`) makes it public.

## 3. Build the snapshot in a fresh clone

```sh
git clone --quiet git@github.com:dohflow/dohflow.git /tmp/dohflow-launch
cd /tmp/dohflow-launch
```

From the **private repository's checkout**, at the recorded SHA, archive the
tracked tree straight into the new clone — `git archive` walks the tree
exactly as `git ls-files` would, so ignored and untracked paths (including
`.beads/`, which is both) are structurally absent, not filtered out
after the fact:

```sh
git -C "<private repo checkout>" archive <recorded-sha> | tar -x -C /tmp/dohflow-launch
```

## 4. Verify the tree before committing anything

All of the following must pass before proceeding to step 5:

```sh
cd /tmp/dohflow-launch
git add -A

# .beads/ must be completely absent from what's about to be committed.
[ "$(git ls-files | grep -c '^\.beads/')" = "0" ] || echo "FAIL: .beads/ present"

# The six ADR 0062 §6 / personal-cfo-o1nxk item 8 files must be completely
# absent too — this is what the private repo's .gitattributes export-ignore
# entries are FOR (step 3's `git archive` already excluded them; this just
# proves it, the same way the .beads/ check above proves that exclusion).
for f in docs/agent/HANDOFF.md docs/agent/HANDOFF-2026-08-01.md \
         docs/agent/HANDOFF-2026-08-02.md docs/agent/SESSION_KICKOFF.md \
         docs/agent/PLAN_KICKOFF.md docs/agent/BEAD_REVIEW_KICKOFF.md; do
  [ -f "$f" ] && echo "FAIL: $f present (check the private repo's .gitattributes)"
done

# Nothing unexpected staged.
git status --short

# The new clone's tree matches the private checkout's TRACKED tree exactly.
# Comparing working directories with `diff -r` here would be wrong: the
# private checkout also has gitignored build output (target/, node_modules/,
# dist/, .dolt/) and untracked local directories (.agents/, .codex/) that
# `git archive` never carries into the snapshot in the first place — a
# working-directory diff reports dozens of "Only in <private>" lines that
# have nothing to do with whether the snapshot is correct. Compare by BLOB
# HASH over each tree's tracked paths instead, which is exactly what "the
# tree matches" means and is immune to either side's untracked/ignored
# clutter:
diff <(git -C "<private repo checkout>" ls-tree -r <recorded-sha>) \
     <(git -C /tmp/dohflow-launch ls-tree -r "$(git -C /tmp/dohflow-launch write-tree)")
```

The `diff` must be empty — every tracked path in both trees names the
identical blob. If it is not, stop — do not commit a tree that doesn't
match what was reviewed.

Then run the two content scans the public tree must pass:

```sh
gitleaks detect --source /tmp/dohflow-launch --no-git
```

And the `o1nxk` value scan (real financial figures, bank/broker patterns):

```sh
./scripts/value-scan.sh /tmp/dohflow-launch
```

Both must report zero findings. If either finds something, stop, fix it in
the **private repository first** (so the fix is also in every future
snapshot), re-run step 3 from the corrected SHA, and re-verify.

## 5. Commit and push

```sh
git commit -m "Initial public release

Snapshot of the private development repository at <recorded-sha>. DohFlow
was developed privately from May 2026; the pre-release history is
intentionally not published so that personal dogfooding data never enters
the public record. See README.md and CHANGELOG.md."
git push origin main
```

Substitute the actual SHA recorded in step 1 for `<recorded-sha>`. The
README's "History" note and the CHANGELOG line explaining the squashed
history (added by `fkt5.4` or `n67eh`) should already be present in the tree
being committed — this step does not add them; it only records why this
particular commit has no ancestry.

## 6. Hand off to the remaining go-live steps

- `personal-cfo-867.1.3` tags `v0.1.0` on this commit and builds the release
  from **this clone** (`/tmp/dohflow-launch`, or a fresh clone of
  `dohflow/dohflow` at this commit) — not from the private repository's
  checkout — so the app's About-card commit SHA matches what a public user
  can actually see and clone.
- `personal-cfo-fkt5.9` flips `dohflow/dohflow` to Public and runs the
  fork-day GitHub settings pass.
- The private repository continues exactly as before: same remote, same
  branches, same PR history, same bead graph. Development does not move to
  the public repository as part of this procedure — that migration (and
  eventually archiving the private repository read-only) is a separate,
  future bead.
