# Release checklist

- Bead: `personal-cfo-867.1.3` (this doc + `scripts/publish-release.sh`)
- Audience: whoever cuts a DohFlow release — owner-terminal for the
  credentialed/physical steps, an agent session for the scripted steps.
  Every step says which.
- Scope: cutting and publishing **one** signed macOS release
  (`vX.Y.Z`) from a clean, already-decided commit. It does not cover
  *deciding* what ships (that's the CHANGELOG.md `## [Unreleased]` →
  dated-section edit, done beforehand) or first-time credential setup
  (`docs/operations/release-signing.md`, Part A — one-time, already
  done).
- For `v0.1.0` specifically, read
  [`public-launch-runbook.md`](public-launch-runbook.md) first — it
  sequences this checklist alongside the repo-visibility flip and the
  snapshot procedure. This doc is the release mechanics on their own;
  that doc is where they fit in go-live day as a whole.

Two scripts, two different jobs — don't confuse them:

| Script | Job | Docs |
|---|---|---|
| `scripts/release.sh` | Builds, signs, notarizes, staples. Produces the `.dmg`, `.app.tar.gz`, `.sig`, `latest.json` (with a placeholder asset URL). | `release-signing.md` |
| `scripts/publish-release.sh` | Everything *after* the build: tag, checksum, assemble, draft the GitHub Release, and (later) publish it + rebuild the site. | this doc |

## 0. Preconditions

- [ ] Tag signing is configured (`release-signing.md` A-series) — **done once**,
  2026-09-07: `gpg.format ssh`, `user.signingkey`, `tag.gpgsign true`, the SSH
  key registered on GitHub as a signing key, verified with a throwaway tag
  showing "Verified". Confirm it's still true with:
  ```sh
  git config --get tag.gpgsign   # expect: true
  ```
- [ ] `apps/desktop/src-tauri/tauri.conf.json`'s `version`,
  `apps/desktop/src-tauri/Cargo.toml`'s `version`, and
  `apps/desktop/package.json`'s `version` all read the version being
  released. `./scripts/publish-release.sh preflight` (step 2 below) checks
  this for you — but if it fails here, decide the bump as its own commit
  before continuing; don't fold a version bump into the same commit as
  unrelated release-script changes.
- [ ] `CHANGELOG.md` has a dated `## [X.Y.Z] - YYYY-MM-DD` section (not the
  placeholder date), and that date is **today** — `preflight` (step 2 below)
  asserts both, so a go-live date that slipped without the CHANGELOG catching
  up fails loudly here rather than shipping a release whose own changelog
  misstates when it happened. Update the heading the morning you actually
  tag, not before. Proofread the section's content here too — `draft` (step
  4) reads it verbatim as the GitHub Release notes.
- [ ] **Icon note:** if the icon-refresh work isn't merged yet, `v0.1.0` ships
  with the legacy `.icns`. Say so explicitly in the release notes if that's
  still the case when this release is cut — don't let it be a silent
  surprise on `/download`.
- [ ] `git status --short` is clean on the commit being released (working
  tree, not just staged) — `preflight` refuses a dirty tree unless
  `RELEASE_ALLOW_DIRTY=1`.
- [ ] For `v0.1.0` only: this build happens from the **public snapshot
  clone**, not the private repo's checkout — see
  [`public-launch-snapshot.md`](public-launch-snapshot.md) step 6. Every
  command below still applies; only the working directory differs.

## 1. Build (owner-terminal — needs the signing credentials on this Mac)

```sh
source ~/.config/personal-cfo/release.env
./scripts/release.sh --notes "one-line summary of what's in this release"
```

`--notes` here only feeds `latest.json`'s own `notes` field (what the
in-app updater dialog shows). It doesn't need to be the full changelog —
`publish-release.sh draft` (step 4) pulls the complete, real release notes
from `CHANGELOG.md`'s `## [X.Y.Z]` section directly for the actual GitHub
Release.

This signs, notarizes, staples, and verifies (`codesign`, `stapler`,
`spctl`) — see `release-signing.md` for what each step means and how to
read a failure. It ends with the `.dmg`, `.app.tar.gz`, `.sig`, and a
`latest.json` with a **placeholder** asset URL, all under
`apps/desktop/src-tauri/target/release/bundle/{macos,dmg}/`.

## 2. Preflight (either session)

```sh
./scripts/publish-release.sh preflight
```

Checks version consistency (item 0 above), that the CHANGELOG entry is
dated **today** (not a placeholder, and not a stale date left over from an
earlier planned go-live day), a clean tree, and that the release's tag
doesn't already exist locally or on `origin`. Fix anything it flags before
continuing — it exits non-zero and names exactly what's wrong.

## 3. Tag (owner-terminal — same signing identity as step 0)

```sh
./scripts/publish-release.sh tag
```

Runs preflight again, then `git tag -s vX.Y.Z` and `git push origin vX.Y.Z`.
Confirm the tag shows **"Verified"** at
`https://github.com/dohflow/dohflow/releases/tag/vX.Y.Z` (or
`/commits/vX.Y.Z` before the release exists) — this is the one place a
signing regression would show up silently otherwise.

## 4. Package + draft (either session)

```sh
./scripts/publish-release.sh package
./scripts/publish-release.sh draft
```

`package` computes `SHA256SUMS.txt` over the four uploaded assets,
sanity-checks the updater `.sig` (structurally always — a 74-byte
Ed25519 minisign signature whose key id matches the pinned public key in
`tauri.conf.json`; cryptographically too, if `minisign` is installed), and
patches `latest.json`'s placeholder URL to the real, deterministic GitHub
release-asset URL (`.../releases/download/vX.Y.Z/DohFlow.app.tar.gz` —
this is knowable before upload, since GitHub's asset URLs are
tag-and-filename-deterministic; no chicken-and-egg wait for an API
response). It refuses loudly rather than guessing if any of that doesn't
line up — a build that "packaged" but didn't actually sign correctly is
worse than one that failed outright.

`draft` runs `gh release create vX.Y.Z --repo dohflow/dohflow --verify-tag
--draft` with the DMG, `.app.tar.gz`, `.sig`, `SHA256SUMS.txt`, and the
patched `latest.json`, titled `DohFlow X.Y.Z`, with the CHANGELOG's own
`## [X.Y.Z]` section as the release notes (pass `--notes-file FILE` to
override). **This is still a draft — nothing public yet.**

## 5. Owner smoke test (owner-terminal + a second Mac — B6)

Do this against the **draft** URL, before step 6 ever runs. Full detail:
`release-signing.md` §B6. On the second Mac (today running macOS 15.7.9 —
confirmed above ADR 0065's 14.0 floor):

- [ ] Download the DMG **through Safari** from the draft release URL (so it
  carries the quarantine attribute) — `xattr -p com.apple.quarantine` on the
  downloaded file confirms it.
- [ ] Mount, drag to `/Applications`, launch. Expect: **"Apple checked it for
  malicious software and none was detected"**, no "Open Anyway" prompt. If
  the wording differs from what `README.md`'s "What you'll see on first
  launch" section says, fix the README in this same PR.
- [ ] `spctl -a -vv -t install /Applications/DohFlow.app` →
  `accepted, source=Notarized Developer ID`.
- [ ] `defaults read /Applications/DohFlow.app/Contents/Info.plist
  LSMinimumSystemVersion` equals the value decided in
  `personal-cfo-867.1.5`, and the app actually launches on this exact macOS
  version.
- [ ] Full cycle: create/unlock a vault, import a CSV, relaunch, unlock
  again, backup, restore.
- [ ] Paste the `spctl` output, the `LSMinimumSystemVersion` value, and
  screenshots into this bead's notes for **macOS 14, macOS 15 (this run),
  and the stated minimum** — and separately for **macOS 26** when that
  environment is available (tracked by `personal-cfo-wd7ep` if deferred).
- [ ] Publish a small follow-up release afterward and confirm Settings →
  Software update finds it, installs, and relaunches — proving the update
  path against a *real*, non-smoke release once one exists.

If anything here fails, **do not proceed to step 6.** Fix it, rebuild
(step 1), and re-run from step 2 (a fresh version/tag if the fix touches
anything already tagged — a tag is never reused).

## 6. Publish (owner-terminal, only after step 5 passes)

```sh
./scripts/publish-release.sh publish
```

In order: `gh release edit vX.Y.Z --draft=false` (the release goes public),
then a `POST` to `$DOHFLOW_SITE_DEPLOY_HOOK_URL` (the dohflow-site Workers
Builds deploy hook, `personal-cfo-z5ag4` — set this from
`~/.config/personal-cfo/release.env`), then the same two checks as
`./scripts/publish-release.sh verify`:

- `curl -sI https://github.com/dohflow/dohflow/releases/latest/download/latest.json`
  → `200`.
- `curl -s https://dohflow.app/download` mentions `vX.Y.Z`.

Run `verify` again on its own a minute later if the site rebuild is still
in flight when `publish` finishes — `dohflow-site` → Deployments shows the
build's progress.

## 7. After publishing

- [ ] Review `/help/known-limitations` on dohflow-site against this
  release's CHANGELOG entry: remove anything that just shipped, add
  anything that was deliberately cut. (Breadcrumb from `personal-cfo-n67eh`
  — this is a separate repository, so it's a manual step, not something
  `publish-release.sh` can touch.)
- [ ] Record the published release URL and the smoke-test evidence
  (step 5) in the release's bead.

## Roll-forward rule (never delete or unpublish a release)

Releases are **immutable** once published. Every installed app's update
check resolves to "the latest release" — pulling one out from under that
breaks the next check for anyone still on it. A bad release is
**superseded** by a higher patch version, never deleted or unpublished;
the previous DMG stays linked from `/changelog`'s history rather than
disappearing. Full rationale and the rehearsal that proved this against a
real GitHub repo: [`public-launch-runbook.md`](public-launch-runbook.md#roll-forward-rule).

If a release turns out to be bad: fix forward with a new patch version
through this same checklist. Do not run `gh release delete`, do not
unpublish, and do not force-push the tag.

## See also

- [`release-signing.md`](release-signing.md) — what signing/notarization
  actually do, credential setup, and `scripts/release.sh`'s own flags.
- [`updater-smoke-test.md`](updater-smoke-test.md) — the pre-flip proof
  that the update mechanism works, run once against a throwaway public
  repo (`personal-cfo-867.1.2`, already closed).
- [`public-launch-snapshot.md`](public-launch-snapshot.md) — how the
  `v0.1.0` release commit itself gets created, for the one release that
  isn't just "the current tip of `main`".
- [`public-launch-runbook.md`](public-launch-runbook.md) — where this
  checklist fits into go-live day as a whole.
