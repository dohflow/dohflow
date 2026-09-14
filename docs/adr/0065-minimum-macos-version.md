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
