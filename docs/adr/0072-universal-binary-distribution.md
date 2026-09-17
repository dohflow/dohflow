# ADR 0072 — Universal binary distribution for Intel + Apple Silicon

- **Status:** Accepted (2026-09-17)
- **Tier:** Public — release/distribution engineering decision, no business
  content.
- **Bead:** `personal-cfo-wpgqr`
- **Decider:** Owner, 2026-09-17, in chat with the implementation session
- **Related:** ADR 0065 (minimum macOS version — its floor rises to 15.0 in
  November 2026, decision f below), ADR 0068 (release distribution and
  update channel — §2's `latest.json` shape is refined, not superseded, by
  decision c below), ADR 0067 (rename policy — the bundle identifier and
  data directory are unaffected by architecture), `personal-cfo-9e79c`
  (DIST-1, the spike this ADR depends on), `personal-cfo-0zlg9` (DIST-3,
  implements decision c in the release scripts), `personal-cfo-rr0lm`
  (DIST-5, implements decision e's CI job), `personal-cfo-0wfrr` (DIST-4,
  the Intel smoke drill decision d's checklist step feeds), `personal-cfo-
  867.1.4` / `personal-cfo-56s6` (release signing in CI — decision e
  explicitly defers to these, not decided here)

## Context

`personal-cfo-sg8.1.1` (Intel Mac support, M1) has always required this
decision before any build-pipeline work starts — the candidate it used to
list alongside a real universal build, "Rosetta 2 is good enough," turned
out to be factually impossible: Rosetta 2 translates x86_64 binaries to run
*on* Apple silicon. It has no mechanism to run an arm64 binary on actual
Intel hardware, so it was never a way to support Intel Macs at all, only a
way to run non-native software on the platform this project already ships
for. That candidate is struck from `sg8.1.1` already (its 2026-09-15 AC
rewrite), and this ADR records the reason formally per decision b.

`personal-cfo-9e79c` (DIST-1) closed the remaining open question: would any
arch-sensitive dependency force a workaround? Its finding, verified
2026-09-17 both by cross-compiling from this arm64 Mac and natively on a
`macos-15-intel` GitHub runner: **none**. The root workspace (including
`vault-crypto` and `db-worker`, which wrap SQLCipher/OpenSSL via
`rusqlite`'s vendored C) and the full desktop crate (including the
`tao`/`objc2-web-kit`/`window-vibrancy`/`muda` macOS-native bindings) both
build cleanly for `x86_64-apple-darwin`, zero warnings, zero errors, on
both legs. Nothing here is arm64-only.

## Decision

### a. One universal binary, one DMG

`tauri build --target universal-apple-darwin` (Tauri invokes `lipo`) ships a
single `.app`/DMG containing both architecture slices, rather than two
separate per-architecture DMGs with two separate updater manifest entries.
Universal wins on every axis that matters for this project: the download
page states "one download" instead of asking a non-technical user to pick
an architecture; support never has to answer "which one do I have"; the
updater ships one archive and one signature. The only axis two artifacts
would win on — smaller download size per architecture — is a non-issue for
a desktop finance app's DMG size. Build time roughly doubles (both slices
compile); there are no sidecar binaries to complicate the lipo step.

### b. Rosetta 2 — rejected alternative

**Rejected.** Rosetta 2 translates x86_64 binaries to run on Apple silicon;
it has no mechanism to run an arm64 binary on real Intel hardware. It is
not a way to support Intel Macs — an Intel Mac cannot use Rosetta to run an
Apple-silicon-only build at all. Not evaluated further because there is
nothing to evaluate: it does not address the problem.

### c. Updater manifest: one universal archive under both platform keys

`latest.json` serves the **same universal archive, same URL, same
signature**, under both `platforms.darwin-aarch64` and
`platforms.darwin-x86_64`. This is correct — not a placeholder or a
temporary duplication — because the pinned `tauri-plugin-updater = 2.11.0`
selects its manifest entry by the running binary's `cfg!(target_arch)`, and
a universal binary's two slices both live in the one artifact being
served; there is no second file to point to.

This refines ADR 0068 §2, which correctly anticipated "if a universal or
Intel build ever ships, its platform entry is `platforms.darwin-x86_64`
alongside it" but did not specify that a *universal* build's two keys
share one artifact rather than pointing at two independently-built ones —
decided here, not a correction of anything ADR 0068 got wrong.

**This manifest shape is a one-way door once real users are on it.**
Decided now, at ~0 installed users on the affected version, rather than
after `v0.2.0` ships and an installed base depends on however the keys
are shaped. `personal-cfo-0zlg9` (DIST-3) implements it across the five
hardcoded `darwin-aarch64`-only sites in `scripts/publish-release.sh`
(lines 271, 274, 445, 461, 477) plus `scripts/release.sh:277` — all of
which currently write or read only the `darwin-aarch64` key, per ADR
0068's original Apple-silicon-only scope for `v0.1.0`.

### d. Intel is a supported tier, not best-effort

The owner owns a second, Intel Mac. The release checklist gains an
explicit Intel smoke step (`personal-cfo-0wfrr`, DIST-4) that runs before
every release is published, the same way the existing checklist already
gates on other release-blocking checks. This is a supported platform
tier with its own verification step, not a community-maintained,
best-effort target that ships untested.

### e. D9a resolved, D9b explicitly deferred

**D9a (resolved):** a CI Intel build-and-launch smoke test on
`runs-on: macos-15-intel` is free on this public repository and costs
nothing unless it runs — `personal-cfo-9e79c` proved this concretely with a
throwaway `workflow_dispatch` job; `personal-cfo-rr0lm` (DIST-5) is the
permanent version of that job, running on every release-relevant change
rather than by hand.

**D9b (explicitly deferred, not decided here):** moving release **signing**
into CI — the Apple certificate and the minisign private key currently
live only in the build Mac's keychain and local files — is a separate
owner key-custody decision, tracked by `personal-cfo-867.1.4` and
`personal-cfo-56s6`. The `.p12` and the minisign key stay on the build Mac
for `v0.2.0`. Universal-binary support does not require moving them; it is
built and signed on the same machine that already builds and signs the
Apple-silicon-only release today, just with the `universal-apple-darwin`
target.

### f. ADR 0065's floor rises to 15.0 in November, on schedule

Unchanged from ADR 0065's own text, restated here because it is the same
machine this decision leans on: the current 14.0 floor is explicitly
dated and rises to 15.0 when Sonoma leaves its patch window (~November
2026). The second, Intel Mac is already on 15.7.9 and becomes the
Intel-side floor-testing rig; `personal-cfo-wd7ep` pairs the build Mac's
own upgrade with that floor raise, per ADR 0065's existing text — nothing
new decided about the floor itself here.

## Consequences

- **Positive.** One build artifact, one manifest, one download link — the
  simplest shape for users and for the release scripts, at the cost of a
  roughly 2x build time that only the release process (not development)
  pays.
- **Positive.** The manifest-shape one-way door is closed correctly before
  any real user is on a version that depends on it.
- **Positive.** No new key-custody surface, no change to where signing
  material lives — `v0.2.0` ships from the same trusted machine as
  `v0.1.0`.
- **Negative / accepted.** `personal-cfo-0zlg9` must touch six call sites
  across two scripts to make this real; none of that is done by this ADR,
  which only fixes the shape they must converge on.
- **Negative / accepted.** D9b (signing in CI) stays deferred — Intel
  support does not force that decision, but it also does not resolve it;
  `867.1.4` / `56s6` remain open, tracked separately.
