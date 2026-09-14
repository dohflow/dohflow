# Re-evaluation report — 2026-05-06

Fresh-eyes pass over the bead corpus after the 8-walk bootstrap completed. Goal: catch what the narrow walks may have missed.

## TL;DR

Corpus is in good shape overall — every structural diagnostic is clean (0 inversions, 0 cycles, 0 unparented, 0 canned AC, 0 missing AC). But targeted parent-assignment audits surfaced **20 specific corrections** worth making, plus **5 placeholder-AC beads** that need concrete content.

Plan §22 deliverable coverage through R3 is 100%. Every plan deliverable has a matching bead.

## Diagnostics (clean)

| Check | Result |
|---|---|
| `bd lint` | ✓ No template warnings (555 issues checked) |
| `bd orphans` | ✓ No orphaned issues |
| `bd stale` | ✓ No stale issues |
| Priority inversions (corpus-wide) | 0 |
| Cycles in blocks-deps graph | 0 |
| Unparented non-epic beads | 0 |
| Beads with multiple parent-child parents | 0 |
| Canned-boilerplate AC | 0 |
| Beads without AC | 0 |
| AC length distribution | min 39, median 332, max 1244 |

## Sample-walk (30 random beads)

Spot-check: 30 random open beads sampled across all priorities. Each verified for parent presence, concrete AC, sensible blocks-deps. **0 of 30 had any structural flag.**

## Plan §22 deliverable coverage

Every §22 R1/R2/R3/Week-8 deliverable mapped to a bead:

- **Phase 0.5 / Week 8 (9 deliverables)**: 9/9 covered
- **Phase 1 / R1 (10 deliverables)**: 10/10 covered
- **Phase 2 / R2 (7 deliverables)**: 7/7 covered
- **Phase 3 / R3 (17 deliverables)**: 17/17 covered

## Findings — corrections to apply

### F1. Money Inbox items mis-parented (7 beads)

The bulk reparenting earlier mapped `area:money-inbox` → `5n4` (Transaction categorization). That was wrong — Money Inbox items are typed projections per ADR 0014; their parent should be the domain whose canonical state produces the inbox item, not categorization.

| Bead | Title | Current parent | Should be |
|---|---|---|---|
| `58t5` | Money Inbox item: reconciliation discrepancies | 5n4 | `g3m` (Reliability) |
| `asqy` | Money Inbox item: imported transactions waiting commit | 5n4 | `pxi` (Connectors/ingestion) |
| `r52x` | Money Inbox item: stale balances | 5n4 | `g3m` (Reliability) |
| `ruo9` | Money Inbox item: document extractions needing confirmation | 5n4 | `uz0` (Documents) |
| `y8rq` | Money Inbox item: possible recurring bills | 5n4 | `esmy` (Recurring feature) |
| `ykdv` | Money Inbox item: forecast assumptions needing attention | 5n4 | `5ie` (Forecast) |
| `zfyo` | Money Inbox item: connector errors | 5n4 | `pxi` (Connectors) |

`pi54` (possible transfers) and `uc95` (low-confidence categories) stay at `5n4` — those genuinely are categorization concerns.

### F2. Other parent-assignment errors (8 beads)

Heuristic title-vs-parent audit surfaced these as real mismatches:

| Bead | Title | Current → Suggested |
|---|---|---|
| `5ivp` | Vault recovery wizard for failed health checks | `g3m` → `4d8` (it's a UX wizard) |
| `2no` | Tauri window/webview model: main / document_preview / agent_report / external_auth | `4d8` → `915` (architecture decision, not UX) |
| `jah` | Sanitize SVG/markdown/HTML-like document text and LLM markdown | `4d8` → `7ie` (security/redaction) |
| `n9w` | Vault health check command | `g3m` → `7ie` (vault security) |
| `8cg2` | Test harness: automated accessibility (axe-core) in CI | `4d8` → `56w` (test harness) |
| `1srf` | Test harness: fault-injection framework | `g3m` → `56w` (test harness) |
| `hbd8` | Plaintext CSV transaction export | `g3m` → `pxi` (export/ingestion domain) |
| `3ru` | Establish + enforce query/UI performance budgets | `g3m` → `56w` (testing perf budgets) |

### F3. Schema bead under feature wrapper (1 bead)

`dppg` (Schema: recurring_event_instances) is parented to `esmy` (Recurring feature wrapper). Per convention all schemas are parented under `klr` (data model). Move `dppg` → `klr`.

### F4. Placeholder-AC beads (5 beads)

These have AC field populated but the content is just a pointer to where AC was defined ("Already specified in earlier batch"). The AC field should contain the actual concrete criteria, not a redirect.

| Bead | Title | AC content |
|---|---|---|
| `2lnn` | Doc: docs/architecture/connector-relay.md | "(Already specified in earlier batch.)" — 39 chars |
| `7oax` | Doc: docs/architecture/connector-provider-registry.md | "(Already specified in earlier batch.)" — 39 chars |
| `n9uc` | Pre-release review: connector relay | "(Already specified in earlier batch.)" — 39 chars |
| `eqfw` | Forecast input snapshots + model registry | "(Defined in R2 walk; transitively in R3 closure too.)" — 55 chars |
| `8g9j` | Agent tests: schema validation, prompt-injection, cost caps | "(Already covered in cross-cutting walk; reaffirmed for Phase 5 closure.)" — 74 chars |

These 5 beads need real concrete AC.

## Findings — observations (no action)

### O1. Synthetic persona fixtures parented to Testing epic

Six persona-fixture beads (`0ff1` contractor, `1o1x` power-user, `5z09` hourly, `x447` couple, `yl6u` salaried, `ytyq` investor) all parent to `56w` (Testing epic). That's defensible — they're test fixtures. An alternative would be a dedicated synthetic-data epic, but creating one for 6 beads isn't worth the structural cost. **Leave as-is.**

### O2. Connector adapter "placeholder" beads

`fl3a` (MX connector placeholder) and `47xg` (Mastercard Open Banking placeholder) exist as anchor points for future-work that may or may not happen. Their AC is honest: "Placeholder bead for X connector. Deferred until a clear user case appears."

These are mildly anti-pattern (a real bead should be a piece of work; a placeholder isn't), but closing them and recreating later is more bookkeeping than benefit. **Leave as-is.**

### O3. ADRs not yet written: 12 of 20 are at P1

8 ADRs are written and merged (0001 / 0002 / 0003 / 0006 / 0007 / 0009 / 0011 / 0012). 12 remain open with concrete AC: 0004, 0005, 0008, 0010, 0013, 0014, 0015, 0016, 0017, 0018, 0019, 0020 (the OCR ADR `nqii`).

ADR `tif` (0010 Tauri capability isolation) is correctly at P0 — gates capability work. The other 11 are P1 or P4 which is correct given when they'll be needed.

**No action; this is the expected post-bootstrap shape.**

### O4. Doc beads under `sg8` could plausibly live under domain epics

The 18 `Doc: docs/.../*.md` beads under `sg8` (OSS strategy) are arguably better parented under the domain they document. But "release docs" as a coherent group is also a valid grouping. The DoD doc + bead-structure doc + AGENTS.md set a clear "what docs exist" picture; further re-parenting is bookkeeping with no clear benefit.

**Leave as-is.**

### O5. 184 beads outside any gate-aggregator closure

Composition:
- 21 Risk: beads (intentionally outside per documented Risk-bead exception)
- 1 post-MVP sync ADR (`6kn` ADR 0017)
- ~162 P2/P3/P4 backlog work (post-MVP polish, OSS prep, deferred adapters)

This is the expected pattern: gate closures cover the work that gates depend on; backlog work that has no current gate to gate doesn't need to be tracked by one.

**Leave as-is.**

### O6. Title-prefix patterns

Distribution of title prefixes across the corpus:
- 36 "Schema:" beads — all under `klr` per convention ✓ (after F3)
- 26 "Risk:" beads — all under `915` per convention ✓
- 26 "Doc:" beads — mostly under `sg8` per O4
- 20 "Cross-cutting test" — all under `56w` per convention ✓
- 12 "Structured logging spec:" — distributed across domain epics ✓
- 10 "Read model:" beads — all under `klr` per convention ✓
- 10 "Money Inbox" + "Insight" — distributed across domains (after F1)
- 9 "FEATURE:" — under appropriate domains ✓
- 9 "Pre-release review" — under `7ie` per convention ✓
- 8 "UX:" — under `4d8` (UX) per convention ✓
- 6 "Synthetic persona" — under `56w` per O1
- 6 "Notification trigger" — under `4d8` ✓

The distribution is consistent. No surprise patterns.

## Findings — pattern verification

### V1. Gate epic AC names §22 deliverables explicitly

Spot-checked all 9 gate epic AC fields. Each correctly names the §22 deliverables it gates. Examples:

- `rtez` (Week-8 gate): names vault create + unlock, manual accounts, manual transactions, manual salary, manual bills, deterministic Future Cash, dashboard, encrypted backup, real-data safety gate.
- `qmqk` (R1 gate): names CSV import + column mapping, source records + provenance links, staged candidate review, etc.
- `1xz3` (R2 gate): names hourly + contractor income, recurring detection v1, manual future entries, etc.
- `jt21` (R3 gate): names Variable spending forecasts (Layer 2), Credit-card forecast, Confidence bands, etc.

✓ Consistent pattern across all gate epics.

### V2. Risk-bead exception is consistently applied

All 26 Risk: beads:
- Have AC following the "monitored; mitigation tracked by X; resolution criteria Y; revisit if Z" pattern ✓
- Do NOT have `blocks` deps on the work that mitigates them ✓
- Are parented under `915` (Strategic Decisions) ✓
- Do not appear in any gate epic's transitive closure ✓

✓ Consistently applied.

### V3. ADR scoping is consistent

All 12 unwritten ADRs have AC that names a `docs/adr/00NN-*.md` file path. ✓

## Recommendations

**Apply the 20 corrections in F1 + F2 + F3 + F4** in one focused PR. They're all small (re-parenting and AC content writes), they don't change priorities or close anything, and the corpus stays in the same final-state shape.

After that, the bead corpus should be ready for actual implementation work.

## Out-of-scope for this re-evaluation

Things explicitly NOT covered:

- ADR writing (the 12 unwritten ADRs are tracked beads with concrete AC; they'll be written when their phase starts).
- Per-feature implementation guidance — that's in the bead AC + DoD.
- Phase 4-7 deep audit beyond the 7c + 7d walks (deferred until those phases approach).
- Code style or test guidance — covered by `docs/architecture/definition-of-done.md`.

## Process notes

The fresh-eyes pass found ~25 corrections across ~555 beads (~4.5% drift rate from the documented model). For an 8-walk bootstrap, that's an acceptable drift; the audits caught the major issues (priority inversions, missing AC, hollow gates, over-decomposed test bundles).

The dominant pattern of error: **bulk-reparenting heuristics in the cross-cutting walk over-mapped some labels to wrong epics.** Specifically the `area:money-inbox` → `5n4` mapping was wrong — Money Inbox items belong to the *domain that produces the canonical state*, not categorization. This was caught by the title-vs-parent heuristic during the re-evaluation.

Lesson for any future structural work: always validate auto-generated parent assignments against per-bead titles, not just labels.
