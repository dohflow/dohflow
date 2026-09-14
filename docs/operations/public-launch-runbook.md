# Public launch runbook

- Beads: `personal-cfo-fkt5.10` (this doc), `personal-cfo-uxev1` (go-live
  day itself), `personal-cfo-fkt5.9` (fork-day hardening),
  `personal-cfo-fkt5.6` (CLA bot), `personal-cfo-fkt5.12` (mirror
  backups), `personal-cfo-867.1.3` ([release checklist](release-checklist.md)
  + `scripts/publish-release.sh`).
  Dates and hour totals: [`docs/product/launch-calendar.md`](../product/launch-calendar.md)
  — read that first; this doc is the sequence, not the schedule.
- **Not the Show HN / launch-week runbook.** `personal-cfo-n76x.20`
  covers what happens after the app is public (Show HN day, Product Hunt
  day, hotfix standby). This doc covers only getting the repository and
  `v0.1.0` public in the first place — everything up through the flip.

This doc **assembles and sequences already-decided procedures; it does
not invent new process.** Every step below links to the bead or doc that
is the actual source of truth for its mechanics. Where a stale note
existed on a linked bead, that's called out explicitly rather than
silently carried forward — see the footnote after the T-0 sequence.

## T-14 (2026-09-17) — superseded, kept for the record

**Owner decision 2026-09-13: go-live moved up to today**, once every
readiness item below (and everything in the T-7 section) was
independently verified done — see
[launch-calendar.md's Target dates](../product/launch-calendar.md#target-dates).
This section's original date is overtaken, not wrong: it's the checklist
that made moving the date up possible. The one item this section
originally scheduled for T-14 that is genuinely part of go-live day
itself — the pre-flip mirror backup — now lives in the **T-0 sequence
below (step 5)**, not here, so a reader working through go-live day sees
it in place rather than dated four days in the past.

The full reconciled checklist — which of the nine originally-planned
items were done as of 2026-09-11 — lives in
[launch-calendar.md's T-14 section](../product/launch-calendar.md#t-14-checklist-reconciled).
In short, as things stood before today's move: 9 of 11 tracked items were
done, ahead of schedule. Two remained:

- **`personal-cfo-fkt5.6`** — link the CLA gist to cla-assistant.io on
  the private repo, test with a second account. Owner-browser, ~20 min.
  As of 2026-09-13 the gist is published and the cla-assistant.io flow is
  verified; the remaining piece — linking `dohflow/dohflow` itself — only
  becomes possible once that repository is public, so it is step 3 of the
  T-0 sequence below, not a precondition to starting it.
- ~~**`personal-cfo-fkt5.12`** — run the mirror-backup script manually,
  record the timestamp.~~ **Done 2026-09-13**, as part of today's go-live
  sequence (T-0 step 5) rather than on the original T-14 date — see that
  bead's notes for the timestamp and per-repo results.

## T-7 (2026-09-24)

- Updater round-trip + tamper-test: already done
  (`personal-cfo-867.1.2`, closed — live smoke-repo round trip and both
  negative tests passed, see that bead's notes for the exact evidence).
- Tag signing: already configured and verified
  (`personal-cfo-867.1.3`'s 2026-09-07 note — SSH signing key registered,
  a throwaway tag showed "Verified" on github.com).
- `v0.1.0` draft smoke-tested on a second Mac, or a Tart VM if that Mac
  can't run the minimum macOS version
  ([`release-checklist.md`](release-checklist.md)'s step 5 has the full
  checklist: Gatekeeper dialog text, `spctl` output, `LSMinimumSystemVersion`,
  a full vault → import → relaunch → backup/restore cycle).
- `security-scan` CI job green; a full-history `gitleaks` re-run clean.

## T-0 — go-live day

This is `personal-cfo-uxev1`'s own sequence, which is the current,
owner-approved plan (snapshot approach, ADR 0062's 2026-09-08 amendment)
— superseding any earlier transfer/rename/history-rewrite plan.[^stale-note]

1. **Owner merges `personal-cfo-fkt5.4`** in the private repo (LICENSE,
   CLA.md, TRADEMARK.md, package metadata). *Already true as of
   2026-09-11 — this step is complete ahead of go-live day itself.*
2. **Agent produces the snapshot and pushes one commit** to
   `dohflow/dohflow`'s `main` (still private at this point). Full
   procedure, preconditions, and verification commands:
   [`docs/operations/public-launch-snapshot.md`](public-launch-snapshot.md)
   — not repeated here.
3. **Owner links cla-assistant.io** to the new repository (same gist as
   the private-repo link), grants the OAuth app org access, and re-runs
   the second-account test PR (`personal-cfo-fkt5.6`'s "post-transfer
   re-check" step).
4. **Agent tags, builds, and drafts `v0.1.0`** on the snapshot commit,
   built from the new clone — not the private repo's checkout — so the
   app's About-card commit SHA matches what a public user can actually
   see and clone ([`release-checklist.md`](release-checklist.md) steps
   1-4). Owner smoke-tests the draft release on the second Mac while
   it's still a private draft (step 5).
5. **Agent runs the pre-flip mirror backup** (`./scripts/mirror-backup.sh`,
   `personal-cfo-fkt5.12`) and records the timestamp in that bead's
   notes — **before** the visibility flip in the next step, not after.
   GitHub is the only copy of the repos plus their issues/releases
   metadata, and a transfer, rename, or visibility flip is exactly the
   kind of operation that occasionally goes wrong; this is the backup the
   plan puts in front of that risk. Confirm the mirror covers all four
   repos, `dohflow/dohflow` included (added to the script's `REPOS` list
   — it was previously missing despite existing since 2026-09-09).
6. **Owner flips visibility**: Settings → Danger Zone → Change
   visibility → Public.
7. **Fork-day hardening clicks** (`personal-cfo-fkt5.9`): private
   vulnerability reporting first, then secret/push protection,
   Dependabot, branch ruleset on `main`, immutable releases, issue
   labels created *before* the issue-form render check (a label a form
   references but that doesn't yet exist on the repo is silently
   dropped and never retroactively applied), social preview, About
   section, "keep my email private."
8. **Publish `v0.1.0`** (flip the draft release off draft), trigger the
   `dohflow-site` rebuild (Workers Builds deploy hook,
   `personal-cfo-z5ag4`; run with
   `./scripts/publish-release.sh publish` — [`release-checklist.md`](release-checklist.md)
   step 6), verify `curl -sI .../releases/latest/download/latest.json`
   returns 200.
9. **Add the GitHub Security Advisories URL and a `Policy:` line** to
   `dohflow-site`'s `public/.well-known/security.txt`, now that
   `dohflow/dohflow` is public (step 6) — confirmed as of 2026-09-11,
   that file's own comment already says both are deferred to exactly
   this point: *"The GitHub advisories link is added on fork day, once
   the repository is public (personal-cfo-fkt5.9). Until then email is
   the only channel."* **`dohflow-site` itself is never made public**
   (owner decision 2026-09-11, ADR 0061's amendment,
   `personal-cfo-pedp9`) — this step only edits a file the still-private
   repository publishes to the already-public `dohflow.app`.

[^stale-note]: `personal-cfo-uxev1`'s own notes carry one entry (dated
    2026-09-08, titled "NEW PRECONDITION") describing a requirement that
    GitHub Support confirm a purge of cached views and pull-request refs
    before flipping visibility. That precondition belongs to the
    **abandoned history-rewrite plan** — it directly contradicts
    `uxev1`'s own description ("There is no repository transfer, no
    rename, no history rewrite and no GitHub Support ticket") and
    `public-launch-snapshot.md`'s precondition list, neither of which
    requires any such purge, because the snapshot approach never
    exposes the old history to begin with (it's a new, empty repository,
    not a rewritten one). This runbook does not carry that stale
    precondition forward. It isn't listed in any bead's acceptance
    criteria; every other source is unanimous that it doesn't apply.

## Roll-forward rule

Releases are **immutable** once published. Never delete or unpublish a
release — every installed app's update check resolves to "the latest
release," and pulling one out from under that breaks the next check for
anyone still on it. A bad release is **superseded** by a higher patch
version instead; the previous DMG stays linked from the changelog history
rather than disappearing. Rehearse this once, deliberately, on the
throwaway `dohflow/updater-smoke` repository before it's ever needed for
a real release — not as part of this runbook's own steps, but as
standing practice from `personal-cfo-867.1.2`'s own negative tests.

## T+3–7 days

Show HN, Product Hunt, and Reddit posting follow go-live and are **not**
Launch-gate items (`personal-cfo-2owr`'s own acceptance criteria says so
explicitly). See `personal-cfo-n76x.20` (the Show HN / PH day runbook —
a different document from this one) and
[launch-calendar.md](../product/launch-calendar.md#target-dates) for the
2026-10-02 floor date.

## T+7 days

`v0.1.1` ships, proving the auto-update path works in the wild for real
users (not just the smoke repo). Once verified, close
`personal-cfo-867.1` and `personal-cfo-fkt5` (both still open today —
each has open children of its own: `867.1` has `867.1.3`; `fkt5` has
`uxev1`, `fkt5.6`, `fkt5.9`, `r36ck`, and `fkt5.12` in progress).

## What this runbook does not cover

- **Bead-graph reconciliation** (whether `sg8`'s waived items still
  correctly stay off `personal-cfo-2owr`'s dependency list, hour-total
  bookkeeping) — [`docs/product/launch-calendar.md`](../product/launch-calendar.md).
- **The actual snapshot mechanics** (git commands, tree verification,
  content scans) — [`docs/operations/public-launch-snapshot.md`](public-launch-snapshot.md).
- **Show HN / Product Hunt day itself** — `personal-cfo-n76x.20`.
- **Anything after `v0.1.1`** — out of scope for a *launch* runbook.
