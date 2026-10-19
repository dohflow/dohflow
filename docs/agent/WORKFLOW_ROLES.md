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

Each runs in its own worktree. **One implementation agent is active at a time.**
No parallel implementation workers in this pilot.

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
sequential ownership — one implementation agent, no simultaneous writes to the
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
