# Bead structure

This document is for contributors (human or AI agent) who need to understand how the DohFlow bead corpus is organized. It complements `AGENTS.md` §9 (which covers the bead CLI mechanics) by documenting the **conventions** we use here.

If you're new and need to find work to do, start with `bd ready`. If you want to understand why the corpus is shaped the way it is, read on.

---

## 1. The big picture

The bead tracker (`bd`, [beads](https://github.com/steveyegge/beads)) is the project's task source of truth. **All work is tracked as beads.** TodoWrite, TaskCreate, markdown TODOs, and the like are explicitly forbidden — they fragment context across tools and don't survive sessions.

As of the bootstrap completion: the corpus has ~555 open beads spanning the project from Week 8 first-playable through Phase 7 OSS release. Every bead has concrete, verifiable acceptance criteria. There are zero priority inversions and zero hollow epics.

---

## 2. Three epic axes

Every non-epic bead hangs off **exactly one** parent epic via a `parent-child` dependency. But each bead is _associated_ with multiple aggregations through different mechanisms:

```
                 ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
                 │  Phase epics    │  │  Domain epics   │  │ Cross-cutting   │
                 │  (when?)        │  │  (what?)        │  │  epics (how?)   │
                 └────────┬────────┘  └────────┬────────┘  └────────┬────────┘
                          │                    │                    │
                  via tracks deps       via parent-child       via parent-child
                  (gate aggregation)    (1 parent per bead)    (1 parent per bead)
```

Most beads have one `parent-child` parent (a domain epic OR a cross-cutting epic, not both) and zero or more phase associations through gate-checklist epics' `tracks` deps.

### 2.1 Phase epics

Time-bucketed work organized by §22 of the project plan. Eight phases across roughly 60 weeks. Each phase has a **gate-aggregator epic** that tracks its deliverable beads via `tracks` deps:

| Phase epic | Phase | When | Gate-aggregator epic |
|---|---|---|---|
| `personal-cfo-6s7` | Phase 0 — Foundation | Weeks 1–6 | `personal-cfo-6s7` (epic itself) |
| `personal-cfo-1ik` | Phase 0.5 — Week 8 first playable | Weeks 7–8 | `personal-cfo-rtez` |
| `personal-cfo-hlh` | Phase 1 — Local manual cash forecast | Weeks 9–14 | `personal-cfo-qmqk` |
| `personal-cfo-nq2` | Phase 2 — Importable local finance tracker | Weeks 11–16 | `personal-cfo-1xz3` |
| `personal-cfo-8o3` | Phase 3 — Learning forecast | Weeks 17–26 | `personal-cfo-jt21` |
| `personal-cfo-bq1` | Phase 4 — Connectors + documents | Weeks 27–36 | `personal-cfo-bq1` (epic itself) |
| `personal-cfo-x4s` | Phase 5 — Investments + debt + agents | Weeks 37–46 | `personal-cfo-x4s` (epic itself) |
| `personal-cfo-867` | Phase 6 — Polish, hardening, beta | Weeks 47–54 | `personal-cfo-867` (epic itself) |
| `personal-cfo-apc` | Phase 7 — OSS release prep | Weeks 55–60 | `personal-cfo-apc` (epic itself) |

Phases 0.5 / 1 / 2 / 3 use a **separate gate-checklist epic** (`rtez`, `qmqk`, `1xz3`, `jt21`) that tracks deliverables and serves as the gate-pass acceptance bead. Phases 0 / 4 / 5 / 6 / 7 use the phase epic itself as the aggregator.

### 2.2 Domain epics

What kind of work, regardless of phase. Beads hang off these via `parent-child`:

- `personal-cfo-7ie` — Local security and privacy architecture
- `personal-cfo-klr` — Canonical data model and persistence layer
- `personal-cfo-5ie` — Future Cash forecasting engine (the product wedge)
- `personal-cfo-4d8` — UX and information architecture
- `personal-cfo-g3m` — Reliability, sync, and data quality
- `personal-cfo-3b8` — Income modeling
- `personal-cfo-5n4` — Transaction categorization and learning
- `personal-cfo-6wk` — Spending, credit, and debt management
- `personal-cfo-pxi` — Connector adapters and self-hosted relay
- `personal-cfo-uz0` — Documents module
- `personal-cfo-9h0` — Investments and net worth
- `personal-cfo-k7s` — Budgeting and goals
- `personal-cfo-4ai` — AI agent system

### 2.3 Cross-cutting epics

Work that touches every phase and domain:

- `personal-cfo-915` — Strategic decisions, architecture, and product philosophy (ADRs live here)
- `personal-cfo-hs4` — Technology stack standardization
- `personal-cfo-56w` — Testing and quality strategy
- `personal-cfo-sg8` — Open source and release strategy

---

## 3. The gate-tracks pattern

Phase progress isn't tracked through `parent-child` reparenting; it's tracked through **gate epics** that hold `tracks` deps to the deliverable beads. This keeps individual beads' parenting stable (a vault bead stays parented under the security domain epic) while still letting gate-pass acceptance be a clean operation.

```
Gate Week 8 (rtez)
├─ tracks → 164u (Layer-1 Future Cash ledger)        ← parented under 5ie Forecast
├─ tracks → 3ry  (vault create flow)                 ← parented under 7ie Security
├─ tracks → ef3  (encrypted backup export v1)        ← parented under g3m Reliability
└─ tracks → 14 more deliverable beads
```

A gate is "passed" when every tracked bead is closed. The gate epic's own AC asserts this: see `personal-cfo-rtez`'s acceptance criteria for the canonical pattern.

Tracks are **aggregation** deps; they don't create blocking. A gate epic can reference (and require closure of) a deliverable that is also `parent-child`-parented under an unrelated domain epic. This is intentional: deliverables move through phases without losing their domain identity.

---

## 4. Acceptance criteria discipline

Every bead's `acceptance_criteria` field has concrete, verifiable content. **No "Implementation matches the description above"** boilerplate anywhere in the corpus.

What "verifiable" means depends on the bead type:

| Bead type | AC must name |
|---|---|
| **Schema** | The migration creates specific tables/columns/constraints; specific invariants (e.g., posting balance) enforced; rebuild test on golden fixture produces stable hash |
| **Task** (impl) | Specific test names; performance budget numbers; CI gates; specific command/file/IPC entry points |
| **Feature** (wrapper) | Subset of child task closures; named end-to-end smoke test; integration test against fixture vault |
| **ADR (decision)** | The `docs/adr/00NN-slug.md` file exists; ≥1 rejected alternative with reason; "revisit if..." trigger documented |
| **Epic** | Every tracks/child closure; phase/release-gate criteria; retro note appended pre-close |

When in doubt, ask: "if I read this AC in 6 months, can I tell whether the bead is done?" If yes, it's good. If no, sharpen it.

### 4.1 The Definition of Done

`docs/architecture/definition-of-done.md` (tracked by `personal-cfo-3hsx`, closed) is the canonical reference for **per-feature test obligations**: required test layers (unit / integration / E2E / snapshot / property), required logging instrumentation, redaction policy, performance budgets, CI gates.

Per-feature acceptance criteria reference the DoD by path rather than restating it. This replaced 267 templated "Tests + logging:" beads that were collapsed into per-feature AC during the Tier A+B cleanup.

---

## 5. Priority discipline

Five priorities, with strict semantics:

| Pri | Meaning | Example |
|---|---|---|
| **P0** | Safety-critical or release-blocking | Vault encryption, ledger invariants, redaction CI |
| **P1** | Week-8 first-playable foundation, or Phase 1 critical-path | Layer-1 forecast, account schemas, manual bill UI |
| **P2** | MVP-1 manual/import functionality, or Phase 2 critical-path | CSV importer, recurring detection v1, Money Inbox items |
| **P3** | MVP-2/3 features, or Phase 3-5 critical-path | Local classifier, agent system, investment views |
| **P4** | Release prep, polish, future bets | Pre-release reviews, OSS docs, deferred adapters |

### 5.1 Inversion rule

**No higher-priority bead `blocks` on a lower-priority bead.** If you find one:

1. Bump the blocker up (most common).
2. Defer the blocked work (less common — only when the dep is genuinely the wrong direction).
3. Reverse the dep direction (rare — only when the dep was authored backward; e.g. v0 should not block on v1).

The corpus is currently inversion-free. Run `bd export` + a Python check (or grep the JSONL) to verify after any priority change.

### 5.2 Risk: beads

"Risk:" beads track risks, not work. Their AC follows a **monitored** pattern: "monitored; mitigation tracked by bead X; resolution criteria Y; revisit if Z." They do **not** depend on the work that mitigates them — that direction is wrong (the mitigator doesn't depend on the risk; the risk is mitigated by the work). Risk beads exist as searchable tracking surfaces.

Because they don't depend on mitigators, Risk beads **do not appear in any gate epic's transitive closure**. They live under the Strategic Decisions epic (`personal-cfo-915`) via parent-child and surface in `bd search "Risk:"`. This is intentional: a gate is "passed" when its work is done, not when its risks are eliminated (risks are continuous monitoring concerns).

---

## 6. ADRs

Architectural decisions live in `docs/adr/` as numbered Markdown files (`00NN-slug.md`). Each ADR has:

- Context — why this decision is needed.
- Decision — the chosen path.
- Consequences (positive + negative).
- Rejected alternatives, each with the reason it was rejected.
- "Revisit if..." trigger conditions.
- Linked beads.

If the project has a public/private disclosure boundary for its ADRs (this project's is ADR 0082): tier decided and recorded **before** the file is written — an ADR bead's own acceptance criteria must say which tier it is, since that decides which repository the file gets written into.

The corpus tracks ADRs as `decision`-type beads. As of bootstrap completion:

**Written and merged** (8): 0001 Tauri+Rust+React, 0002 vault, 0003 trust boundary, 0006 Finance Kernel, 0007 ledger/posting, 0009 read-model strategy, 0011 hybrid ledger/op-log, 0012 idempotency.

**Scoped with concrete AC** (10): 0004 connector relay, 0005 Future Cash wedge, 0008 staged ingestion, 0010 Tauri capability isolation, 0013 parser isolation, 0014 Money Inbox, 0015 connector capability registry, 0016 headless CLI safety, 0017 sync-readiness, 0018 forecast non-advice, 0019 license, 0020 OCR engine choice. Each ADR bead's acceptance criteria specify the exact deliverable shape; writing the ADR is a single-bead piece of work.

A new ADR is needed when an architectural choice will be hard to revisit later. New ADRs MUST list rejected alternatives — that's how we avoid re-litigating decisions.

---

## 7. Persistence and session mechanics

The bead corpus lives in `.beads/` (a Dolt-backed SQLCipher store) and is exported to `.beads/issues.jsonl` as the durable git artifact.

End-of-session protocol:

```bash
git pull --rebase
bd dolt push          # push beads-DB changes to Dolt remote
git push              # push the JSONL export
git status            # MUST show "up to date with origin"
```

Persistent knowledge across sessions (insights, lessons learned, decisions made by an agent) lives in `bd remember "..."` (searchable via `bd memories <keyword>`). **Do not introduce `MEMORY.md` files** — they fragment across accounts and sessions.

---

## 8. Common operations cheat-sheet

```bash
# Find unblocked work
bd ready
bd ready --priority=0          # P0 only
bd ready --json                # for scripting

# Inspect
bd show <id>                   # full bead view
bv --robot-blocker-chain <id>  # dependency chain (read-only TUI)

# Claim work atomically
bd update <id> --claim         # sets assignee + status=in_progress

# Update fields without opening $EDITOR
bd update <id> --priority=0
bd update <id> --acceptance="<verifiable AC>"
bd update <id> --add-label=area:vault
bd update <id> --parent=personal-cfo-7ie

# Dependencies
bd dep add <blocked> <blocker>            # default type: blocks
bd dep add <a> <b> -t tracks              # gate aggregation
bd dep remove <a> <b>                     # remove
bd dep <blocker> --blocks <blocked>       # alternative syntax

# Close
bd close <id> --reason="..."
bd close <id1> <id2> ... --reason="..."   # batch
bd close <id> --force --reason="..."      # bypass blocker check (e.g., for duplicates)

# Health checks
bd lint                # template / structure warnings
bd orphans             # broken dependency targets
bd stale               # no recent activity
bd preflight           # PR-readiness checklist

# Persistent memory
bd remember "..."
bd memories <keyword>
```

**Forbidden**: `bd edit` (opens `$EDITOR`, blocks agents); bare `bv` (opens interactive TUI). Always use `bd update --field=...` and `bv --robot-*` flags.

---

## 9. When the corpus needs a structural change

Sometimes a walk surfaces a structural issue: a hollow gate epic, a wrong-direction dep, a duplicate bead, an anti-task that shouldn't exist as a task. These are real problems and the corpus is allowed to evolve.

The audit-and-walk pattern (used 8 times during bootstrap):

1. **Map** — read the relevant plan section; list the deliverables.
2. **Search** — `bd search` to find existing beads for each deliverable.
3. **Wire** — add `tracks` deps from the gate epic to the deliverable beads.
4. **Bump priorities** — fix any inversions surfaced.
5. **Walk closure** — for each bead in the gate's transitive closure, write concrete AC.
6. **Reverse wrong deps** — inversion analysis catches semantic dep errors (extension blocking on base, future-phase blocking on current-phase, risk blocking on mitigator).
7. **Close noise** — duplicates, anti-tasks, deferred-as-tasks-when-they-should-be-notes.
8. **Verify** — run `bd lint`, `bd orphans`, run a Python inversion check, confirm zero canned AC remaining.
9. **Commit + push + open PR**.

Each walk PR has a detailed commit message + a `bd remember` memory entry capturing what changed. The audit trail is preserved indefinitely.

---

## 10. Final corpus state at bootstrap completion (2026-05-05)

For reference — the corpus shape after the 7-step bootstrap audit + walks:

- **555 open beads** (82 P0 / 120 P1 / 190 P2 / 102 P3 / 61 P4)
- **Zero priority inversions** corpus-wide
- **Zero canned-boilerplate AC** — every bead has concrete, verifiable acceptance criteria
- **Zero unparented non-epic beads** — every bead has a `parent-child` parent epic
- **9 gate-aggregator epics** wired with tracks deps (Phase 0, Week 8, R1, R2, R3, Phase 4, 5, 6, 7)
- **30 epics** across 3 axes (9 phase + 13 domain + 4 cross-cutting + 4 gate-checklist)
- **184 beads not in any gate's transitive closure** — these are intentional exceptions: 21 Risk beads (tracked under Strategic Decisions), 1 sync-readiness ADR (post-MVP), and ~162 P2/P3/P4 backlog work (post-MVP polish, OSS prep, lower-priority extensions, deferred adapters)
- **8 ADRs written and merged** under `docs/adr/` (0001 / 0002 / 0003 / 0006 / 0007 / 0009 / 0011 / 0012); 11 more scoped with concrete AC ready for writing.
- **303 beads closed** through the bootstrap audit (267 templated test bundles + 19 deferred anti-tasks + 3 duplicates + 8 ADRs + DoD bead + 5 misc closures + others).
- **DoD doc** at `docs/architecture/definition-of-done.md` (test layers, logging, redaction, perf budgets, CI gates, real-data safety gate).
- **`AGENTS.md` §9 + §9.1** documents the bead conventions in agent-facing form.

This baseline is auditable: every bead change in the bootstrap was committed with a detailed message + a `bd remember` memory entry, and the bead JSONL is the durable git artifact.

---

## 11. References

- **Project plan**: `docs/planning/personal-finance-app-project-plan.md` (the source-of-truth for phases, deliverables, gate criteria).
- **AGENTS.md** §9 + §9.1: agent-facing bead conventions.
- **DoD**: `docs/architecture/definition-of-done.md` — per-feature test/logging obligations.
- **ADRs**: `docs/adr/` — written-and-merged architectural decisions.
- **Project profile**: `docs/agent/PROJECT_PROFILE.md` — stack pins, quality gates, real-data safety gate.
- **Beads CLI docs**: `bd prime` (in-terminal); upstream at https://github.com/steveyegge/beads.
