# Development roles and the review gate

Referenced by `AGENTS.md` §16. Procedures live in the `plan-bead`,
`implement-bead`, `escalate-bead` and `review-pr` skills; this file is the
durable policy those skills implement.

Status: **pilot, under calibration.** Nothing here is settled. Revise it from
observation rather than defending the initial design.

## The four roles

| Session | Model | Effort | Owns | Built-in file editing |
|---|---|---|---|---|
| `01-planning` | Fable | high | Plan + bead formulation | **None** |
| `02-implementation` | Sonnet 5 | high | Implementation while the bead is active | Yes |
| `03-escalation` | Opus 5 | xhigh | Diagnosis; implementation only after authorized takeover | None by default |
| `04-review` | Fable | high | The review verdict | **None** |

Each runs in its own worktree.

**The parallel-lane model** (owner decision `personal-cfo-915.9`, 2026-10-18,
replacing the pilot's "one implementation agent at a time"). Work is organized in
durable **lanes**. Each lane has its own implementation, escalation and review
sessions, suffixed with the lane's letter (`02a`/`03a`/`04a`, `02b`/`03b`/`04b`,
and so on); the table above names the roles without a suffix.

| Lane | Work | Sessions |
|---|---|---|
| a | the Sync engine (the priority lane) | `02a`, `03a`, `04a` |
| b | the iPhone app | `02b`, `03b`, `04b` |
| c | the Sync substrate: accounts, billing, storage, the ops floor, the public Sync protocol | `02c`, `03c`, `04c` |

A further lane adds the next letter. Three sessions stay single and shared by every
lane: `00-overseer` (routing, notifications, build-machine hygiene),
`01-planning` (plans, beads, and the docket) and `05-decisions` (owner decisions).

- **One owner per bead at a time** (`AGENTS.md` section 16, rule 1) still holds. Two
  lanes never edit the same bead or the same file at once.
- **Each lane records its file-ownership split** on its milestone beads: which
  crates, directories and docs it owns.
- **Shared hotspots are coordinated by `01-planning`.** Ask before editing a
  migration number, a shared frontend file, `Cargo.lock`, `pnpm-lock.yaml`, a
  shared CI workflow (`ci.yml`), or a shared policy document such as this one.
  Whichever change merges second rebases.
- **The two cargo-heavy jobs limit is unchanged and is across all lanes** (see
  "Usage and hardware constraints").
- **A lane's review session reviews that lane's pull requests.** The review gate,
  the SHA binding and the no-self-merge rule are the same in every lane.

Role permissions come from `.claude/role-settings/*.json`, merged by hand into
each worktree's `.claude/settings.local.json`. They are **tool-level guardrails,
not a security boundary** — see "What the guardrails do not do".

## Three process levels

**Full workflow — expected for substantive or risky work.** New features or
material behavior changes; multi-file changes; anything touching auth,
authorization, crypto, vault, privacy, billing, financial logic, persistence,
migrations, deletion, or external integrations; architecture changes;
multi-state user-facing flows; releases.

**Lightweight — allowed for low-risk work.** Small isolated fixes with an
obvious expected outcome; tests that do not alter behavior; refactors covered by
tests; docs and copy; small design corrections. Planning may write a concise
bead and implementation may proceed with proportionate review. The review is
shorter — it is not skipped, and the implementation agent still does not merge.

**Direct / exceptional — user-authorized.** The user may explicitly authorize a
trivial change, experiment, spike, or emergency repair. Record the exception and
its scope in the bead, or in a `bd remember` entry if no bead exists. The
four-role process must never obstruct an explicit instruction from the user.

## Definition of ready

Beyond `AGENTS.md` §9.1 (structured `acceptance_criteria`, epic parentage,
priority discipline), a bead ready for implementation carries, where applicable:
outcome; context; scope; **out of scope**; verifiable acceptance criteria;
design states (normal, loading, empty, error, success, disabled, offline,
permission-denied, responsive, accessibility); constraints; dependencies;
verification commands; relevant code area (guidance, not a fence); risks;
escalation conditions; deliverables.

Split a bead when it holds independent outcomes that can be implemented and
reviewed separately. Do not split so finely that a bead loses acceptance value.

## Planning artifacts across worktrees

Each role works in a separate worktree, so **a file edited in one worktree is
invisible to another until it is committed and reachable there.** The bead
ledger is shared (one `bd` database); tracked files are not.

Planning is also read-only with respect to tracked files by design. So when an
ADR, architecture document, product specification, or other tracked planning
artifact must change:

1. Planning creates or identifies a **documentation bead**, and drafts the
   proposed content in conversation for the user.
2. The **implementation** session performs the tracked-file change.
3. That documentation change is reviewed and merged like any other work.
4. Dependent implementation beads reference the **merged document**, or an
   exact commit SHA named in the bead.

An implementation bead must never depend silently on an uncommitted file in
another worktree. If planning finds itself saying "see the doc I just wrote",
that doc is not yet available and the dependent bead is not yet ready.

## Escalation triggers

Stop and escalate when any of these is true. Escalation is a controlled state
transition, not a failure.

- The same failure survives two materially different, evidence-based repairs.
- Acceptance criteria are contradictory, absent, or need a new product decision.
- The work unexpectedly crosses an architecture boundary (`PROJECT_PROFILE.md`).
- Auth, crypto, credential, privacy, migration, deletion, corruption, or
  data-loss risk appears outside the approved plan.
- Scope expands materially beyond the bead.
- Required tests cannot be made reliable or reproducible.
- The agent cannot explain why continuing is safer than stopping.
- Remaining context or plan allowance is insufficient for a responsible
  completion **and** review handoff.

## Concurrent PASSes: the merge queue, and the cross-check where there is none

Two pull requests can each pass review against their own base and still break
`main` together: on 2026-10-19 the PRs #100 and #109 did, and `cargo test -p
db-worker --test column_kinds` failed on `main` once both had merged
(`personal-cfo-md8h2.11`, decision `personal-cfo-915.13`).

**`dohflow/dohflow`: the merge queue is live** (`personal-cfo-md8h2.11`; ruleset
`23361528`; squash merges, the queue waits for all required checks, groups of 1 to
5). The queue builds and tests each pull request on the temporary merge commit with
the pull requests ahead of it, so the combination is gated by CI before it reaches
`main`. The reviewer's two-PR cross-check described below is **retired for `dohflow/dohflow`**.
Every push to `main` still runs the full gates, never cancelled and never
path-filtered, and a failure opens an issue labeled `ci-main-red`.

**The private repositories (`dohflow/iOS`, `dohflow/internal`, `dohflow/platform`)
have no merge queue, so the cross-check still applies there:**

- When two or more pull requests in such a repository have each PASSed and are
  waiting for the owner's merge at the same time, the reviewer **re-checks them
  against each other** before saying they are merge-ready together: merge them onto
  current `main` in a scratch worktree (in the order the owner is likely to merge
  them, and the reverse order if both are plausible) and run that repository's
  gates (for a Rust repository: `cargo fmt --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace`; for `dohflow/platform`:
  `pnpm check`). Report the combined result in the verdict comments of both pull
  requests, and **state the safe merge order in both verdicts, or "either order"**.
- If the combination fails, neither is "merge-ready together": say which one must
  rebase on the other, and remove the `merge-ready` label from the one that must
  change.
- This is the reviewer's job, not the implementer's, and it costs one extra build
  per concurrent pair, so it counts toward the two-cargo-heavy-jobs limit.

This rule ends for a repository when a merge queue is enabled in its `main` ruleset;
delete the repository from the list above then.

## Review and merge policy

- A verdict binds to one commit SHA. A review of a PR title, branch name, or
  working tree is not a review.
- Exactly one verdict: `PASS`, `CHANGES_REQUIRED`, `HUMAN_DECISION_REQUIRED`.
- The reviewer posts its findings as a **structured comment on the PR**, via
  `gh pr comment <N> --body-file -` with a heredoc. It does not edit the PR
  body — that belongs to the implementation agent — and it does not write a
  file to do it.
- Any new commit invalidates the prior verdict. Re-review the new SHA.
- **Automatic reviewer merging is disabled for the calibration period.** A pass
  returns `APPROVED_TO_MERGE`; the user authorizes or performs the merge.
- The bead closes only after the merge is verified — never on a passing review,
  and never on locally passing tests.
- Because all sessions share one GitHub account, this is *procedural*
  independence, not an independent identity — and in this pilot not
  cross-provider independence either, since the reviewer is also a Claude model.
  Both are accepted, recorded limitations.
- Do not configure a required second-party GitHub approval; no second account
  exists to satisfy it.

### Merge-ready pull requests

A pull request that has a `PASS` (`APPROVED_TO_MERGE`) verdict at an exact SHA and
now needs the owner's merge must be unmistakable. The procedure
(`personal-cfo-0rein.5`):

1. **The lane's review session applies the `merge-ready` label** when it posts the
   verdict, after re-checking that the head still equals the reviewed SHA. **It applies
   the label only to the exact head it reviewed**, because the Review gate (item 6) turns
   the label into a merge condition. On any other verdict it makes sure the label is
   absent.
2. **A workflow removes the label on every new commit.**
   `.github/workflows/merge-ready-label.yml` removes `merge-ready` on every
   `pull_request` `synchronize` event, so a label cannot outlive the SHA it
   approved. It cannot do so for a pull request from a fork (read-only token); for
   those, check the head against the verdict before merging.
3. **`00-overseer` sends the owner a push notification** naming the pull request,
   its SHA and its lane, when a pull request becomes merge-ready.
4. **`05-decisions` lists the merge-ready pull requests first** in every ledger
   presentation to the owner:
   ```bash
   for r in dohflow/dohflow dohflow/internal dohflow/dohflow-site; do
     gh pr list --repo "$r" --label merge-ready
   done
   ```
5. A merge never closes a bead by itself (rule 4 of `AGENTS.md` section 16).
6. **The `Review gate` check makes the label load-bearing** (`personal-cfo-md8h2.23`, owner
   decision `personal-cfo-915.20`; `.github/workflows/review-gate.yml`,
   `scripts/review-gate.sh`). On a pull request it passes only while the label is on, and
   never on a `synchronize` (a new commit): a reviewer must re-apply the label to the
   exact head they reviewed, and then it passes. In the merge queue each entry is gated on
   its own and gates only the pull request named in its
   `gh-readonly-queue/<base>/pr-<N>-<sha>` ref (the `<sha>` there is the entry's base): that
   pull request must carry the label **and** its current head must be an ancestor of the
   commit the queue is testing, so a head pushed after the review cannot be the one that
   merges. It fails closed (an unreadable ref, a lookup error, a missing label) and uses
   only the read-only token. **It is a guardrail, not a security boundary**: labels are not
   permission-restricted on GitHub and a pull request can edit the workflow (ADR 0071).
   **Limitation:** removing the label after a queue entry's run has passed does not eject
   the entry; only a new commit does. The check is added to the `main` ruleset by
   `00-overseer` once it has run green on a real pull request and a merge-group run (it is
   not required before that, or every open pull request would stall); its job name,
   `Review gate`, must stay stable.

### Red-main fast path (owner decision `personal-cfo-915.15`, option D)

When `main` itself is red, a pull request can fail a required check through no
fault of its own, and waiting for `main` to be fixed blocks unrelated work. The
standing rule:

- A pull request whose **only** failing required checks are `main`'s current,
  already-tracked failures (a bead exists for each), and which has a reviewer
  `PASS` that states, in the verdict, **"fails only on main's known red `<check>`
  (bead `<id>`)"**, **may be admin-merged by the owner without a fresh decision**.
  The Admin bypass of the `main` ruleset (ADR 0071) is the mechanism, and the
  `PASS` comment is the artifact the merge cites.
  On `dohflow/dohflow`, where the merge queue is live, "admin-merged" means the owner
  **admin-bypasses the queue** (merges the pull request directly with the Admin role,
  ADR 0071) instead of queueing it, since a queued run would fail on `main`'s red
  check; the verdict line, the three things the reviewer establishes below, and the
  exclusions are unchanged. In a repository without a queue it is an ordinary admin
  merge.
- `05-decisions` still lists such a pull request at the top of the ledger as
  merge-ready, with the named check and bead beside it.
- The reviewer names the check **and the bead** every time, and establishes
  three things before writing the line:
  1. **The failure is `main`'s own.** A pull request's CI tests
     `refs/pull/N/merge`, a merge commit that sits on `main`'s tip *at the time of
     that run*, not on the pull request's base. Compare against the **first parent
     of the failing run's merge commit** (the `main` commit the run actually
     merged onto), and confirm `main` is **still red there** in the same check,
     the same way. A failure the reviewer cannot match to a tracked red on `main`
     is a `CHANGES_REQUIRED`, not a fast-path `PASS`.
  2. **Nothing was masked.** A step that fails early skips the later steps of the
     same job, so a known-red step can hide a real problem in the pull request
     (for example, a failing `cargo audit` step skips `cargo deny`, `pnpm audit`
     and the frontend license scan that follow it). The reviewer accounts for
     **every later step** of each failing job: it ran and passed, or the reviewer
     ran it locally and says so, or it provably cannot differ from `main` (for
     example the lockfiles are identical to `main`'s). The verdict **names any step
     that could not be run**.
  3. **No exclusion applies** (below).

**Two exclusions, with no exceptions:**

1. **Never for a pull request that touches the failing area itself.** If the
   change is in the crate, test, workflow or file that is red on `main`, its
   failure is not provably `main`'s, so it waits for a green run. For an audit
   or other CI failure the failing area includes the **workflow step and the
   scripts it runs, the lockfiles, and the audit configuration** (`deny.toml`,
   audit allowances).
2. **Never when `main`'s red is a security advisory and the pull request changes
   manifests or lockfiles** (owner decision 2026-10-21). A pull request that adds,
   changes or bumps a dependency (any `Cargo.toml`, `Cargo.lock`, `package.json`
   or lockfile change) does not ride on a red audit. **A code-only pull request**
   (including one in a crate that depends on the flagged crate) **may** use the
   fast path, **but only if the reviewer's `PASS` explicitly confirms that the
   pull request does not start using the advisory's affected API.** If the
   reviewer cannot confirm that, the verdict is `CHANGES_REQUIRED`. To make the
   guard checkable:
   - **The affected API** is the advisory's listed affected functions; if the
     advisory lists none, it is **any use of the affected crate**.
   - **"Starts using"** includes newly reaching it **through a wrapper crate**: for
     example a new TLS client or connection for a TLS advisory counts, even though
     the pull request never names the flagged crate.
   - **The verdict records which advisory and which API were checked** (the
     advisory ID and the functions or crate), so the owner can see what was
     confirmed.

The rule does not relax anything else: every other required check must be green,
a new commit voids the `PASS`, and the owner still merges (rule 2 of `AGENTS.md`
section 16).

## What the guardrails do not do

Stated plainly so nobody mistakes the pilot for containment:

- `Edit` deny rules cover Claude's built-in file tools, the file commands Claude
  Code recognizes in Bash (`cat`, `head`, `tail`, `sed`), and shell redirection
  targets. **They do not cover arbitrary subprocesses** — a Python or Node
  script that opens a file itself is unaffected.
- The reviewer and escalation roles run the project's own test suites
  (`cargo test`, `pnpm test`). Those execute arbitrary project code with full
  write access. A "read-only" role is read-only with respect to *the agent's own
  edits*, not with respect to everything that happens in its worktree.
- The pre-push guard is local to this machine, and `--no-verify` bypasses it.
  GitHub-side branch protection is unavailable on this plan.
- OS-level sandboxing would be needed for a real filesystem boundary. That is
  deliberately **out of scope** for this pilot.

Agents must not use Python, Node, shell redirection, or any other indirect
mechanism to do something their role's permission profile denies. The profile
expresses the intent; routing around it is a process violation even where it is
technically possible.

## Usage and hardware constraints

Since 2026-10-11, the working and build machine is a Mac Mini (Apple M5 Pro,
15 cores: 5 performance + 10 efficiency, 64 GB). It replaced an 8 GB M3 MacBook,
whose one-build-at-a-time limit no longer applies. A clean aarch64 release
build of the desktop app takes about a minute on the Mac Mini.

**At most two cargo-heavy jobs at once across sessions.** Cargo-heavy means
`cargo test --workspace`, the desktop `cargo test`, `cargo clippy` over a full
graph, or a Tauri/release build. Each one already uses every core. Each
worktree has its own `target/`, so two jobs don't block each other on cargo's
lock, but a third mostly queues for CPU. Frontend builds and tests don't count
towards the two. All four sessions may be open, and they may generate at the
same time.

The desktop suite has one known parallel-test race
(`connector_spans_carry_the_adapter_and_never_the_token`, `personal-cfo-56w.5`),
which the 15-core machine loses every time. Until that lands, confirm a lone
failure with `--test connector_ipc -- --test-threads=1` before treating it as
real.

Planning and review draw Fable from the shared Max allowance, deliberately, so
the pilot can measure it. **If Claude Code indicates a request needs separate
usage credits rather than the included allowance, stop and tell the user.** Do
not enable credits, API billing, top-ups, or another paid inference route.

## Build-machine hygiene

On 2026-10-19 the Mac Mini (460 GiB) reached 98% full. The causes were a Rust
`target/` directory in every worktree (the largest were around 50, 36 and 32 GiB),
about 54 GiB of session scratch builds under `/private/tmp/claude-<uid>`, and
Time Machine local snapshots that kept deleted build output alive, so that
`cargo clean` freed roughly 139 GiB while free space barely moved until the
snapshots thinned. A full disk blocks builds and releases. The rules below keep
that from building up again (`personal-cfo-0rein.6`, owner request 2026-10-19).

**Decisions.**

- **Each worktree keeps its own `target/`.** Cargo fingerprints include absolute
  source paths, so a shared `CARGO_TARGET_DIR` across worktrees would share
  little and add lock contention.
- **`sccache` is not adopted.** It is a new dependency, and a possible later owner
  decision, not part of this change.
- **Nothing here deletes a worktree, a branch or a scratch directory on its
  own.** Those stay explicit actions (`AGENTS.md` section 1).

**The tool.** `scripts/dev-hygiene.sh` is read-only by default.

- `scripts/dev-hygiene.sh` (or `--report`, with `--json` for machine output) lists,
  for every registered worktree: its cargo `target/` size, last activity, branch,
  whether the branch is merged into `origin/main` or its remote branch is gone,
  uncommitted changes, commits that exist on no remote, and whether a
  cargo/rustc process is running in it. It also reports the size of the session
  scratch directory and the free disk space, and flags free space under 15%.
- `scripts/dev-hygiene.sh --clean [--idle-days N] [--yes]` shows its plan and asks
  for confirmation (`--yes` skips the question). It does exactly two things:
  `cargo clean` in worktrees idle for at least N days (default 3) that have no
  uncommitted changes, no unpushed commits and no running cargo process; and
  `git worktree prune` for registrations whose directory is already gone. It
  never removes a worktree directory, a branch or a scratch directory, never runs
  `rm`, never touches a dirty or unpushed worktree, and never touches the
  worktree it is run from. `scripts/tests/dev-hygiene.test.sh` covers those
  refusals.
- "Last activity" is an approximation (git index and reflog, and the newest
  entries in the cargo target directory). The worst result of a wrong guess is a
  rebuild, never lost work.

**Rules per role.**

- **Implementation.** After a PR merges, remove your own per-bead worktree and
  local branch once you have verified they are clean and pushed. First run
  `git status --short --ignored` in the worktree: `git worktree remove` deletes
  ignored files too, so confirm the only ignored entries are rebuildable output
  (`target/`, `node_modules/`) and nothing you still need. Then `git worktree
  remove <path>` (never `rm -rf`, never `--force`), then `git branch -d <branch>`.
  If `-d` refuses only because the PR was squash-merged, confirm the PR is
  merged and the branch tip is on `origin` (`git branch -r --contains <branch>`),
  and only then use `git branch -D`; otherwise stop and ask. This is the standing
  owner authorization for this one case, for the **local** branch only, and for
  nothing else in `AGENTS.md` section 1. Deleting the branch on `origin` is
  outside it and stays the owner's call. A long-lived lane worktree is not removed: after the merge, return it
  to a detached `origin/main` (`git switch --detach origin/main`).
- **Review and escalation.** Both run the project's test suites, so they fill
  `target/` too. Run `cargo clean` after each verdict on a large PR, or whenever
  your `target/` exceeds about 15 GiB.
- **Overseer.** Run `scripts/dev-hygiene.sh --report` at session start and once a
  week. When free space is under 15%, **ask the owner before any `--clean`**.
- **Everyone.** Session scratch builds under `/private/tmp/claude-<uid>` are
  reported only. Removing one is an explicit act by the session that made it or
  by the owner, never by `--clean`.

**Owner-side note: Time Machine local snapshots.** Deleting build output does not
return space at once if local Time Machine snapshots still hold it. These are
owner-run commands, never run by an agent:

```bash
tmutil listlocalsnapshots /
sudo tmutil thinlocalsnapshots / 50000000000 4
```

The first lists the snapshots. The second asks macOS to reclaim about 50 GB
(the second number is the byte target; the last is urgency, 1 to 4) and needs
administrator rights. Free space can take a few minutes to catch up.

## Bead access across worktrees

All worktrees share one embedded-Dolt bead database. Under this pilot's
one-owner-per-bead ownership — no simultaneous writes to the
same bead — that is the intended configuration, not a hazard. Observe how lock
contention is reported if it occurs, and record it here. Do not manufacture
concurrent writes against real bead data to test it.

## Git hooks across worktrees

`git config core.hooksPath scripts/git-hooks` is a **repository-level**
setting, not a per-worktree one (`extensions.worktreeConfig` is not enabled
here) — set it once, from any worktree, and every worktree of this repository
picks it up immediately from the shared `.git/config`. Because
`scripts/git-hooks/` is a tracked directory, it is present in every worktree's
checkout without any extra step, including a brand-new one created with
`git worktree add`. Set it once per **clone** (not per worktree) as part of
initial setup (`CONTRIBUTING.md`); `personal-cfo-r36ck` runs it for a fresh
clone's bootstrap.

The tracked pre-push hook (`scripts/git-hooks/pre-push`) invokes
`bd hooks run pre-push` **directly** — since `personal-cfo-bb96v`, it no
longer chains to a file under `.beads/hooks/` at all (the original
`personal-cfo-apesm` design did; that file-reference was the root cause of
`bb96v`, see below). If `bd` is not on PATH at all (the public clone, or a
machine without it installed), the direct invocation is a silent no-op —
the branch-protection guard above it still runs regardless. See
`scripts/git-hooks/pre-push` for the full rationale.

**The mirror-backup call is different**: `scripts/backup-beads.sh` resolves
`.beads/` next to *its own* file path, i.e. relative to whichever worktree is
running it — not via `--git-common-dir`. A linked worktree with no `.beads/`
of its own (this repository's `../personal-cfo-review`, for example) has
always had the backup effectively unavailable there; the only change from
this bead is that it is now a **silent** no-op in that case rather than a
warning, matching the mirror-backup call's "silent when the script or
database is absent" contract. The backup only ever actually runs from a
worktree that has `.beads/` — normally the main one.

**There is no need to ever run `bd hooks install --beads` on this repo** —
the tracked hooks above already invoke `bd hooks run <name>` directly, so
its generated `.beads/hooks/*` files would be redundant. If it's run
anyway, it resets `core.hooksPath` to the **absolute** `.beads/hooks`
(verified against bd 1.2.2, no warning); just re-run
`git config core.hooksPath scripts/git-hooks` and nothing else needs
fixing or deleting.

**Historical note (`personal-cfo-bb96v`, fixed 2026-09-09):** before the
direct-invocation fix above, the tracked hooks chained to a file at
`.beads/hooks/`, resolved via `git rev-parse --git-common-dir`. Running
`bd hooks install --beads` copied that file's content into its own
generated output ahead of its own managed section, making the chain
resolve to *itself* — the next push recursed infinitely, forking a new
process each level, **even after re-running `git config core.hooksPath
scripts/git-hooks`** (the corruption lived in the file's content, not the
config). Confirmed via reproduction: 5+ duplicate commits landed on the
production JSONL mirror in ~10 seconds before a watchdog killed the
process tree. That file-reference no longer exists anywhere in the tracked
hooks, so this failure mode cannot recur.

## Calibration log

Record per PR: planning revisions before ready; first-pass completion; repair
cycles; escalations and cause; reviewer findings by severity; defects found
after review; usage pressure; human interventions; steps that added overhead
without improving quality. Use these to simplify or strengthen the workflow.

### Entries

- **2026-10-18/19 — parallel lanes adopted** (`personal-cfo-915.9`, owner option
  b; recorded by `personal-cfo-0rein.5`). The one-implementation-agent pilot rule
  is replaced by durable lanes, each with its own implementation, escalation and
  review sessions. Watch: hotspot collisions between lanes; reviewer contention
  (the Sync engine lane wins ties); whether per-lane review keeps findings per
  PR where they were; whether the `merge-ready` label is ever stale or missing;
  and the cargo-heavy limit across lanes.
