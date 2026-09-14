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

This machine is an 8 GB M3. **One build at a time** — never run `cargo test
--workspace`, a Tauri build, and a frontend build concurrently across sessions.
Four sessions may be open; they must not all generate continuously.

Planning and review draw Fable from the shared Max allowance, deliberately, so
the pilot can measure it. **If Claude Code indicates a request needs separate
usage credits rather than the included allowance, stop and tell the user.** Do
not enable credits, API billing, top-ups, or another paid inference route.

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
