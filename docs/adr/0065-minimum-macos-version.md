# ADR 0065 — Minimum supported macOS version

- **Status:** Accepted (2026-09-05)
- **Decision:** **macOS 14.0 Sonoma**, declared in
  `bundle.macOS.minimumSystemVersion`
- **Bead:** `personal-cfo-867.1.5`

## The bug this fixes

`bundle.macOS.minimumSystemVersion` was **never set**, so Tauri's default
applied and the built app declared:

```
LSMinimumSystemVersion: 10.13
```

macOS 10.13 High Sierra shipped in **2017**. The app has never been built or run
on it, Tauri 2 itself requires 10.15+, and the app would almost certainly fail
immediately. The DMG was making a compatibility promise nobody had tested — a
user on an old Mac would download it, watch it fail, and reasonably conclude the
software is broken.

## Context

As of 2026-09-05, Apple's patch window sits at three major versions:

| Version | Security updates |
|---|---|
| macOS 26 Tahoe | current |
| macOS 15 Sequoia | supported |
| macOS 14 Sonoma | **supported, ending ~November 2026** |
| macOS 13 Ventura | **ended 2025-09-15** |

The build Mac runs **14.5**, so 14.x is the only version that can actually be
tested today. Testing on 15 or 26 needs `personal-cfo-wd7ep`.

## Decision and why

**14.0.** Three reasons, in order of weight:

1. **It is the oldest version we can honestly claim.** The test machine is 14.5,
   and API and WebKit surface is consistent within a major version. Claiming 13
   would repeat the 10.13 mistake with a smaller number.
2. **13 is already unpatched.** Recommending that a *financial* application hold
   a vault on an OS receiving no security fixes contradicts SECURITY.md, which
   tells users to stay current. The floor should not be below Apple's own.
3. **Raising a floor is easy; lowering one is not.** Users on the excluded
   version never installed, so nothing breaks for them. Dropping support for
   users who already installed does break things.

**Not 15.0**, though it is tempting given Sonoma's November end-of-life: we
cannot test on 15 today, and shipping an untested floor is the exact failure
being corrected here.

## Consequences

Macs that cannot run Sonoma are excluded — 2017-era hardware and earlier. For a
macOS-only, local-first application in 2026 that is a small and defensible
population, and they are also the machines least able to run the app well.

**This floor is dated and must be revisited.** Sonoma leaves the patch window
around November 2026, at which point the minimum should rise to **15.0** —
subject to actually testing there, which `wd7ep` enables. Tracked in the release
runbook rather than left to memory.

Also worth stating plainly: **the build Mac itself falls out of security support
in November 2026.** That makes `wd7ep` a security item, not a convenience for
Icon Composer.

## Where this number appears

Changing it means changing all of these together:

- `apps/desktop/src-tauri/tauri.conf.json` — the source of truth
- `dohflow-site` `/download` — the stated requirement
- `README.md` requirements section
- the release checklist (`867.1.3`)

## Verification

```sh
plutil -extract LSMinimumSystemVersion raw \
  "apps/desktop/src-tauri/target/release/bundle/macos/DohFlow.app/Contents/Info.plist"
```

Must print `14.0`. This is asserted in the release checklist because the value
silently reverts to Tauri's default if the config key is ever dropped.

## Accepted addendum (2026-10-17): the 15.0 raise waits for an updater guard

- **Status:** Accepted
- **Tier:** Public — supported-platform policy and release engineering.
- **Decider:** Project owner
- **Beads:** `personal-cfo-wd7ep` (the raise), `personal-cfo-md8h2.4` (the guard)

**What changed.** The trigger this ADR named has arrived. Sonoma's last security
update was 14.8.9 on 2026-08-06, and Apple's September 2026 security releases
patched macOS 27, Tahoe 26 and Sequoia 15 but not Sonoma. A new build machine
can now test on 15 and newer.

**Decision.** v0.3.0 keeps the **14.0** floor. The raise to **15.0** ships in
the first release *after* v0.3.0, and only once that release's predecessor
carries an updater guard. Two facts drive the order:

1. **v0.3.0 carries a data-safety fix** for the bundled SQLCipher engine
   (`personal-cfo-g3m.5`). Raising the floor in the same release would leave
   every 14.x user on v0.2.0's unfixed engine, worse than an unpatched OS
   running a fixed app.
2. **The release updater has no OS check.** It reads one static `latest.json`,
   so a 14.x install would download a 15.0-floor release that macOS then
   refuses to open. This is the "dropping users who already installed breaks
   them" case in this ADR's third reason. v0.3.0 therefore ships a guard: an
   installed app declines an update whose declared minimum macOS is newer than
   the running one, and says why (`personal-cfo-md8h2.4`).

**How v0.3.0 stays honest at 14.0.** This ADR's first reason (claim only what we
test) still holds. The release candidate is smoke-tested on a macOS 14.x Mac
with synthetic data before publication.

**Who the later raise excludes.** Sequoia supports every Mac that supports
Sonoma except the 2018–2019 Intel MacBook Air, so nearly everyone on 14.x can
update macOS for free. Public usage data is thin: browser statistics can't tell
macOS versions apart, and developer-skewed package-manager analytics put Sonoma
at roughly 3% of Macs in September 2026. The general share is probably
somewhat higher.

**Residual risk.** A v0.2.0 install that never updates to v0.3.0 has no guard,
and would still take the floor-raise release. The gap between v0.3.0 and the
raise is the mitigation. If that macOS refusal happens, the fix is to reinstall
v0.3.0 from the releases page.
