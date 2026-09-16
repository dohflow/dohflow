#!/usr/bin/env bash
#
# Regression tests for scripts/publish-release.sh — bead personal-cfo-867.1.3.
#
# WHAT IT COVERS
#   The script's decision logic (version-consistency preflight, changelog-entry
#   check, dirty-tree/existing-tag refusal, the updater-signature structural
#   sanity check, latest.json URL patching, changelog-section extraction for
#   release notes, and the rebuild/verify/publish command sequencing) — run
#   against real throwaway git repositories with the ACTUAL script under test,
#   exactly like scripts/tests/value-scan.test.sh does. `gh` and `curl` are
#   stubbed (fake executables on a prepended PATH) so nothing here ever makes a
#   network call, touches the real dohflow/dohflow repo, or needs GPG/SSH tag
#   signing configured on the machine running the test.
#
# WHAT IT DOES NOT COVER
#   `git tag -s` itself (needs real signing config — release-signing.md's
#   one-time owner setup) and the full `scripts/release.sh` build pipeline
#   (needs Apple/Tauri signing credentials). Both are exercised for real only
#   by an owner running the actual release procedure
#   (docs/operations/release-checklist.md).
#
# RUN
#   bash scripts/tests/publish-release.test.sh
#
#   KEEP_WORKDIR=1 leaves the temp tree in place for inspection.

set -uo pipefail   # deliberately not -e: every case must run and report.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/publish-release.sh"
[ -f "$SCRIPT_UNDER_TEST" ] || { echo "no script at $SCRIPT_UNDER_TEST" >&2; exit 1; }

# preflight now asserts the CHANGELOG heading is dated TODAY (personal-cfo-nj93r),
# not just "dated" — fixtures use the real date so the passing cases stay
# correct no matter which day this suite runs.
TODAY="$(date +%Y-%m-%d)"

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/publish-release-test.XXXXXX")" || exit 1
cleanup() {
  if [ -n "${KEEP_WORKDIR:-}" ]; then
    echo "workdir kept: $WORKDIR"
    return
  fi
  # Guarded: only ever removes the directory this run created (AGENTS.md §1).
  case "$WORKDIR" in
    */publish-release-test.??????) rm -rf "$WORKDIR" ;;
    *) echo "refusing to remove unexpected workdir: $WORKDIR" >&2 ;;
  esac
}
trap cleanup EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
assert_eq() {  # <label> <actual> <expected>
  if [ "$2" = "$3" ]; then pass "$1"; else fail "$1 (expected '$3', got '$2')"; fi
}
assert_contains() {  # <label> <haystack> <needle>
  case "$2" in *"$3"*) pass "$1" ;; *) fail "$1 (no '$3' in: $2)" ;; esac
}
assert_not_contains() {  # <label> <haystack> <needle>
  case "$2" in *"$3"*) fail "$1 (unexpectedly found '$3' in: $2)" ;; *) pass "$1" ;; esac
}
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

# ── Fake tools ─────────────────────────────────────────────────────────────
# One shared fake-bin directory, prepended to PATH for every case. Behavior is
# steered per-invocation via env vars the test sets before calling the script.
FAKE_BIN="$WORKDIR/fake-bin"
mkdir -p "$FAKE_BIN"

cat > "$FAKE_BIN/gh" <<'GH_EOF'
#!/usr/bin/env bash
[ -n "${FAKE_GH_LOG:-}" ] && printf '%s\n' "$*" >> "$FAKE_GH_LOG"
prev=""
for a in "$@"; do
  if [ "$prev" = "--notes-file" ] && [ -n "${FAKE_GH_NOTES_CAPTURE:-}" ]; then
    cp "$a" "$FAKE_GH_NOTES_CAPTURE" 2>/dev/null || true
  fi
  prev="$a"
done
# `release download <tag> --repo <repo> --pattern latest.json --dir <dir>
# --clobber` (personal-cfo-xvj0k's verify_manifest_url, called twice: once
# to read the manifest as it currently is, once more after a correction to
# prove the correction actually took). Models a real GitHub release's
# server-side state with a plain file: FAKE_GH_DOWNLOAD_STATE, if it
# already exists, IS what the release currently serves (an upload
# overwrites it, below) — falling back to FAKE_GH_DOWNLOAD_SEED for the
# very first download of a test case that never uploaded anything yet.
if [ "$1" = "release" ] && [ "$2" = "download" ]; then
  dir="."
  prevarg=""
  for a in "$@"; do
    [ "$prevarg" = "--dir" ] && dir="$a"
    prevarg="$a"
  done
  mkdir -p "$dir"
  if [ -n "${FAKE_GH_DOWNLOAD_STATE:-}" ] && [ -f "$FAKE_GH_DOWNLOAD_STATE" ]; then
    cp "$FAKE_GH_DOWNLOAD_STATE" "$dir/latest.json"
  elif [ -n "${FAKE_GH_DOWNLOAD_SEED:-}" ]; then
    cp "$FAKE_GH_DOWNLOAD_SEED" "$dir/latest.json"
  fi
  # FAKE_GH_DOWNLOAD_EXIT, when set, fails ONLY this subcommand (e.g. "gh
  # couldn't find latest.json on the release at all") without also failing
  # the `release edit --draft=false` call that always runs first in
  # `publish` — falls back to the general FAKE_GH_EXIT otherwise.
  exit "${FAKE_GH_DOWNLOAD_EXIT:-${FAKE_GH_EXIT:-0}}"
fi
# `release upload <tag> --repo <repo> --clobber <file>...` — one or more
# files in one call (verify_manifest_url always uploads latest.json and
# SHA256SUMS.txt together, since correcting the manifest invalidates its
# checksum line). Captures each by its own basename, not "whichever arg
# came last", since a multi-file upload has no single "last" file that
# means anything. Uploading a file named latest.json also becomes the new
# FAKE_GH_DOWNLOAD_STATE, so a subsequent `release download` in the same
# test case sees the correction for real, the same way a real release
# would.
if [ "$1" = "release" ] && [ "$2" = "upload" ]; then
  for a in "$@"; do
    case "$a" in
      */latest.json)
        [ -n "${FAKE_GH_UPLOAD_CAPTURE:-}" ] && cp "$a" "$FAKE_GH_UPLOAD_CAPTURE" 2>/dev/null || true
        [ -n "${FAKE_GH_DOWNLOAD_STATE:-}" ] && cp "$a" "$FAKE_GH_DOWNLOAD_STATE" 2>/dev/null || true
        ;;
      */SHA256SUMS.txt)
        [ -n "${FAKE_GH_UPLOAD_SUMS_CAPTURE:-}" ] && cp "$a" "$FAKE_GH_UPLOAD_SUMS_CAPTURE" 2>/dev/null || true
        ;;
    esac
  done
fi
exit "${FAKE_GH_EXIT:-0}"
GH_EOF
chmod +x "$FAKE_BIN/gh"

cat > "$FAKE_BIN/curl" <<'CURL_EOF'
#!/usr/bin/env bash
[ -n "${FAKE_CURL_LOG:-}" ] && printf '%s\n' "$*" >> "$FAKE_CURL_LOG"
has_post=0
has_w=0
for a in "$@"; do
  [ "$a" = "POST" ] && has_post=1
  [ "$a" = "-w" ] && has_w=1
done
if [ "$has_post" = "1" ]; then
  exit "${FAKE_CURL_POST_EXIT:-0}"
elif [ "$has_w" = "1" ]; then
  printf '%s' "${FAKE_CURL_STATUS:-200}"
  exit 0
else
  printf '%s' "${FAKE_CURL_PAGE_BODY:-}"
  exit "${FAKE_CURL_PAGE_EXIT:-0}"
fi
CURL_EOF
chmod +x "$FAKE_BIN/curl"

TEST_PATH="$FAKE_BIN:$PATH"

# ── Fixture builder ──────────────────────────────────────────────────────────
# A real, complete throwaway git repo with the script under test at its real
# relative path, a fixture tauri.conf.json/Cargo.toml/package.json/CHANGELOG.md
# all agreeing on version "0.1.0", and a local bare "origin" remote so
# `git ls-remote --tags origin` works fully offline.
#
# When `minisign` is installed (it now is, personal-cfo-867.1.3 go-live day —
# this suite must not silently only test the structural path just because an
# earlier run happened on a machine without it), generates a REAL ephemeral
# keypair so the "good" fixture's signature (built in make_sig) can pass an
# actual cryptographic verification, not just the structural check. Falls
# back to a synthetic pubkey (random key id + random body — never a real
# signing key) when minisign isn't available, matching the CI environment,
# where only the structural path runs.
KEYID_HEX=""
SECKEY_PATH=""
HAVE_MINISIGN=0
command -v minisign >/dev/null 2>&1 && HAVE_MINISIGN=1
new_case_repo() {
  CASE="$WORKDIR/case-$CASES"
  REPO="$CASE/repo"
  ORIGIN="$CASE/origin.git"
  mkdir -p "$REPO/scripts/tests" "$REPO/apps/desktop/src-tauri"

  cp "$SCRIPT_UNDER_TEST" "$REPO/scripts/publish-release.sh"
  chmod +x "$REPO/scripts/publish-release.sh"

  if [ "$HAVE_MINISIGN" = "1" ]; then
    SECKEY_PATH="$CASE/minisign.key"
    local pubkey_path="$CASE/minisign.pub"
    minisign -G -W -f -s "$SECKEY_PATH" -p "$pubkey_path" -q >/dev/null 2>&1
    PUBKEY_B64="$(python3 -c "
import base64
print(base64.b64encode(open('$pubkey_path', 'rb').read()).decode())
")"
    KEYID_HEX="$(python3 -c "
import base64
raw = base64.b64decode(open('$pubkey_path').read().splitlines()[1])
print(raw[2:10].hex().upper())
")"
  else
    SECKEY_PATH=""
    KEYID_HEX="$(python3 -c "import os; print(os.urandom(8).hex().upper())")"
    PUBKEY_B64="$(python3 -c "
import base64, os
keyid_hex = '$KEYID_HEX'
body = b'RW' + bytes.fromhex(keyid_hex) + os.urandom(32)
text = 'untrusted comment: minisign public key: ' + keyid_hex + '\n' + base64.b64encode(body).decode() + '\n'
print(base64.b64encode(text.encode()).decode())
")"
  fi

  cat > "$REPO/apps/desktop/src-tauri/tauri.conf.json" <<EOF
{
  "version": "0.1.0",
  "plugins": {
    "updater": {
      "pubkey": "$PUBKEY_B64",
      "endpoints": ["https://github.com/dohflow/dohflow/releases/latest/download/latest.json"]
    }
  }
}
EOF

  cat > "$REPO/apps/desktop/src-tauri/Cargo.toml" <<'EOF'
[package]
name = "personal-cfo-desktop"
version = "0.1.0"
EOF

  mkdir -p "$REPO/apps/desktop"
  cat > "$REPO/apps/desktop/package.json" <<'EOF'
{"name": "personal-cfo-desktop", "version": "0.1.0"}
EOF

  cat > "$REPO/CHANGELOG.md" <<EOF
# Changelog

## [Unreleased]

## [0.1.0] - $TODAY

### Added

- First real release line.
- Second real release line.

## [0.0.9] - 2026-01-01

- Older, unrelated section — must never leak into 0.1.0's extracted notes.
EOF

  ( cd "$REPO" \
    && git init -q \
    && git config user.email test@example.com \
    && git config user.name "Test" \
    && git config commit.gpgsign false \
    && git config tag.gpgsign false \
    && git add -A \
    && git commit -q -m "fixture" )

  git init -q --bare "$ORIGIN"
  ( cd "$REPO" && git remote add origin "$ORIGIN" )
}

# Builds a valid-shaped (or deliberately corrupted) minisign signature file
# for the current case's key id, matching the REAL on-disk shape Tauri
# produces (verified against an actual build, personal-cfo-867.1.3 go-live
# day): the file's raw content is base64 of the FULL multi-line minisign
# signature-file text (untrusted comment, the base64 signature line, trusted
# comment, global signature line) — not base64 of the raw signature bytes
# directly.
#
# "good" mode, when minisign is installed and new_case_repo generated a real
# keypair: actually signs $archive with minisign, so the fixture passes a
# genuine cryptographic verification, not only the structural check — a
# fixture with a random-bytes "signature" always fails minisign's own -V,
# which a synthetic-only fixture would never have caught (this is exactly
# the class of gap that let the real sanity_check_sig bug ship unnoticed
# until it ran against an actual Tauri build). Falls back to a structurally
# valid but not-really-signed blob (algorithm id "ED", a real Tauri build's
# choice — prehashed) when minisign isn't installed, since only the
# structural path is reachable there.
make_sig() {  # <out-path> <mode: good|badkeyid|badlen|badalgo> [archive-to-really-sign, for good mode]
  local out="$1" mode="$2" archive="${3:-}"
  if [ "$mode" = "good" ] && [ -n "$SECKEY_PATH" ] && [ -n "$archive" ]; then
    local plain="$out.plain"
    minisign -S -s "$SECKEY_PATH" -m "$archive" -x "$plain" -q >/dev/null 2>&1
    python3 -c "
import base64
open('$out', 'w').write(base64.b64encode(open('$plain', 'rb').read()).decode())
"
    rm -f "$plain"
    return
  fi
  python3 -c "
import base64, os
keyid_hex, mode, out = '$KEYID_HEX', '$mode', '$out'
keyid = bytes.fromhex(keyid_hex)
if mode == 'good':
    sig = b'ED' + keyid + os.urandom(64)
elif mode == 'badkeyid':
    sig = b'Ed' + bytes(8) + os.urandom(64)
elif mode == 'badlen':
    sig = b'Ed' + keyid + os.urandom(10)
elif mode == 'badalgo':
    sig = b'XX' + keyid + os.urandom(64)
else:
    raise SystemExit(f'unknown mode {mode}')
sig_line = base64.b64encode(sig).decode()
text = ('untrusted comment: signature from tauri secret key\n' + sig_line
        + '\ntrusted comment: timestamp:0\tfile:fixture.tar.gz\n'
        + base64.b64encode(os.urandom(64)).decode() + '\n')
open(out, 'w').write(base64.b64encode(text.encode()).decode())
"
}

# Places a full, buildable set of scripts/release.sh's output artifacts
# (fake content — only shapes/paths matter to publish-release.sh) so `package`
# has something to assemble. `sig_mode` selects make_sig's corruption mode;
# `latest_json_url` lets a case pre-fill latest.json's URL to something other
# than the expected placeholder.
place_build_artifacts() {  # <repo> [sig_mode=good] [latest_json_url=placeholder]
  local repo="$1" sig_mode="${2:-good}" url="${3:-REPLACE_WITH_THE_UPLOADED_APP_TAR_GZ_ASSET_URL}"
  local macos_dir="$repo/apps/desktop/src-tauri/target/release/bundle/macos"
  local dmg_dir="$repo/apps/desktop/src-tauri/target/release/bundle/dmg"
  mkdir -p "$macos_dir" "$dmg_dir"
  mkdir -p "$macos_dir/DohFlow.app"
  echo "fake dmg" > "$dmg_dir/DohFlow_0.1.0_aarch64.dmg"
  echo "fake app archive" > "$macos_dir/DohFlow.app.tar.gz"
  make_sig "$macos_dir/DohFlow.app.tar.gz.sig" "$sig_mode" "$macos_dir/DohFlow.app.tar.gz"
  python3 -c "
import json
json.dump({
    'version': '0.1.0',
    'notes': 'fixture',
    'pub_date': '2026-09-12T00:00:00Z',
    'platforms': {'darwin-aarch64': {'url': '$url', 'signature': 'irrelevant-for-these-tests'}},
}, open('$macos_dir/latest.json', 'w'), indent=2)
"
}

run_script() {  # <repo> <args...>  — runs with fake PATH, captures stdout+stderr and exit code
  local repo="$1"; shift
  OUT="$(cd "$repo" && PATH="$TEST_PATH" bash scripts/publish-release.sh "$@" 2>&1)"
  CODE=$?
}

# Writes a minimal, valid latest.json-shaped file at <path> with the given
# darwin-aarch64 url. Used to seed what a stubbed `release download`
# returns (personal-cfo-xvj0k's verify_manifest_url) — deliberately
# separate from `place_build_artifacts`' local $assets_dir copy, since the
# whole point of the fix is that those two can independently disagree.
#
# pub_date is DELIBERATELY different from place_build_artifacts' fixture
# (2026-09-12) — review round 1 (F1) found that when the only field that
# ever differed between the two fixtures was the url itself, a corrected
# manifest built from the downloaded copy came out byte-identical to what
# `package` had already hashed, so skipping the SHA256SUMS.txt
# regeneration entirely was undetectable (all 25 cases stayed green).
# Verified: with this date change, deleting the regeneration at
# publish-release.sh's verify_manifest_url turns exactly the checksum
# assertion in the xvj0k regression test red, with a real hash mismatch —
# not just "stays green" as a fluke of identical fixtures.
write_manifest() {  # <path> <url>
  python3 -c "
import json
json.dump({
    'version': '0.1.0',
    'notes': 'fixture',
    'pub_date': '2026-09-10T00:00:00Z',
    'platforms': {'darwin-aarch64': {'url': '$2', 'signature': 'irrelevant-for-these-tests'}},
}, open('$1', 'w'), indent=2)
"
}

# ══════════════════════════════════════════════════════════════════════════
# preflight
# ══════════════════════════════════════════════════════════════════════════

case_start "preflight passes on a clean, consistent, dated-changelog repo"
new_case_repo
run_script "$REPO" preflight
assert_eq "exit code" "$CODE" "0"
assert_contains "output" "$OUT" "preflight passed"

case_start "preflight fails on apps/desktop/package.json version mismatch"
new_case_repo
python3 -c "import json; json.dump({'name':'x','version':'0.0.0'}, open('$REPO/apps/desktop/package.json','w'))"
run_script "$REPO" preflight
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "package.json version is '0.0.0'"

case_start "preflight fails on a placeholder (undated) CHANGELOG entry"
new_case_repo
cat > "$REPO/CHANGELOG.md" <<'EOF'
# Changelog

## [Unreleased]

## [0.1.0] - YYYY-MM-DD

- Not released yet.
EOF
run_script "$REPO" preflight
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "no dated '## [0.1.0]"

case_start "preflight fails when the CHANGELOG heading is dated but not today (stale go-live date)"
new_case_repo
cat > "$REPO/CHANGELOG.md" <<'EOF'
# Changelog

## [Unreleased]

## [0.1.0] - 2020-01-01

- Dated, but not today — a go-live slip nobody updated the heading for.
EOF
run_script "$REPO" preflight
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "is dated 2020-01-01, but today is $TODAY"

case_start "preflight fails on a dirty tree, RELEASE_ALLOW_DIRTY=1 overrides"
new_case_repo
echo "uncommitted" > "$REPO/scratch.txt"
run_script "$REPO" preflight
assert_eq "dirty tree exit code" "$CODE" "1"
assert_contains "output" "$OUT" "worktree is dirty"
OUT=""; CODE=""
( cd "$REPO" && RELEASE_ALLOW_DIRTY=1 PATH="$TEST_PATH" bash scripts/publish-release.sh preflight > "$CASE/out.txt" 2>&1 )
CODE=$?
OUT="$(cat "$CASE/out.txt")"
assert_eq "RELEASE_ALLOW_DIRTY=1 exit code" "$CODE" "0"

case_start "preflight fails when the tag already exists locally"
new_case_repo
( cd "$REPO" && git tag v0.1.0 )
run_script "$REPO" preflight
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "tag v0.1.0 already exists locally"

case_start "preflight fails when the tag already exists on origin"
new_case_repo
( cd "$REPO" && git tag v0.1.0 && git push -q origin v0.1.0 && git tag -d v0.1.0 )
run_script "$REPO" preflight
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "tag v0.1.0 already exists on origin"

case_start "tag refuses (never attempts git tag -s) when preflight fails"
new_case_repo
echo "uncommitted" > "$REPO/scratch.txt"
run_script "$REPO" tag
assert_eq "exit code" "$CODE" "1"
( cd "$REPO" && git rev-parse -q --verify refs/tags/v0.1.0 > /dev/null 2>&1 )
assert_eq "no tag was created" "$?" "1"

# ══════════════════════════════════════════════════════════════════════════
# package
# ══════════════════════════════════════════════════════════════════════════

case_start "package fails cleanly when scripts/release.sh has not been run"
new_case_repo
run_script "$REPO" package
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "run scripts/release.sh first"

case_start "package succeeds: sanity check passes, SHA256SUMS written, latest.json patched"
new_case_repo
place_build_artifacts "$REPO" good
run_script "$REPO" package
assert_eq "exit code" "$CODE" "0"
assert_contains "output" "$OUT" "structural check passed"
SUMS="$REPO/apps/desktop/src-tauri/target/release/bundle/release-assets/SHA256SUMS.txt"
[ -f "$SUMS" ] && LINES="$(wc -l < "$SUMS" | tr -d ' ')" || LINES="missing"
assert_eq "SHA256SUMS.txt line count" "$LINES" "4"
PATCHED_URL="$(python3 -c "import json; print(json.load(open('$REPO/apps/desktop/src-tauri/target/release/bundle/release-assets/latest.json'))['platforms']['darwin-aarch64']['url'])")"
assert_eq "latest.json url patched" "$PATCHED_URL" "https://github.com/dohflow/dohflow/releases/download/v0.1.0/DohFlow.app.tar.gz"

case_start "package fails when the signature's key id doesn't match tauri.conf.json's pubkey"
new_case_repo
place_build_artifacts "$REPO" badkeyid
run_script "$REPO" package
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "does not match tauri.conf.json pubkey key id"

case_start "package fails when the signature is the wrong length"
new_case_repo
place_build_artifacts "$REPO" badlen
run_script "$REPO" package
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "expected 74"

case_start "package fails when the signature algorithm id isn't Ed"
new_case_repo
place_build_artifacts "$REPO" badalgo
run_script "$REPO" package
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "expected b'Ed'"

case_start "package refuses to patch a latest.json whose url isn't the expected placeholder"
new_case_repo
place_build_artifacts "$REPO" good "https://example.com/already-set.tar.gz"
run_script "$REPO" package
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "refusing to overwrite"

# ══════════════════════════════════════════════════════════════════════════
# draft
# ══════════════════════════════════════════════════════════════════════════

case_start "draft fails cleanly when package has not been run"
new_case_repo
run_script "$REPO" draft
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "run '"

case_start "draft auto-extracts exactly the 0.1.0 CHANGELOG section (not neighboring sections)"
new_case_repo
place_build_artifacts "$REPO" good
run_script "$REPO" package
assert_eq "package precondition exit code" "$CODE" "0"
GH_LOG="$CASE/gh.log"; NOTES_CAP="$CASE/notes-captured.txt"
export FAKE_GH_LOG="$GH_LOG" FAKE_GH_NOTES_CAPTURE="$NOTES_CAP"
run_script "$REPO" draft
unset FAKE_GH_LOG FAKE_GH_NOTES_CAPTURE
assert_eq "exit code" "$CODE" "0"
assert_contains "gh invoked with --verify-tag" "$(cat "$GH_LOG")" "--verify-tag"
assert_contains "gh invoked with --draft" "$(cat "$GH_LOG")" "--draft"
assert_contains "gh uploaded the DMG" "$(cat "$GH_LOG")" "DohFlow.dmg"
assert_contains "gh uploaded SHA256SUMS.txt" "$(cat "$GH_LOG")" "SHA256SUMS.txt"
NOTES_CONTENT="$(cat "$NOTES_CAP" 2>/dev/null || echo "MISSING")"
assert_contains "notes include this version's line" "$NOTES_CONTENT" "First real release line."
assert_not_contains "notes exclude the older section" "$NOTES_CONTENT" "Older, unrelated section"

# ══════════════════════════════════════════════════════════════════════════
# rebuild-site / verify / publish
# ══════════════════════════════════════════════════════════════════════════

case_start "rebuild-site fails clearly when DOHFLOW_SITE_DEPLOY_HOOK_URL is unset"
new_case_repo
( cd "$REPO" && unset DOHFLOW_SITE_DEPLOY_HOOK_URL; PATH="$TEST_PATH" bash scripts/publish-release.sh rebuild-site > "$CASE/out.txt" 2>&1 )
CODE=$?
OUT="$(cat "$CASE/out.txt")"
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "DOHFLOW_SITE_DEPLOY_HOOK_URL"

case_start "rebuild-site POSTs to the hook URL when set"
new_case_repo
CURL_LOG="$CASE/curl.log"
( cd "$REPO" && DOHFLOW_SITE_DEPLOY_HOOK_URL="https://example.com/hook" FAKE_CURL_LOG="$CURL_LOG" PATH="$TEST_PATH" bash scripts/publish-release.sh rebuild-site > "$CASE/out.txt" 2>&1 )
CODE=$?
OUT="$(cat "$CASE/out.txt")"
assert_eq "exit code" "$CODE" "0"
assert_contains "curl POSTed the hook URL" "$(cat "$CURL_LOG")" "https://example.com/hook"

case_start "verify fails when latest.json is not HTTP 200"
new_case_repo
( cd "$REPO" && FAKE_CURL_STATUS="404" PATH="$TEST_PATH" bash scripts/publish-release.sh verify > "$CASE/out.txt" 2>&1 )
CODE=$?
OUT="$(cat "$CASE/out.txt")"
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "HTTP 404"

case_start "verify fails when the download page doesn't mention the version"
new_case_repo
( cd "$REPO" && FAKE_CURL_STATUS="200" FAKE_CURL_PAGE_BODY="<html>no version here</html>" PATH="$TEST_PATH" bash scripts/publish-release.sh verify > "$CASE/out.txt" 2>&1 )
CODE=$?
OUT="$(cat "$CASE/out.txt")"
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "does not (yet) mention"

case_start "verify passes when both checks succeed"
new_case_repo
( cd "$REPO" && FAKE_CURL_STATUS="200" FAKE_CURL_PAGE_BODY="download DohFlow 0.1.0 today" PATH="$TEST_PATH" bash scripts/publish-release.sh verify > "$CASE/out.txt" 2>&1 )
CODE=$?
OUT="$(cat "$CASE/out.txt")"
assert_eq "exit code" "$CODE" "0"

case_start "verify's latest.json check follows redirects (personal-cfo-uxev1: releases/latest/download/* is always a 302 by GitHub's own design; a HEAD without -L would report 302 for every genuinely healthy release, never 200)"
new_case_repo
CURL_LOG="$CASE/curl.log"
( cd "$REPO" && FAKE_CURL_STATUS="200" FAKE_CURL_PAGE_BODY="download DohFlow 0.1.0 today" FAKE_CURL_LOG="$CURL_LOG" PATH="$TEST_PATH" bash scripts/publish-release.sh verify > "$CASE/out.txt" 2>&1 )
CODE=$?
assert_eq "exit code" "$CODE" "0"
LATEST_JSON_CALL="$(grep 'latest.json' "$CURL_LOG")"
assert_contains "the latest.json HEAD request passes -L" "$LATEST_JSON_CALL" "-L"

case_start "publish runs gh edit, verifies the manifest URL, POSTs the rebuild hook, then verify, in that order"
new_case_repo
place_build_artifacts "$REPO" good
run_script "$REPO" package
assert_eq "package precondition exit code" "$CODE" "0"
GH_LOG="$CASE/gh.log"; CURL_LOG="$CASE/curl.log"
SEED="$CASE/seed-manifest.json"
write_manifest "$SEED" "https://github.com/dohflow/dohflow/releases/download/v0.1.0/DohFlow.app.tar.gz"
( cd "$REPO" \
  && DOHFLOW_SITE_DEPLOY_HOOK_URL="https://example.com/hook" \
     FAKE_GH_LOG="$GH_LOG" FAKE_CURL_LOG="$CURL_LOG" \
     FAKE_GH_DOWNLOAD_SEED="$SEED" \
     FAKE_CURL_STATUS="200" FAKE_CURL_PAGE_BODY="DohFlow 0.1.0" \
     PATH="$TEST_PATH" bash scripts/publish-release.sh publish > "$CASE/out.txt" 2>&1 )
CODE=$?
OUT="$(cat "$CASE/out.txt")"
assert_eq "exit code" "$CODE" "0"
assert_contains "gh edit --draft=false was called" "$(cat "$GH_LOG")" "--draft=false"
assert_contains "output confirms the manifest URL" "$OUT" "manifest URL confirmed"
assert_eq "gh was called exactly twice (edit + download, no correction/upload needed)" "$(wc -l < "$GH_LOG" | tr -d ' ')" "2"
# Both the hook POST and the verify checks ran (3 curl calls: POST, status
# check, page fetch) — confirms publish drove rebuild-site AND verify, not
# just the gh edit.
assert_eq "curl was called three times (POST + status + page)" "$(wc -l < "$CURL_LOG" | tr -d ' ')" "3"
assert_contains "one curl call POSTed the hook" "$(cat "$CURL_LOG")" "POST https://example.com/hook"

case_start "personal-cfo-xvj0k: verify_manifest_url corrects latest.json AND regenerates SHA256SUMS.txt when the manifest's own url is stale (reproduces the 2026-09-14 go-live case: the manifest still names an untagged- draft url even though the asset itself already sits at the tag path — a fact this check no longer even looks at)"
new_case_repo
place_build_artifacts "$REPO" good
run_script "$REPO" package
assert_eq "package precondition exit code" "$CODE" "0"
GH_LOG="$CASE/gh.log"
UPLOAD_CAP="$CASE/reuploaded-latest.json"
SUMS_CAP="$CASE/reuploaded-sums.txt"
SEED="$CASE/seed-manifest.json"
STATE="$CASE/server-state.json"
STALE_URL="https://github.com/dohflow/dohflow/releases/download/untagged-abc123/DohFlow.app.tar.gz"
EXPECTED_URL="https://github.com/dohflow/dohflow/releases/download/v0.1.0/DohFlow.app.tar.gz"
write_manifest "$SEED" "$STALE_URL"
( cd "$REPO" \
  && DOHFLOW_SITE_DEPLOY_HOOK_URL="https://example.com/hook" \
     FAKE_GH_LOG="$GH_LOG" FAKE_GH_UPLOAD_CAPTURE="$UPLOAD_CAP" \
     FAKE_GH_UPLOAD_SUMS_CAPTURE="$SUMS_CAP" \
     FAKE_GH_DOWNLOAD_SEED="$SEED" FAKE_GH_DOWNLOAD_STATE="$STATE" \
     FAKE_CURL_STATUS="200" FAKE_CURL_PAGE_BODY="DohFlow 0.1.0" \
     PATH="$TEST_PATH" bash scripts/publish-release.sh publish > "$CASE/out.txt" 2>&1 )
CODE=$?
OUT="$(cat "$CASE/out.txt")"
assert_eq "exit code" "$CODE" "0"
assert_contains "output explains the correction, naming the bead" "$OUT" "personal-cfo-xvj0k"
assert_contains "output confirms the re-download re-check passed, not just that an upload happened" "$OUT" "re-verified by downloading it back"
CORRECTED_URL="$(python3 -c "import json; print(json.load(open('$UPLOAD_CAP'))['platforms']['darwin-aarch64']['url'])" 2>/dev/null || echo MISSING)"
assert_eq "the re-uploaded latest.json carries the CORRECT tag-path URL, not the stale untagged- one it started with" "$CORRECTED_URL" "$EXPECTED_URL"
assert_eq "SHA256SUMS.txt was re-uploaded too (the manifest edit invalidated its old checksum line)" "$([ -f "$SUMS_CAP" ] && echo yes || echo no)" "yes"
RECOMPUTED_HASH="$(shasum -a 256 "$UPLOAD_CAP" | awk '{print $1}')"
SUMS_HASH="$(grep 'latest.json' "$SUMS_CAP" | awk '{print $1}')"
assert_eq "the re-uploaded SHA256SUMS.txt's latest.json line matches a fresh recomputation of the corrected file" "$SUMS_HASH" "$RECOMPUTED_HASH"

case_start "publish fails clearly if latest.json can't be downloaded from the published release to verify"
new_case_repo
place_build_artifacts "$REPO" good
run_script "$REPO" package
assert_eq "package precondition exit code" "$CODE" "0"
( cd "$REPO" \
  && DOHFLOW_SITE_DEPLOY_HOOK_URL="https://example.com/hook" \
     FAKE_GH_DOWNLOAD_EXIT="1" \
     PATH="$TEST_PATH" bash scripts/publish-release.sh publish > "$CASE/out.txt" 2>&1 )
CODE=$?
OUT="$(cat "$CASE/out.txt")"
assert_eq "exit code" "$CODE" "1"
assert_contains "output" "$OUT" "could not download latest.json"

# ══════════════════════════════════════════════════════════════════════════
echo
echo "$CASES case(s), $FAILURES failure(s)."
[ "$FAILURES" -eq 0 ]
