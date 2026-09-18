# Intel smoke drill (launch leg automated, Gatekeeper leg owner-run)

- Bead: `personal-cfo-0wfrr` (DIST-4). Depends on `personal-cfo-rr0lm`
  (DIST-5, the CI launch leg) and `personal-cfo-0zlg9` (DIST-3, the
  universal build the CI job builds).
- What this proves: ADR 0072 decision (d) — "Intel is a supported tier" —
  is backed by an actual run, not just asserted.
- Two independent legs. The first runs automatically in CI on every push to
  `main`. The second needs real Intel hardware and is the owner's to run.

## Leg 1 — Launch leg (CI, automated, already passing)

`.github/workflows/ci.yml`'s `intel-smoke` job, on a real `runs-on:
macos-15-intel` GitHub-hosted runner: builds the universal binary
(`--target universal-apple-darwin`), confirms `lipo -archs` reports both
`arm64` and `x86_64`, then launches the binary headlessly
(`PCFO_SMOKE_TEST_EXIT=1`) and confirms it prints a confirmation line and
exits `0` on its own. No display, no vault, no secrets — see
`docs/development/desktop-app.md`'s "CI: the Intel build + launch smoke"
section for exactly what this does and does not prove.

**Pass criteria:** the job is green; its log contains `lipo -archs: ...
x86_64 ... arm64 ...` and `DohFlow smoke test: setup() completed
successfully, exiting 0.` with exit code `0`.

**First green run:**
<https://github.com/dohflow/dohflow/actions/runs/35291727308> (2026-09-18).
Confirmed by both the implementing session and an independent review pass
that re-pulled the log directly rather than trusting a summary.

This leg runs on every push to `main` and via manual `workflow_dispatch` —
it does not need to be re-run specially for a release; the release
checklist's own Intel-smoke item (`docs/operations/release-checklist.md`
step 5) just confirms the job passed on the release commit.

## Leg 2 — Gatekeeper leg (real Intel hardware, owner-run)

**Whether this leg runs at all depends on `personal-cfo-r7nma`** — a
one-command check of the owner's second Mac's CPU
(`sysctl -n machdep.cpu.brand_string`), not yet run as of this writing. Two
outcomes:

- **If the second Mac is Intel:** run the steps below on it.
- **If the second Mac is Apple silicon:** record "unavailable — second Mac
  is Apple silicon" here and on `personal-cfo-sg8.1.1`, and state the same
  on the `/download` page copy (no live Intel-hardware verification exists
  for this release; the CI launch leg above is the only automated proof).

The launch leg above is not a substitute for this one if Intel hardware
*is* available — it proves the binary runs, not that Gatekeeper accepts a
real signed, notarized artifact, or that the actual first-launch UX (the
"unidentified developer" wall, or its absence for a properly signed build)
matches what `README.md` documents.

### Steps (on the Intel Mac, once one is confirmed available)

1. Download the signed + notarized DMG **through Safari** from the
   release's download page (so it carries the quarantine attribute) —
   `xattr -p com.apple.quarantine` on the downloaded file confirms it.
2. Mount the DMG, drag `DohFlow.app` to `/Applications`, launch it.
   **Record the exact dialog text shown** (or its absence) — this is what
   `README.md`'s "What you'll see on first launch" section quotes, and it
   must match reality on both architectures, not just Apple silicon.
3. `spctl -a -vv -t install /Applications/DohFlow.app` — record the exact
   output (expected: `accepted, source=Notarized Developer ID`).
4. `sysctl -n hw.machine` and `sw_vers -productVersion` — record both, for
   the exact machine/OS this leg ran against.
5. Create a vault, unlock it. **Time the unlock** (password entry to
   dashboard) — this is the Argon2id KDF cost on real Intel hardware, not
   a proxy or an estimate. Record the wall-clock time.
6. Import a CSV, relaunch the app, unlock again, back up, restore. Confirm
   the full cycle works exactly as it does on Apple silicon.

### After the drill

Record in this bead's notes (`personal-cfo-0wfrr`), dated:

- The machine (from `personal-cfo-r7nma`'s CPU check), macOS version.
- The exact Gatekeeper dialog strings from step 2, and the `spctl` output
  from step 3.
- The vault create/unlock/import/backup/restore result (pass, or the exact
  failure).
- The measured unlock time from step 5.

Then:

- Cross-post the measured unlock time (or "no Intel hardware available") to
  `personal-cfo-0sqk` (Argon2id calibration) — its own acceptance criteria
  gain an "unlock time on the slowest supported Intel Mac" data point from
  this drill.
- If Leg 2 is unavailable (Apple-silicon-only second Mac), update
  `personal-cfo-sg8.1.1`'s notes and the `/download` page copy to say so
  plainly, rather than let the gap go unstated.

## Leg 3 — Updater round trip (v0.2.1, not this release)

`v0.1.0` never ran on Intel at all, so the **first** Intel updater round
trip can only happen once an Intel-capable `v0.2.0` is installed and a
`v0.2.1` exists to update to. This is **not** a `v0.2.0` release gate — it
is listed as a `v0.2.1` release-checklist item
(`docs/operations/release-checklist.md`) so it is not forgotten, without
blocking `v0.2.0` on a release that doesn't exist yet.
