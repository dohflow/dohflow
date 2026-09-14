# Updater smoke test runbook (owner-run)

- Bead: `personal-cfo-867.1.2` (closes `personal-cfo-miei`)
- Decision this proves: ADR 0068 point 8 — `dohflow/updater-smoke`, a throwaway
  **public** repository, is the pre-flip proof that the real GitHub Releases
  path (redirects, asset URLs, `latest.json` resolution) actually works,
  since the real `dohflow/dohflow` repo is private before fork day
  (`personal-cfo-uxev1`) and `/releases/latest` excludes drafts/prereleases
  even after — a draft release under the real repo cannot prove this.
- **Owner-run.** Per `personal-cfo-867.1.2`'s own session split
  (`owner-terminal → claude-code (plan mode) → owner`), the code/config half
  of this bead was implemented and gated by an agent session; this runbook
  and the round trip itself are the owner's to execute, because it needs a
  real installed app, the private signing key on this Mac, and a judgment
  call about what "the update visibly worked" means for a finance app's
  trust story.

## Before you start

1. **Add the updater signing vars to `release.env`** — as of this writing
   they are not there yet (only the `APPLE_*` lines from A5's table exist).
   Append to `~/.config/personal-cfo/release.env`:
   ```sh
   export TAURI_SIGNING_PRIVATE_KEY=$HOME/.tauri/dohflow.key
   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD='<the password you set at keygen>'
   ```
   (`docs/operations/release-signing.md` A6.) `chmod 600` was already set on
   the file; re-confirm it (`ls -l ~/.config/personal-cfo/release.env`).
2. `source ~/.config/personal-cfo/release.env`
3. `./scripts/release.sh --check` — should now pass (identity + notary creds
   + the two `TAURI_SIGNING_*` vars all present).

## 1. Create the smoke repo

```sh
gh repo create dohflow/updater-smoke --public --description "Throwaway repo: Tauri updater round-trip proof for personal-cfo-867.1.2. Safe to delete." -y
```

Add a one-line README explaining what it is and that it will be deleted
(so it doesn't look abandoned/mysterious to anyone who stumbles on it while
it's up).

## 2. Build and publish 0.9.0

The smoke test uses fictional versions `0.9.0` → `0.9.1`, not the real
`0.1.0` — this proves the MECHANISM, not a real release.

`--smoke-version` and `--smoke-endpoint` inject the throwaway version and
endpoint through the same `tauri build --config` deep-merge the signing
identity already uses, so **`tauri.conf.json` is never edited**. That matters
for more than tidiness: `release.sh` refuses to build from a dirty worktree
(`RELEASE_ALLOW_DIRTY=1` overrides it), so an edited config would fail the
build outright — and a `0.9.x` version could never be committed by accident.
The two flags must be given together, and the version must be `0.9.x`; the
script refuses otherwise.

```sh
./scripts/release.sh --notes "Smoke test baseline" \
  --smoke-version 0.9.0 \
  --smoke-endpoint "https://github.com/dohflow/updater-smoke/releases/latest/download/latest.json"
```

The Rust compile log will say `Compiling personal-cfo-desktop v0.1.0` — that
is the *crate* version and is expected. The smoke version applies to the app
bundle (macOS `CFBundleShortVersionString`), which is what the updater
compares against `latest.json`.

Then, in `latest.json` (written next to the DMG), replace the placeholder
`platforms.darwin-aarch64.url` with the real asset URL you're about to
create, and:

```sh
gh release create 0.9.0 --repo dohflow/updater-smoke --title "0.9.0" \
  --notes "Smoke test baseline" \
  "<path to>/DohFlow.app.tar.gz" \
  "<path to>/latest.json"
```

(`gh release create` with an explicit tag publishes a real, non-draft,
non-prerelease release by default — confirm with `gh release view 0.9.0
--repo dohflow/updater-smoke` that `isDraft`/`isPrerelease` are both
`false`, since a draft/prerelease is exactly what `/releases/latest`
excludes and this whole test exists to avoid.)

Install the resulting `.app` (drag to `/Applications`, or `open` the DMG).
Launch it once to confirm it opens normally.

## 3. Build and publish 0.9.1, confirm the round trip

Repeat the build with `--smoke-version 0.9.1` (same `--smoke-endpoint`) and
a `--notes` describing a fake change, then publish a second real release the
same way. Again: nothing in the worktree is edited.

With the `0.9.0` install still running (or freshly relaunched):

1. Open Settings → Software update, or wait for the launch-time check.
2. Confirm it reports **v0.9.1 available** with the notes you wrote.
3. Click **Update & relaunch**. Confirm:
   - a downloading state appears (progress, if the content length was
     reported),
   - then installing,
   - then the app **relaunches automatically** and now reports **0.9.1**
     as current.
4. **Log the result in the `personal-cfo-867.1.2` bead note**, with the
   date: "0.9.0 → 0.9.1 round trip via `dohflow/updater-smoke` succeeded,
   signature verified, relaunched to 0.9.1 automatically."

## 4. Negative test 1 — mismatched signature (closes half of `miei`)

With `0.9.1` still the latest real release, edit ONLY the published
`latest.json` asset (re-upload it) so `platforms.darwin-aarch64.signature`
is a plausible-looking but WRONG value (e.g. flip a few base64 characters
near the middle — a value that still base64-decodes, so the failure you
provoke is a real signature mismatch, not a parse error).

```sh
gh release upload 0.9.1 --repo dohflow/updater-smoke --clobber \
  "<path to a hand-edited latest.json with a bad signature>"
```

From an install still on `0.9.0` (or a fresh one), trigger a check + Update.
**Expected: the app refuses the update** — an error surfaces in the
Software update card (not a silent no-op, not a crash), and the app is
**not** replaced. Copy the exact refusal text shown in the UI, and check
Console.app / the app's own log output for the underlying error message
from the updater plugin's signature verification.

**Log the exact refusal text in the `personal-cfo-867.1.2` bead note**,
dated. Restore the correct `latest.json` afterward (re-upload the real one)
before the next test.

## 5. Negative test 2 — tampered artifact (closes the other half of `miei`)

Restore `latest.json` to its correct, valid content (signature matches the
REAL `.app.tar.gz`), then replace the published `.app.tar.gz` asset itself
with a tampered copy (flip a few bytes with `dd` or similar — enough to
change the file's hash without corrupting the tar/gzip structure so
outright, e.g. touch a byte inside the payload rather than the gzip
header):

```sh
cp DohFlow.app.tar.gz DohFlow.app.tar.gz.tampered
# flip one byte somewhere past the gzip header, e.g. offset 1000
python3 -c "
data = bytearray(open('DohFlow.app.tar.gz.tampered', 'rb').read())
data[1000] ^= 0xFF
open('DohFlow.app.tar.gz.tampered', 'wb').write(data)
"
gh release upload 0.9.1 --repo dohflow/updater-smoke --clobber \
  DohFlow.app.tar.gz.tampered#DohFlow.app.tar.gz
```

`latest.json`'s `signature` field still names the ORIGINAL, correct
artifact's signature — so this specifically tests "does the plugin verify
the downloaded BYTES, not just trust whatever `latest.json` claims."

Trigger a check + Update from an install on `0.9.0`. **Expected: refused**,
same as negative test 1 — no silent install of tampered bytes, no crash,
a clear error surfaced. **Log the exact refusal text in the bead note**,
dated.

## 6. Close out

- Update the `personal-cfo-867.1.2` bead's acceptance criteria: both
  negative tests logged with dates and exact refusal text; the 0.9.0 →
  0.9.1 round trip logged; close `personal-cfo-miei` referencing this
  runbook and the bead note.
- Nothing to revert in the worktree: `--smoke-version` never touched
  `tauri.conf.json`. Confirm with `git status --porcelain` (should be empty
  apart from build artifacts, which are gitignored).
- **Delete the smoke repo** — this is a destructive, outward-facing action
  (`AGENTS.md` §1): record explicit approval in the bead note first
  (who approved, when), then:
  ```sh
  gh repo delete dohflow/updater-smoke --yes
  ```
- Note in the bead whether `miei`'s AC is fully met or whether a waiver is
  needed (e.g. "the CI-automated version of this negative test is deferred
  to `867.1.4`; the manual evidence above stands in for it").
