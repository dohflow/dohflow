# ADR 0069 — Bead-graph reconciliation rules

- **Status:** Accepted (2026-09-18)
- **Tier:** Public — process rules for this repository's own task-tracking
  practice, no business content.
- **Bead:** `personal-cfo-jtn8n`
- **Decider:** Owner, program plan v0.8.1 §5.1/§5.2/§5.10 (approved
  2026-09-15/16); transcribed into ADR form by the implementation session
  per AGENTS.md §1A ("architecturally-significant choice... has an Accepted
  ADR").
- **Related:** AGENTS.md §9/§9.1 (this ADR's ladder text is quoted there,
  not restated), `personal-cfo-9lwtm` (Phase A reconciliation sweep, the
  first and largest consumer of these rules), `personal-cfo-u9ze6` (`bd
  ready` undercounting bug, a known limitation of the "ready" queue this
  ADR's process leans on), `personal-cfo-56w.2` (the per-session tracking-
  log precedent 9lwtm follows), ADR 0064 (task tracking stays out of the
  repo — this ADR's rules apply to that same private bead graph, not to a
  public issue tracker)

## Context

AGENTS.md §9.1's priority ladder reads "P1 = Week-8 first-playable
foundation; P2 = MVP-1" — every band but P0 names a milestone that is over,
so re-grading a bead against it re-encodes confusion rather than resolving
it. Three consecutive plan drafts regraded beads without reading their live
priority, compounding the drift. 396 of 523 open beads (as of 2026-09-16)
came from the May 2026 plan import and describe an architecture that
shipped in a different shape than planned. 22 risk beads have acceptance
criteria that restate the risk itself, so none of them can ever satisfy
their own AC and close. `bd` has a first-class deferred state (`bd update
--defer <date>`; `personal-cfo-n0yf2` is already in it) that no prior
process defined rules for. Six of the fifteen open P0 beads are epics, and
the existing ladder has no epic rule — an epic inherits no real urgency
from its priority field the way a task does, so grading it on the same
scale as a task is a category error.

Phase A (`personal-cfo-9lwtm`) is about to run a ~500-bead reconciliation
sweep over this graph. Without a written, attackable set of rules, that
sweep either re-derives judgment calls bead-by-bead (slow, inconsistent
across sessions) or applies an unstated house style no later session can
verify against. This ADR is that written rule set — the vocabulary and
tests Phase A, and every future audit, applies.

## Decision

### 1. Four close verdicts

Every bead closed during a reconciliation pass carries one of exactly four
close-reason prefixes:

- `shipped: <file/PR/crate/release>` — the work exists in the shipped
  product; name the concrete artifact that proves it, not just "done."
- `superseded: <what replaced it>` — a different bead, decision, or shipped
  behavior now covers what this bead asked for; name the replacement.
- `wont-do: <one sentence>` — a deliberate decision not to do this; the
  sentence is the reason, not a restatement of the bead's title.
- `absorbed:<id>` — this bead's remaining scope was folded into another
  open bead; name that bead's ID so the scope is traceable, not lost.

A close reason that uses none of these four prefixes is not a
reconciliation-pass close — it is an ordinary feature-complete close and
this rule does not apply to it.

### 2. Four survivor actions

A bead that is not closed during a reconciliation pass receives exactly one
of four actions, recorded in the pass's session log:

- **keep** — priority, parent, and acceptance criteria are all still
  accurate; no change.
- **regrade** — the priority changes. Read the bead's **live** priority
  before proposing a regrade (not the priority a stale plan draft assumed).
  Every regrade **down** is owned in the close reason or notes with the
  reason it no longer carries its old priority — a silent downgrade is not
  permitted.
- **reparent** — the bead's parent epic changes because the graph has
  drifted from what actually shipped (AGENTS.md §1A, "reconcile first").
- **rewrite** — the title, description, or acceptance criteria are
  materially wrong (describe an architecture that shipped differently) and
  are corrected in place rather than closed and recreated.

### 3. `defer` is a first-class disposition, with a two-strike rule

A bead with real future value but no present claim on priority is deferred,
not left open at a stale priority and not closed:

- Deferred beads carry a `defer:until-<milestone>` or `defer:no-demand`
  label, and `bd update <id> --defer <date>` binds the defer date to that
  milestone's boundary — not an arbitrary future date.
- **The two-strike rule:** a bead may be deferred **once**. At its deferral
  boundary, it is either regraded into a named milestone (a real priority,
  not another `defer`) or closed `wont-do:` with a one-sentence "reopen if
  `<condition>`" clause. A bead already carrying a `defer` label does not
  get a second one — the boundary forces a real decision.

### 4. The survival test

Applied to any bead whose value is in question: **does a program-plan epic
charter name this, or has dogfooding since 2026-09-14 produced a
user-visible need for it?** Yes → the bead survives at P3 (accepted
backlog). No → `defer` (§3), not `keep` at its old priority and not an
immediate close — a bead that fails this test today may still pass it once
a future charter or dogfooding session names it.

### 5. The risk-bead test

A risk bead is real, and stays open, only if **both** hold: the hazard it
names is live in the *shipped* product, and the bead names a verification
we do not yet have. Otherwise:

- The mitigation already shipped → `shipped: <proof>` (§1), citing the test,
  code path, or control that closes the gap.
- The hazard cannot exist yet (the feature it warns about hasn't shipped,
  or shipped in a shape the hazard doesn't apply to) → `superseded: <reason>`
  with a "reopen with `<trigger>`" clause, since the hazard may become real
  later.

A risk bead whose acceptance criteria only restate the risk itself (the
pattern that motivated this ADR — see Context) fails this test by
construction: it names no verification, so it cannot satisfy its own AC.
Rewrite (§2) its AC to name a real verification, or apply the test above.

### 6. The risk-priority rule

A risk bead's priority is never set independently — it carries the
priority of whichever bead mitigates it. If the mitigating bead's priority
changes, the risk bead's priority changes with it, and that regrade is
recorded as a note on the risk bead (not silently, per §2's regrade rule).

### 7. The post-launch priority ladder

Replaces AGENTS.md §9.1's milestone-named ladder:

- **P0** — data-safety, security, or release-blocking. Bypasses every
  queue; nothing outranks it.
- **P1** — on the critical path of the milestone currently in flight.
- **P2** — in the milestone currently in flight but off its critical path,
  or a prerequisite for the *next* milestone.
- **P3** — accepted backlog in a named charter (program-plan epic or
  dogfooding finding — the survival test in §4).
- **P4** — a placeholder awaiting a decision; it does not yet have an ADR
  that would let it be graded for real.
- **deferred `<boundary>`** — not a priority band; see §3.

### 8. The epic rule

Epics carry no priority meaning of their own — an epic's priority field
does not indicate urgency the way a task's does, since an epic's real
urgency lives in its children. Domain epics (the ones parenting ordinary
feature/task beads via `parent-child`, per AGENTS.md §9.1's three-axis
model) sit at a flat **P2** regardless of their children's priorities.
Phase and cross-cutting epics are unaffected by this rule; they continue to
use `tracks` edges and carry whatever priority reflects their own gate
status.

### 9. Process rules for running a reconciliation pass

- **One bead at a time, by hand.** A reconciliation pass never runs as a
  scripted loop across bead IDs (`personal-cfo-audit_no_loops`'s finding —
  a scripted loop cannot apply the judgment §1–§8 require per bead).
- **Batches of ~10, grouped by parent epic** — not by ID order, priority,
  or creation date — so a session's decisions stay contextually coherent.
- **Nothing is deleted.** Every disposition in §1–§3 is a close, a regrade,
  a reparent, a rewrite, or a defer; none of them is `bd` deletion.
- **Export and commit every session:** `bd export --include-memories -o
  .beads/issues.jsonl` (the flag is required — see AGENTS.md §9.0) and `bd
  dolt commit` at the end of any session that touches the graph, so a
  reconciliation pass is resumable and auditable across sessions the way
  `personal-cfo-56w.2`'s precedent established.

### 10. Known limitation: `bd ready` undercounts

`personal-cfo-u9ze6` (open, unresolved as of this ADR): `bd ready` omits
some open, unblocked, non-epic beads for a cause not yet root-caused, while
`bd blocked` names their `parent-child` parent as a blocker even when that
parent is open and siblings under the same parent *do* appear in `bd
ready`. A reconciliation pass — or any process — that treats `bd ready`'s
output as the complete set of claimable/open work will silently skip real
beads. The cross-check: run `bd blocked --json` alongside `bd ready` and
treat a bead listed there with an open `parent-child` parent as still
worth inspecting by hand, or use `bd show <id>` directly when a bead's
status is in doubt rather than trusting its absence from `bd ready` as
"already handled." This limitation does not block reconciliation work —
§9's "one bead at a time, by hand" rule already means every bead in a
batch is inspected individually via `bd show`, not sourced solely from `bd
ready`.

## Consequences

- **Positive.** Phase A (`personal-cfo-9lwtm`) and every future audit apply
  the same vocabulary, so two sessions reconciling different batches
  produce consistent verdicts instead of an unstated house style that
  drifts session to session.
- **Positive.** The risk-bead test (§5) gives the 22 stuck risk beads a way
  to actually close instead of accumulating indefinitely with
  unsatisfiable acceptance criteria.
- **Positive.** The two-strike rule (§3) stops `defer` from becoming a
  silent second `keep` — a bead cannot be deferred forever without a real
  decision being forced at its boundary.
- **Positive.** The epic rule (§8) resolves the six open-P0-epics anomaly
  the previous ladder had no answer for.
- **Negative / accepted.** The post-launch ladder (§7) is itself subject to
  drift the same way the milestone-named ladder it replaces did, if a
  future session grades against a stale "milestone in flight." Mitigation
  is procedural, not technical: §2's "read the live priority" rule applies
  to grading against this ladder too, not just to regrades.
- **Negative / accepted.** §10's `bd ready` limitation is inherited, not
  fixed, by this ADR — `u9ze6` stays open and reconciliation work absorbs
  the extra `bd show` verification cost until it's root-caused.
