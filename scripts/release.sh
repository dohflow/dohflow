#!/usr/bin/env bash
#
# Build a SIGNED + NOTARIZED, distributable "DohFlow" release (.app + .dmg), plus
# the signed updater artifact (.app.tar.gz + .sig) and latest.json, for macOS
# Developer ID distribution + the Tauri v2 updater (personal-cfo-867.1, -867.1.2,
# ADR 0068).
#
# Unlike scripts/build-app.sh (unsigned dogfooding builds), this signs with your
# Developer ID certificate, notarizes with Apple, staples the ticket, and verifies
# Gatekeeper accepts the result — so a downloaded DMG opens with no "unidentified
# developer" wall. It also signs the updater artifact with your minisign key
# (tauri.conf.json's plugins.updater.pubkey verifies it at install time) and writes
# the latest.json manifest the updater checks against.
#
# Secrets are read from the ENVIRONMENT, never the repo. Put them in
# ~/.config/personal-cfo/release.env (chmod 600) and source it first:
#
#   source ~/.config/personal-cfo/release.env
#   ./scripts/release.sh --check        # preflight only: verify creds, don't build
#   ./scripts/release.sh                 # full signed + notarized build
#
# Env knobs:
#   RELEASE_SKIP_NOTARIZE=1  sign only, skip the Apple notarization round-trip
#                            (fast iteration; the artifact will NOT pass Gatekeeper).
#   RELEASE_ALLOW_DIRTY=1    build from a dirty worktree (default: refuse — a release
#                            must be reproducible from a committed state).
#   RELEASE_NOTES            latest.json's "notes" field. Required for a real build
#                            (no default guess at what changed) — set it or pass
#                            --notes "…".
#   APPLE_PROVISIONING_PROFILE  path to the macOS Developer ID provisioning profile (personal-cfo-md8h2.17).
#                            REQUIRED for a real release: it authorizes the keychain entitlement the
#                            data-protection keychain needs (Touch ID, Sync device keys). Never committed;
#                            kept outside the repo on the build Mac (docs/operations/release-signing.md A7).
#                            The build REFUSES when it is missing, belongs to another team or app, or
#                            expires within 90 days (scripts/check-provisioning-profile.py).
#   RELEASE_SKIP_PROFILE=1   build WITHOUT the profile and entitlements (no keychain, password-only app).
#                            Allowed ONLY together with a smoke build (--smoke-version); a real release
#                            cannot skip it.
#   RELEASE_SMOKE_ENDPOINT   overrides plugins.updater.endpoints for THIS build only,
#                            via the same --config deep-merge already used for the
#                            signing identity — never set for a real release. This is
#                            the pre-flip smoke-repo round trip (ADR 0068 point 8,
#                            personal-cfo-867.1.2): point it at
#                            https://github.com/dohflow/updater-smoke/releases/latest/download/latest.json
#                            (or a local http.server URL) to prove the update path
#                            against a real public repo before dohflow/dohflow exists.
#                            Same as --smoke-endpoint <url>. Must be paired with a
#                            --smoke-version.
#   RELEASE_SMOKE_VERSION    overrides the app version for THIS build only (same
#                            --config deep-merge — the committed tauri.conf.json is
#                            never edited, so the worktree stays clean and a throwaway
#                            0.9.x can't be committed by accident). Must be 0.9.x and
#                            paired with a --smoke-endpoint. Same as
#                            --smoke-version <semver>.
#
# The signing identity is injected at build time via `tauri build --config`, so it
# never lives in the committed tauri.conf.json (contributors build unsigned).
set -euo pipefail

# Make the Rust toolchain + pnpm reachable even from a minimal shell (mirrors build-app.sh).
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

check_only=0
notes="${RELEASE_NOTES:-}"
smoke_endpoint="${RELEASE_SMOKE_ENDPOINT:-}"
smoke_version="${RELEASE_SMOKE_VERSION:-}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) check_only=1; shift ;;
    --notes) notes="$2"; shift 2 ;;
    --smoke-endpoint) smoke_endpoint="$2"; shift 2 ;;
    --smoke-version) smoke_version="$2"; shift 2 ;;
    *) echo "error: unknown argument: $1" >&2; exit 1 ;;
  esac
done

skip_notarize="${RELEASE_SKIP_NOTARIZE:-0}"
skip_profile=0
app_bundle_id=""
team_id=""

fail() { echo "error: $*" >&2; exit 1; }

# ── Preflight: everything the build needs, checked up front with clear messages ──
preflight() {
  command -v cargo >/dev/null 2>&1 || fail "'cargo' not on PATH — install Rust (https://rustup.rs)."
  command -v xcrun >/dev/null 2>&1 || fail "'xcrun' not found — install the Xcode Command Line Tools: xcode-select --install"

  # Universal binary (ADR 0072): the x86_64 slice needs its target installed.
  # Idempotent — a no-op if it already is, so this is safe to always run.
  rustup target add x86_64-apple-darwin >/dev/null


  : "${APPLE_SIGNING_IDENTITY:?set it (source ~/.config/personal-cfo/release.env) — see docs/operations/release-signing.md}"

  # The identity must be a VALID codesigning identity in the keychain (chain + private key).
  # Capture first, then match: `cmd | grep -q` under pipefail can report a false failure when
  # grep -q exits at the first match and the producer takes SIGPIPE (personal-cfo-md8h2.15).
  identities="$(security find-identity -v -p codesigning || true)"
  if ! grep -qF -- "$APPLE_SIGNING_IDENTITY" <<<"$identities"; then
    echo "Valid codesigning identities on this machine:" >&2
    security find-identity -v -p codesigning >&2 || true
    fail "APPLE_SIGNING_IDENTITY ('$APPLE_SIGNING_IDENTITY') is not a valid identity. See release-signing.md A3."
  fi

  # ── Provisioning profile + keychain entitlement (personal-cfo-md8h2.17; ADR 0002 addendum 2026-10-24) ──
  # The team id comes from the signing identity's trailing "(TEAMID)"; the profile must belong to that
  # team and to this app, carry the signing certificate, and stay valid for 90 more days.
  app_bundle_id="$(python3 -c "import json;print(json.load(open('$repo_root/apps/desktop/src-tauri/tauri.conf.json'))['identifier'])")"
  [[ "$APPLE_SIGNING_IDENTITY" =~ \(([A-Z0-9]{10})\)$ ]] \
    || fail "cannot read the 10-character team id from APPLE_SIGNING_IDENTITY ('$APPLE_SIGNING_IDENTITY'): it must end in (TEAMID)."
  team_id="${BASH_REMATCH[1]}"
  identity_line="$(grep -F -- "$APPLE_SIGNING_IDENTITY" <<<"$identities" || true)"
  identity_sha1="$(awk 'NR==1 {print $2}' <<<"$identity_line")"
  if [[ "${RELEASE_SKIP_PROFILE:-0}" == "1" ]]; then
    [[ -n "$smoke_version" ]] \
      || fail "RELEASE_SKIP_PROFILE=1 is only allowed for a smoke build (--smoke-version): a real release must carry the keychain entitlement."
    skip_profile=1
    echo "note: RELEASE_SKIP_PROFILE=1 — smoke build WITHOUT the provisioning profile: no data-protection keychain (password-only)."
  else
    : "${APPLE_PROVISIONING_PROFILE:?set it (source ~/.config/personal-cfo/release.env) — the macOS Developer ID provisioning profile, see docs/operations/release-signing.md A7}"
    [[ -f "$APPLE_PROVISIONING_PROFILE" ]] || fail "APPLE_PROVISIONING_PROFILE does not point at a file: $APPLE_PROVISIONING_PROFILE"
    python3 -I "$repo_root/scripts/check-provisioning-profile.py" \
      --profile "$APPLE_PROVISIONING_PROFILE" --bundle-id "$app_bundle_id" --team-id "$team_id" \
      --min-days 90 --cert-sha1 "$identity_sha1" \
      || fail "the provisioning profile is not usable for this release (see above). Renew or replace it: docs/operations/release-signing.md A7."
  fi

  if [[ "$skip_notarize" == "1" ]]; then
    echo "note: RELEASE_SKIP_NOTARIZE=1 — signing only, skipping notarization (artifact will NOT pass Gatekeeper)."
  else
    : "${APPLE_API_KEY:?notarization needs the App Store Connect Key ID (A4)}"
    : "${APPLE_API_ISSUER:?notarization needs the Issuer ID (A4)}"
    : "${APPLE_API_KEY_PATH:?notarization needs the path to AuthKey_<KEYID>.p8 (A4)}"
    [[ -f "$APPLE_API_KEY_PATH" ]] || fail "APPLE_API_KEY_PATH does not point at a file: $APPLE_API_KEY_PATH"
  fi

  # The updater signing keypair (A6) — required for every real release build, whether or
  # not notarization is skipped: the DMG can be sign-only for fast iteration, but a
  # release with no updater signature would ship an app the updater can never verify
  # future updates against, and there is no "skip" flag for this one (personal-cfo-867.1.2:
  # "release.sh refuses to build without the signing vars").
  : "${TAURI_SIGNING_PRIVATE_KEY:?set it (source ~/.config/personal-cfo/release.env) — see docs/operations/release-signing.md A6}"
  : "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:?set it (source ~/.config/personal-cfo/release.env) — see docs/operations/release-signing.md A6}"

  if [[ "$skip_notarize" == "1" ]]; then
    echo "✓ preflight passed — signing identity + updater key valid (notarization skipped)."
  else
    echo "✓ preflight passed — signing identity + updater key valid, notarization credentials present."
  fi
}

preflight

if [[ "$check_only" == "1" ]]; then
  echo "Preflight only (--check): not building."
  exit 0
fi

if [[ -z "$notes" ]]; then
  fail "no release notes given — set RELEASE_NOTES or pass --notes \"…\" (latest.json needs a real notes field, not a guess)."
fi
# ── Smoke-build guard ────────────────────────────────────────────────────────
# A smoke build and a real release differ only by these two flags, so the failure mode
# worth designing against is a smoke artifact being published as a real release by
# mistake. The two are therefore REQUIRED TOGETHER and the version must be a 0.9.x
# throwaway: a smoke build cannot silently carry the real endpoint, and a build carrying
# the smoke endpoint cannot silently claim a real version number.
if [[ -n "$smoke_endpoint" || -n "$smoke_version" ]]; then
  [[ -n "$smoke_endpoint" && -n "$smoke_version" ]] \
    || fail "--smoke-endpoint and --smoke-version must be given together (a smoke build needs both a throwaway endpoint and a throwaway version)."
  [[ "$smoke_version" =~ ^0\.9\.[0-9]+$ ]] \
    || fail "--smoke-version must be a 0.9.x throwaway (got '$smoke_version') — real release versions come from tauri.conf.json, never a flag."
  echo "⚠  SMOKE BUILD — not a release. Overrides for this build only:"
  echo "     version:  $smoke_version (tauri.conf.json is NOT edited)"
  echo "     endpoint: $smoke_endpoint"
  echo "   Never publish these artifacts as a real release."
fi

# A release is reproducible from a committed state; refuse a dirty tree unless
# overridden. --porcelain catches untracked (non-ignored) files too, not just
# tracked edits — build artifacts under target/ and dist/ are gitignored, so they
# don't count.
if [[ "${RELEASE_ALLOW_DIRTY:-0}" != "1" && -n "$(git -C "$repo_root" status --porcelain 2>/dev/null)" ]]; then
  fail "worktree is dirty (tracked or untracked) — commit/clean first, or set RELEASE_ALLOW_DIRTY=1."
fi

# The build's effective version: the committed one, unless a smoke build overrode it —
# latest.json and the log line must both name what was ACTUALLY built, or a smoke
# manifest would advertise the real version number.
if [[ -n "$smoke_version" ]]; then
  version="$smoke_version"
else
  version="$(python3 -c "import json;print(json.load(open('$repo_root/apps/desktop/src-tauri/tauri.conf.json'))['version'])")"
fi
echo "Building DohFlow ${version} (signed$([[ "$skip_notarize" == "1" ]] && echo '' || echo ' + notarized'))…"

cd "$repo_root/apps/desktop"

# Ensure deps match the lockfile exactly for a reproducible release.
pnpm install --frozen-lockfile

# Inject the signing identity (and, only for a smoke build, the updater endpoint
# override) via a --config file so neither ever enters the committed tauri.conf.json
# (python emits properly-escaped JSON). `tauri build --config` DEEP-MERGES into
# tauri.conf.json, but we also re-assert hardenedRuntime here so that even if the merge
# ever behaved as a replace, the shipped app can never lose hardened runtime (which
# notarization REQUIRES). Defense-in-depth on the one thing that must not silently
# regress.
cfg_dir="$(mktemp -d)"
trap 'rm -rf "$cfg_dir"' EXIT
cfg="$cfg_dir/signing.json"
# The keychain entitlement (personal-cfo-md8h2.17): the template's @TEAM_ID@ / @BUNDLE_ID@ are filled in
# here (nothing build-specific is committed) and the profile is embedded as
# Contents/embedded.provisionprofile through the bundler's `files` map. A smoke build that skips the
# profile gets neither.
entitlements_file=""
profile_file=""
if [[ "$skip_profile" != "1" ]]; then
  entitlements_file="$cfg_dir/entitlements.plist"
  python3 -c "
import sys
template, team, bundle, out = sys.argv[1:5]
open(out, 'w').write(open(template).read().replace('@TEAM_ID@', team).replace('@BUNDLE_ID@', bundle))
" "$repo_root/apps/desktop/src-tauri/entitlements/macos-release.plist.in" "$team_id" "$app_bundle_id" "$entitlements_file"
  plutil -lint "$entitlements_file" >/dev/null || fail "the generated entitlements plist is not valid: $entitlements_file"
  profile_file="$APPLE_PROVISIONING_PROFILE"
fi
python3 -c "
import json, sys
identity, endpoint, smoke_version, out, entitlements, profile = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5], sys.argv[6]
conf = {'bundle': {'macOS': {'signingIdentity': identity, 'hardenedRuntime': True}}}
if entitlements:
    conf['bundle']['macOS']['entitlements'] = entitlements
    conf['bundle']['macOS']['files'] = {'embedded.provisionprofile': profile}
if endpoint:
    conf['plugins'] = {'updater': {'endpoints': [endpoint]}}
if smoke_version:
    # Same deep-merge as the signing identity: the smoke build gets its throwaway version
    # WITHOUT the committed tauri.conf.json ever being edited, so the worktree stays clean
    # (release.sh refuses a dirty tree) and a 0.9.x can never be committed by accident.
    conf['version'] = smoke_version
json.dump(conf, open(out, 'w'))
" "$APPLE_SIGNING_IDENTITY" "$smoke_endpoint" "$smoke_version" "$cfg" "$entitlements_file" "$profile_file"

# Notarization is automatic when the APPLE_API_* creds are in the environment. For a
# sign-only build, hide them from the bundler so it signs and stops there. Unsetting
# APPLE_API_KEY (the Key ID) + APPLE_API_ISSUER is what disables it: the bundler needs
# both to authenticate, so even though the .p8 still sits under
# ~/.appstoreconnect/private_keys/, it can't construct or use a request without them.
if [[ "$skip_notarize" == "1" ]]; then
  env -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH -u APPLE_ID -u APPLE_PASSWORD \
    pnpm tauri build --target universal-apple-darwin --bundles app,dmg --config "$cfg"
else
  pnpm tauri build --target universal-apple-darwin --bundles app,dmg --config "$cfg"
fi

# ── Verify (never skip): a build that "succeeded" but didn't actually sign is worse
# than a failure, because it looks shippable. Prove the signature + Gatekeeper verdict. ──
# `--target universal-apple-darwin` (ADR 0072) moves cargo/Tauri's own output
# under a target-triple-named directory — confirmed empirically against a
# real build, not assumed: `target/release/...` (no target dir) is where a
# plain, non-`--target` build would have landed, and is stale since 0zlg9.
bundle_dir="$repo_root/apps/desktop/src-tauri/target/universal-apple-darwin/release/bundle"
app="$bundle_dir/macos/DohFlow.app"
[[ -d "$app" ]] || fail "build finished but the .app is missing: $app"
shopt -s nullglob
dmgs=("$bundle_dir"/dmg/*.dmg)
shopt -u nullglob
[[ ${#dmgs[@]} -ge 1 ]] || fail "build finished but no .dmg was produced."
dmg="${dmgs[0]}"

# ADR 0072: prove the shipped binary is actually universal, not just that the
# build succeeded — `--target universal-apple-darwin` invokes `lipo` under the
# hood, but a "succeeded" build that silently dropped a slice would still pass
# every check above.
binary="$app/Contents/MacOS/personal-cfo-desktop"
[[ -f "$binary" ]] || fail "build finished but the expected executable is missing: $binary"
archs="$(lipo -archs "$binary")"
case " $archs " in
  *' arm64 '*) ;;
  *) fail "universal binary is missing the arm64 slice — lipo -archs reported: $archs" ;;
esac
case " $archs " in
  *' x86_64 '*) ;;
  *) fail "universal binary is missing the x86_64 slice — lipo -archs reported: $archs" ;;
esac
echo "✓ universal binary confirmed — lipo -archs: $archs"

# createUpdaterArtifacts (tauri.conf.json) makes `tauri build` emit the signed updater
# sidecar files alongside the .app — VERIFY-ON-BUILD: confirmed the exact naming/location
# once run for real; expected next to the .app per Tauri v2's documented macOS updater
# artifact layout.
updater_archive="$bundle_dir/macos/DohFlow.app.tar.gz"
updater_sig="$updater_archive.sig"
[[ -f "$updater_archive" ]] || fail "build finished but the updater archive is missing: $updater_archive (is bundle.createUpdaterArtifacts true in tauri.conf.json, and were TAURI_SIGNING_* set?)"
[[ -f "$updater_sig" ]] || fail "build finished but the updater signature is missing: $updater_sig"

# Tauri notarizes + staples the .app, then wraps it in a DMG it SIGNS but does not
# notarize/staple. Staple the DMG too so the CONTAINER a user downloads also validates
# offline (the .app inside is already stapled). The app was just notarized, so this
# second submission is quick.
if [[ "$skip_notarize" != "1" ]]; then
  echo
  echo "── Notarizing + stapling the DMG container ─────────────────────"
  xcrun notarytool submit "$dmg" \
    --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER" --wait \
    || fail "DMG notarization failed — fetch the log with: xcrun notarytool log <id> --key \"\$APPLE_API_KEY_PATH\" --key-id \"\$APPLE_API_KEY\" --issuer \"\$APPLE_API_ISSUER\""
  xcrun stapler staple "$dmg" || fail "stapling the DMG failed."
fi

echo
echo "── Verifying signature ─────────────────────────────────────────"
codesign --verify --deep --strict --verbose=2 "$app" \
  || fail "codesign verification FAILED — the app is not validly signed (was APPLE_SIGNING_IDENTITY picked up?)."
# Show the signing authority so a wrong/absent Developer ID is obvious. (--verify above
# is the real gate; this is a human-readable confirmation of WHO signed it.)
authority="$(codesign -dvv "$app" 2>&1 | grep -E "Authority=|TeamIdentifier=" || true)"
if [[ -n "$authority" ]]; then
  echo "$authority"
else
  echo "note: signature verified, but 'codesign -dvv' printed no Authority line (unexpected — inspect manually)."
fi

# The keychain entitlement and its profile (personal-cfo-md8h2.17): prove the SIGNED app carries exactly
# the three keychain/identity entitlements and the very profile that was checked, so a build that signed
# without them (the app would then fail its keychain calls, or not launch) cannot look shippable.
if [[ "$skip_profile" != "1" ]]; then
  echo
  echo "── Verifying the keychain entitlement + embedded provisioning profile ──"
  embedded="$app/Contents/embedded.provisionprofile"
  [[ -f "$embedded" ]] || fail "the app has no Contents/embedded.provisionprofile: the profile was not embedded (bundle.macOS.files)."
  cmp -s "$embedded" "$APPLE_PROVISIONING_PROFILE" \
    || fail "the embedded provisioning profile differs from APPLE_PROVISIONING_PROFILE."
  codesign -d --entitlements :- "$app" 2>/dev/null \
    | python3 -I "$repo_root/scripts/check-signed-entitlements.py" --bundle-id "$app_bundle_id" --team-id "$team_id" \
    || fail "the signed app's entitlements are not exactly the keychain group (see above)."
fi

# LAUNCH GATE (personal-cfo-md8h2.17, 03a): a missing or unauthorized profile may stop the app launching,
# not only the keychain. Launch the signed binary through the smoke hook (setup() runs, then exit 0).
# HOME is pointed at a throwaway directory so the app's data directory is NOT the owner's real one
# (a release build ignores PCFO_DATA_DIR, ADR 0070; the app derives its directory from $HOME, which was
# confirmed by running a throwaway-identity bundle with HOME overridden).
echo
echo "── Launch gate: the signed app starts ──────────────────────────"
launch_home="$cfg_dir/launch-home"
mkdir -p "$launch_home"
launch_out="$(HOME="$launch_home" PCFO_SMOKE_TEST_EXIT=1 "$binary" 2>&1)" \
  || fail "the signed app did not launch cleanly (exit non-zero). Output: $launch_out"
grep -qF -- "setup() completed successfully" <<<"$launch_out" \
  || fail "the signed app ran but did not report a completed setup(). Output: $launch_out"
echo "✓ the signed app launches and completes setup()"
if [[ "$skip_profile" != "1" ]]; then
  echo "note: the data-protection keychain attribute smoke (device-only class, not synchronizable, read back)"
  echo "      is a manual step on the signed release candidate for now (release-checklist.md step 5);"
  echo "      the harness for it is personal-cfo-md8h2.18 (a separate throwaway signed bundle, never shipped)."
fi

if [[ "$skip_notarize" == "1" ]]; then
  echo
  echo "⚠  Sign-only build (RELEASE_SKIP_NOTARIZE=1): signed but NOT notarized/stapled."
  echo "   It will still trip Gatekeeper on other Macs. Re-run without the flag for a real release."
else
  echo
  echo "── Verifying notarization staple + Gatekeeper ──────────────────"
  xcrun stapler validate "$app" || fail "the .app has no notarization ticket stapled."
  xcrun stapler validate "$dmg" || fail "the .dmg has no notarization ticket stapled."
  # The real Gatekeeper verdict a downloader's Mac will apply.
  spctl -a -vvv --type exec "$app" \
    || fail "Gatekeeper REJECTED the .app (spctl) — not notarized/accepted."
  spctl -a -vvv --type open --context context:primary-signature "$dmg" \
    || echo "note: spctl could not assess the DMG directly; the stapled .app inside is the artifact that matters." >&2
fi

echo
echo "── Writing latest.json ──────────────────────────────────────────"
# Universal binary (ADR 0072): the SAME archive serves both architectures, so
# platforms.darwin-aarch64 and platforms.darwin-x86_64 carry IDENTICAL url +
# signature — there is no second file to point to. The pinned
# tauri-plugin-updater=2.11.0 selects its entry by the running binary's
# cfg!(target_arch), so this one manifest correctly serves both.
latest_json="$bundle_dir/macos/latest.json"
python3 "$repo_root/scripts/write-latest-json.py" "$version" "$notes" "$updater_sig" "$latest_json" "$repo_root/apps/desktop/src-tauri/tauri.conf.json"
echo "  wrote: $latest_json"
echo "  ⚠  its platforms.darwin-aarch64.url and platforms.darwin-x86_64.url are"
echo "     placeholders — fill in the real GitHub release asset URL before"
echo "     publishing (release-checklist.md, 867.1.3)."

echo
echo "✓ Release artifacts ready:"
echo "  app:             $app"
echo "  dmg:             $dmg"
echo "  updater archive: $updater_archive"
echo "  updater sig:     $updater_sig"
echo "  latest.json:     $latest_json"
if [[ "$skip_notarize" != "1" ]]; then
  echo
  echo "Next: distribute the .dmg. First-release smoke test (B6): download it via a"
  echo "browser on a second Mac and confirm it opens with no Gatekeeper warning."
fi
if [[ -n "$smoke_endpoint" ]]; then
  echo
  echo "Smoke build: upload the .app.tar.gz + .sig content (as latest.json's signature)"
  echo "to a release at the smoke endpoint's repo, matching the version above, then"
  echo "point an installed 0.9.0 build at $smoke_endpoint and confirm it offers this update."
fi
