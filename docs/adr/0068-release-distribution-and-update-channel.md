# ADR 0068 — Release distribution and update channel

- **Status:** Accepted (2026-09-08). Content decided across owner decisions
  2026-09-05 through 2026-09-08; this ADR records it as a foundation-first
  step for the updater (`personal-cfo-867.1.2`), which asks for an ADR that
  did not yet exist.
- **Decision:** GitHub Releases under `dohflow/dohflow` is the distribution
  channel; the Tauri v2 updater plugin checks one signed `latest.json`
  endpoint on that repo; the owner's minisign keypair signs every release;
  a bad release is superseded, never deleted; the app never installs an
  update silently.
- **Bead:** `personal-cfo-rqxmz` (this ADR); implemented by
  `personal-cfo-867.1.2`; cited by `personal-cfo-sbkw` (privacy fact
  sheet).
- **Related:** ADR 0010 (Tauri capability isolation — `updater:default` /
  `process:allow-restart` follow its deny-by-default model), ADR 0061
  (website stack — states the app's zero-telemetry stance this ADR's
  privacy point matches), ADR 0064 (task tracking leaves the repo — same
  "small decision ADR, house style"), ADR 0065 (minimum macOS version —
  same style), ADR 0067 (DohFlow rename policy — the release artifact
  names this ADR's `latest.json` example uses).

## Context

`867.1.2` — wiring the actual updater plugin, signing, and the public
smoke-repo round trip — needs a settled answer to "how does a user's
DohFlow get from version N to version N+1" before any code lands. That
answer was never written down; it existed only as scattered owner
decisions across bead notes between 2026-09-05 and 2026-09-08. This ADR
consolidates those decisions in one place, per the foundation-first
discipline (`AGENTS.md` §1A): the decision is recorded before the
implementation bead starts, not discovered mid-implementation.

Nothing here is a new choice. Where this ADR states a number, a URL, or a
key, it is quoting an owner decision already on record — most of it in
`867.1.2`'s own bead notes (the minisign public key, pasted 2026-09-07) and
design brief (owner decisions C and D, 2026-09-08).

## Decision

### 1. Distribution channel

A signed and notarized DMG on **GitHub Releases**, under `dohflow/dohflow`
(the repository this ADR assumes exists — created by `personal-cfo-uxev1`,
"fork day"; until then, development and testing target the throwaway
public smoke repo in point 8 below). The public website's `/download` page
reads the latest release at **site build time** and is rebuilt by the
publish script (`867.1.3`) — the site never queries GitHub live.

### 2. Update channel

The **Tauri v2 updater plugin** (`tauri-plugin-updater`), checking a
single endpoint:

```
https://github.com/dohflow/dohflow/releases/latest/download/latest.json
```

The manifest lists `platforms.darwin-aarch64` with `url` and `signature`
keys. If a universal or Intel build ever ships, its platform entry is
`platforms.darwin-x86_64` alongside it; **for v0.1.0 the app is Apple
silicon only** (ADR 0065's tested floor is a Sonoma-class Mac, and the
build Mac itself is Apple silicon), so `867.1.2`'s `latest.json` carries
only the `darwin-aarch64` entry and its PR says so explicitly rather than
leaving the omission to be discovered later.

### 3. Signing

The owner's **minisign** keypair, generated 2026-09-07 at
`~/.tauri/dohflow.key`. The public key is embedded in `tauri.conf.json`'s
`plugins.updater.pubkey` — a committed, non-secret artifact. The private
key and its password live **only** in `~/.tauri/dohflow.key`,
`~/.config/personal-cfo/release.env`, and the password manager — never in
a bead, a commit, or an agent session (no session requests or receives
them; `867.1.2`'s notes record the public half only). A tampered archive
or a signature that does not match its artifact is refused by the updater
plugin; that refusal is tested twice by `867.1.2` — a mismatched
`latest.json` signature and a tampered `.tar.gz` — with the exact refusal
text logged in the bead, closing `personal-cfo-miei`. Restoring the key
from the password manager onto a second Mac or VM and signing a test
artifact is drilled quarterly (`personal-cfo-7ie.7`), because losing the
private key strands every existing install with no way to sign a
superseding release.

### 4. Behavior

The app checks for an update on **launch/unlock** and **on demand** from
Settings (`SoftwareUpdateCard` / `useSoftwareUpdate`). On finding one, it
shows the version and release notes, and **downloads and installs only on
an explicit click** — never automatically, never silently — then restarts
via `tauri-plugin-process`'s `process:allow-restart`. The from-source
dev-updater path (`update.rs`'s existing behavior) remains active **only**
when `PCFO_BUILD_CHANNEL == dev`; a release build never takes that path.

### 5. Privacy

The update check is a plain `GET` for `latest.json` — no user identifier,
no device identifier, no analytics payload, nothing beyond what an
unauthenticated HTTP request to a public GitHub Releases asset URL
inherently carries (the requesting IP, visible to GitHub as it would be to
any web request). This matches the app's zero-telemetry stance (ADR 0061)
and is one of the claims `personal-cfo-sbkw`'s public privacy fact sheet
cites verbatim.

### 6. Roll-forward rule

Releases are **immutable** once GitHub's immutable-releases setting is
enabled on the repo (`personal-cfo-fkt5.9`, fork-day hardening). A bad
release is never deleted or unpublished — every installed app's
`latest.json` points at `/releases/latest`, and unpublishing a release an
app has already resolved breaks that install's next check. Instead, a bad
release is **superseded** by a higher patch version; the previous DMG
stays linked from `/download`'s release history rather than removed.

### 7. Rejected alternatives

- **Sparkle.** A second signing system alongside minisign, for no benefit
  over the updater plugin Tauri already ships. Rejected.
- **A self-hosted update server.** Ongoing hosting cost and a new secret
  (the server's own credentials) for a problem GitHub Releases already
  solves for free. Rejected.
- **Silent background installs.** Wrong trust story for a finance
  application handling a user's encrypted vault — an update that changes
  behavior without the user's awareness is exactly the kind of surprise
  SECURITY.md and the app's local-first framing promise the user will
  never get. Rejected; point 4 above is the direct answer.
- **Delta updates.** Not offered by `tauri-plugin-updater` for macOS
  today. Revisit if the plugin adds it and DMG size becomes a real
  friction point.

### 8. Consequences

- **Positive.** GitHub Releases needs no new infrastructure, no new
  secret beyond the signing key the app already needs, and gives the
  smoke test (below) a realistic path — real redirects, real asset URLs,
  real `latest.json` resolution — rather than a stand-in.
- **Negative, accepted.** The public key is committed and therefore
  permanent: rotating it means every existing install stops trusting new
  releases signed with the old key until it manually reinstalls. This is
  the normal cost of any static-key update scheme and is why the private
  key's custody and drill (`7ie.7`) matter more than almost any other
  secret in the project.
- **Negative, accepted.** Losing the private key strands every install —
  no future release can be verified by any existing copy of the app. The
  quarterly restore drill (`7ie.7`) exists specifically to keep this from
  being discovered the day it happens.
- **Test infrastructure.** The real repository is private before the
  fork-day transfer (`uxev1`), and even after transfer,
  `/releases/latest` excludes draft and prerelease releases — so a draft
  release under the real repo cannot prove the update path works. The
  proof before the flip uses a **throwaway public repository**,
  `dohflow/updater-smoke` (owner decision C, 2026-09-08), holding two
  published non-draft releases so the real GitHub Releases resolution
  path is genuinely exercised. It is deleted once `867.1.2` closes and the
  owner has recorded approval for the deletion in that bead's notes — an
  explicitly owner-approved destructive action, not a default one
  (`AGENTS.md` §1).

## Out of scope

- The updater's implementation — plugin wiring, capability grants,
  `release.sh`, the smoke-repo round trip, the two negative tests. All
  `personal-cfo-867.1.2`.
- The CI release workflow. `personal-cfo-867.1.4`.
- Windows and Linux update channels. Not planned; this project ships
  macOS only today (ADR 0065).

## Revisit if

- A second platform (Windows/Linux) ships and needs its own channel entry
  in `latest.json` and its own signing story.
- `tauri-plugin-updater` adds delta updates for macOS.
- The minisign key is ever rotated — this ADR's point 3 and the
  `pubkey` value in `tauri.conf.json` both need a dated addendum recording
  the new key and the reason.

## Verification

Docs-only ADR: no code changes accompany it.

- `docs/adr/0068-release-distribution-and-update-channel.md` merged with
  **Status: Accepted**.
- Every cross-reference above resolves: `personal-cfo-rqxmz`,
  `personal-cfo-867.1.2`, `personal-cfo-867.1.4`, `personal-cfo-7ie.7`,
  `personal-cfo-sbkw`, `personal-cfo-miei`, `personal-cfo-fkt5.9`,
  `personal-cfo-uxev1`, ADR 0010, ADR 0061, ADR 0064, ADR 0065, ADR 0067.
- `0068` is the next free ADR number (`0067` is the highest existing file
  at the time this ADR was written).
- `867.1.2`'s design references this ADR directly rather than
  re-deriving the same decisions.
- US spelling throughout.
