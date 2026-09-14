# Launch calendar

Written 2026-09-11 (`personal-cfo-fkt5.10`), reconciling the owner-hour
plan drafted 2026-09-02/07 against actual bead-graph state as of today.

**Source of the hour totals below:** `personal-cfo-fkt5.10`'s own bead
description — this is the only place in the repo or `bd remember` that the
per-workstream owner-hour breakdown survived. The dedicated planning
artifacts referenced by that description (`launch-plan.json`,
`launch-runbook.md`) were session scratchpad files, never committed; only
the bead-ID keymap (`bd memories launch-plan-2026-09-02-keymap`) made it
into durable memory. This document is now the durable, committed home for
the hour math and critical path — read it instead of trying to
reconstruct the scratchpad.

**Requires owner sign-off.** Per `fkt5.10`'s own acceptance criteria, this
calendar needs the owner's initials recorded in that bead's notes before
the "launch calendar" clause of its AC is satisfied — drafting this file
does not itself complete that clause.

## Target dates

**OWNER DECISION 2026-09-13: go-live moved up to today, 2026-09-13.** The
T-14/T-7 staged-checkpoint plan below was the original pacing (owner
decisions 2026-09-05 and 2026-09-07, restated in `2owr`'s AC) and is kept
here for the record rather than deleted — it is **overtaken**, not wrong:
the readiness items it tracked (tag signing, updater round-trip, security
scan, mirror snapshot, etc.) were independently verified done today, which
is what made moving the date up possible.

**Update 2026-09-14:** slipped one day from the 2026-09-13 target above.
The 2026-09-13 go-live attempt surfaced `personal-cfo-7d6k9` (OWNER
DIRECTIVE: scrub every corporate/personal/internal-infrastructure
reference before the repository ever goes public) — a P0 that took two
review rounds to land clean, plus the owner's delete-and-recreate of
`dohflow/dohflow` (`tj3po`) that had to follow it. Actual go-live is today,
2026-09-14.

| Milestone | Date | Status |
|---|---|---|
| ~~T-14 (readiness checkpoint)~~ | ~~2026-09-17~~ | **Overtaken** — go-live moved earlier; see 2026-09-14 row below |
| ~~T-7 (final readiness checkpoint)~~ | ~~2026-09-24~~ | **Overtaken** — same |
| ~~Go-live (T-0), original~~ | ~~2026-10-01~~ | **Superseded** by the row below |
| ~~Go-live (T-0), 2026-09-13 attempt~~ | ~~2026-09-13~~ | **Superseded 2026-09-14** — the scrub (`7d6k9`) and repo recreate (`tj3po`) pushed it one day |
| **Go-live (T-0), actual** | **2026-09-14** | Owner moved the target to the earliest possible date once every readiness item was verified done; slipped one day for the pre-publish security scrub. |
| **Show HN floor** | 2026-10-02 | Unchanged — per `personal-cfo-n76x.21`'s actual warm-up start date (2026-09-04 + 28 days), independent of when go-live itself lands. **Not on the critical path** — `2owr`'s own AC states Show HN, Product Hunt, Reddit and the blog all follow go-live and are not gate items. |

## Owner-hour totals by workstream

≈47 hours total across six workstreams (W0–W5), per the original plan.
Bead counts below are fresh as of 2026-09-11 (`bd list --label plan:W<n>`),
not the 2026-09-07 snapshot — several items the original plan still
listed as open have since merged.

| Workstream | Hours | Beads | Open | In progress | Closed |
|---|---|---|---|---|---|
| W0 — Accounts & legal | ~7.7h | 9 | 2 | 0 | 7 |
| W1 — Placeholder site | ~1.9h | 7 | 0 | 0 | **7 (fully closed)** |
| W2 — Brand assets | ~5.9h | 18 | 7 | 2 | 9 |
| W3 — Full site | ~4.4h | 8 | 5 | 1 | 2 |
| W4 — Public repo & downloads | ~7.3h | 38 | 8 | 4 | 26 |
| W5 — Launch push (incl. two launch days + 28-day warm-up) | ~19.4h | 11 | 9 | 1 | 1 |
| **Total** | **~47h** | **91** | **31** | **8** | **52** |

### What's actually still open, per workstream

- **W0:** `personal-cfo-n76x.5` (Sponsors/Ko-fi placement — the Sponsors
  profile itself is approved and live; remaining work is placement and
  `FUNDING.yml`), `personal-cfo-n76x.8` (namespace/handle reservations).
- **W1:** nothing — fully closed.
- **W2:** `personal-cfo-4d8.28` (screenshot-ready build), `-4d8.28.5`
  (owner dogfooding round on the branded build), `-n76x.3` (branding:
  logo/palette/icon), `-n76x.3.4` (macOS app icon), `-n76x.3.7` (mark on
  dark surfaces), `-n76x.3.6` (illustration set), `-n76x.14` (screenshot
  automation spike); in progress: `-n76x.3.5` (web/social derivative
  set), `-n76x.13` (screenshot convention + capture set).
- **W3:** `personal-cfo-z5ag4` (owner: Workers Builds deploy hook —
  **the single blocker for `867.1.3` to become ready**), `-n76x.6`
  (website copy pass + **owner approval**, a Launch gate item),
  `-n76x.7` (site deploy pipeline + domain cutover), `-n76x.16` (press
  kit), `-n76x.25` (dohflow-site CI security scanning); in progress:
  `-n76x.15` (site IA/SEO plumbing).
- **W4:** `personal-cfo-867.1` (release-packaging parent), `-867.1.3`
  (release checklist/publish script — blocked only on `z5ag4`), `-867.1.4`
  (post-launch CI, not a gate item), `-fkt5` (fork-hygiene parent),
  `-fkt5.6` (CLA bot linking — **not started**), `-fkt5.9` (fork-day
  hardening), `-uxev1` (go-live day), `-7ie.7` (secrets-custody
  inventory); in progress: `-fkt5.10` (this bead), `-c8em` (SECURITY.md),
  `-n67eh` (known limitations — items 1+3 merged, 2 sub-items still
  deferred), `-fkt5.12` (mirror backups — script done, pre-flip
  timestamped run not yet performed, scheduled for T-14).
- **W5:** `personal-cfo-2owr.1` (pre-launch QA checklist, 0/1 complete),
  `-2ae0y` (organic launch plan — its own AC names
  `docs/product/launch-plan.md`, which does not exist yet, as a separate,
  narrower deliverable from this calendar), `-n76x.17` (blog),
  `-n76x.19` (launch assets/FAQ), `-n76x.20` (launch-day runbook — a
  *different* runbook from this bead's `public-launch-runbook.md`: that
  one covers Show-HN/PH day itself, this one covers the go-live flip),
  `-r36ck` (post-go-live dev migration), `-frkwu` (r/dohflow setup),
  `-h21nc` (Discord setup), `-n76x.22` (post-launch ops calendar); in
  progress: `-n76x.21` (community warm-up, started 2026-09-04).

## T-14 checklist, reconciled

The original 2026-09-07 plan text named nine items due by T-14
(2026-09-17). Checked against today's actual bead state:

| Item | Bead | Status |
|---|---|---|
| GitHub org `dohflow` reserved | `fkt5.1` | ✅ closed |
| ADR 0062 (fork mechanics) | `fkt5.2` | ✅ closed |
| CI public-repo triggers merged | `fkt5.7` | ✅ closed (PR #420, 2026-09-10) |
| Community + funding files merged | `fkt5.8` | ✅ closed |
| DohFlow rename merged | `fkt5.3` | ✅ closed (PR #393) |
| About card / opener grant merged | `n76x.18` | ✅ closed (PR #392) |
| LICENSE-swap PR merged | `fkt5.4` | ✅ closed (PR #446, **merged today, 2026-09-11** — the plan text said "open" as of 2026-09-07; it wasn't yet) |
| Coming-soon placeholder live | `n76x.9` | ✅ closed |
| Sponsors approved | `n76x.5` | ✅ Sponsors profile itself is live (`github.com/sponsors/chrisbustos`); parent bead stays open for remaining placement work |
| cla-assistant.io linked + tested on the private repo | `fkt5.6` | ✅ **updated 2026-09-13** — the gist is published and the cla-assistant.io flow is verified; only the post-flip "link to `dohflow/dohflow`" step remains, which is step 3 of `uxev1`'s own go-live-day sequence, not a T-14 precondition |
| Pre-flip mirror snapshot timestamp recorded | `fkt5.12` | ✅ **done 2026-09-13** — the one-time pre-flip run, now covering all four repos including `dohflow/dohflow` (added to `scripts/mirror-backup.sh`'s `REPOS`; see `personal-cfo-867.1.3` PR #453 review finding F1 and `fkt5.12`'s notes for the timestamp) |

**Updated 2026-09-13: 11 of 11.** Both rows above were still red as of
2026-09-11 (**Net: 9 of 11** at that time) — this table is otherwise
unchanged from that reconciliation. Both closed the same day the owner
moved go-live up to today; see the Target dates table and the Critical
path update below for why that was possible.

## Critical path

**As originally planned** (`fkt5.10`'s own description, plan-key form):
`W0-5` (trademark search, closed) clears within 3 weeks → `W4-5`
(TRADEMARK.md, closed) → `W4-4` (LICENSE PR, closed) → flip; separately,
`W5-5` (community warm-up, in progress since 2026-09-04) needs ≥28 days
before Show HN; on the site lane, `W4-3` (rename, closed) → `W2-4` (app
icon, **open**) → `W2-7` (screenshot capture, **in progress**) → `W3-2`
(website copy pass + owner approval, **open**).

**As of 2026-09-11, reconciled:** the LICENSE/trademark leg is fully
closed. What remained on the actual go-live critical path, at that time:

```
fkt5.6 (CLA gist + cla-assistant.io link, not started)
  → uxev1 (go-live day: snapshot push, CLA re-link, flip Public)
    → fkt5.9 (fork-day hardening)
      → 867.1.3 (tag v0.1.0, build, publish, dohflow-site rebuild)
```

The chain ends at `867.1.3`: `dohflow-site` (the marketing site's *source
repository*) is never made public (owner decision 2026-09-11, ADR 0061's
amendment, `personal-cfo-pedp9`) — only the deployed website, which is
already live at `dohflow.app` and simply rebuilds via the Workers Builds
deploy hook `867.1.3` triggers.

**Update 2026-09-13:** `personal-cfo-z5ag4` closed (2026-09-11), and
`fkt5.6`'s gist is published and its cla-assistant.io flow verified
(the final "link to `dohflow/dohflow`" step is still ahead, as step 5 of
`uxev1`'s own sequence, not a precondition to starting it — the earlier
`uxev1`→`fkt5.6` edge that implied otherwise was corrected). With every
other item in the chain independently verified done today, the owner
moved go-live to today rather than waiting for 2026-10-01 — see the
Target dates table above. This is the day the chain above actually runs.

The **site-copy lane** (`n76x.3.4` app icon → `n76x.13` screenshots →
`n76x.6` website copy + owner approval) runs in parallel. **Updated
2026-09-13:** `n76x.6` — the actual gate `2owr`'s AC item 9 names
("dohflow.app carries owner-approved copy, screenshots at final
branding") — is **closed**. `n76x.13` (screenshots) is in progress;
`n76x.3.4` (app icon) is still open. This lane does not block the
technical flip sequence above, and with its own gate item already
satisfied it is no longer a reason today's go-live would be premature —
the remaining two beads are polish, not a Launch-gate blocker.

## Roll-forward rule

Stated once here, referenced (not repeated) from
[`docs/operations/public-launch-runbook.md`](../operations/public-launch-runbook.md):
releases are immutable once published. A bad release is never deleted or
unpublished — every installed app's update check resolves to "the latest
release," and pulling one out from under that breaks the next check for
anyone still on it. Instead, a bad release is **superseded** by a higher
patch version. Rehearse this once, deliberately, on the throwaway smoke
repo (`dohflow/updater-smoke`) before it's ever needed for real.

## Graph reconciliation

`personal-cfo-fkt5.10`'s own description asked whether the owner still
needs to decide, item by item, on `5kua`/`g97y`/`bpl`/the 18 doc beads
that `sg8`'s acceptance criteria named as public-release blockers. That
decision was **already made** — `sg8` carries a 2026-09-08 note (dated
after this bead's 2026-09-03 creation) recording the owner's waiver for
all four groups in one sitting: the 18 architecture/product docs are
post-launch work, `g97y` (build reproducibility) is post-1.0 hardening,
`bpl` (the CI-blocking release gate) is replaced for v0.1.0 by the
internal evidence-based review (`personal-cfo-o1nxk`, closed), and `5kua`
(external security review) is deferred until revenue justifies it — also
recorded directly on `5kua` itself. None of the four appear anywhere in
`2owr`'s dependency list, confirming the waiver already took effect where
it matters: the Launch gate does not wait on them.

Two of `sg8`'s waived items (`g97y`, `bpl`) didn't carry their own
cross-reference note before this bead — only `sg8`'s umbrella note and
`5kua`'s own note existed. Added a one-line note to each pointing back at
`sg8`'s 2026-09-08 decision, so a future session opening either bead
directly doesn't have to rediscover the waiver by way of its parent.

Spot-checked `2owr`'s 59 dependency edges (`bd dep list personal-cfo-2owr`):
every one is typed `tracks`, none are the rejected default `blocks` type
for an epic↔non-epic pair. No fix needed.

**One AC clause this bead cannot literally satisfy:** `fkt5.10`'s own
acceptance criteria says `bv --robot-blocker-chain personal-cfo-2owr`
should "match the runbook order." Run today, that command reports:

```json
{"is_blocked": false, "chain_length": 0, "root_blockers": [], ...}
```

This is `2owr`'s own blocked/unblocked boolean (it has no blocking
*predecessor*, since these are informational `tracks` edges on an epic,
not hard `blocks` edges) — not an ordered walk of its 17 open or
in-progress dependencies (of 59 total). There is no ordering in this
output to match against the runbook. This is a mismatch between the
AC's wording and what this specific `bv` subcommand actually reports,
not something a different
invocation would fix — noted here rather than silently marked satisfied.
