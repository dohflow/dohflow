#!/usr/bin/env bash
# Static contract test for the manual Linux AppImage spike (personal-cfo-xcrsk).
# The hosted run is the evidence; this guard keeps the manual job and its
# research output from silently losing a required leg or becoming a release gate.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
WORKFLOW="$REPO_ROOT/.github/workflows/ci.yml"
DOC="$REPO_ROOT/docs/research/linux-build-spike.md"
FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }
assert_file_contains() {
  if grep -Fq -- "$2" "$1"; then pass "$3"; else fail "$3 (missing '$2' in $1)"; fi
}

[ -f "$WORKFLOW" ] || { echo "no CI workflow at $WORKFLOW" >&2; exit 1; }
[ -f "$DOC" ] || { echo "no Linux spike doc at $DOC" >&2; exit 1; }

case_start "manual AppImage job is scoped and evidence-producing"
assert_file_contains "$WORKFLOW" 'linux-appimage-spike:' 'Linux spike job exists'
assert_file_contains "$WORKFLOW" "if: github.event_name == 'workflow_dispatch'" 'Linux spike is manual-only'
assert_file_contains "$WORKFLOW" 'pnpm tauri build --bundles appimage' 'AppImage build command is present'
assert_file_contains "$WORKFLOW" 'xvfb-run --auto-servernum' 'Xvfb launch smoke is present'
assert_file_contains "$WORKFLOW" 'xdotool search --name' 'window-presence assertion is present'
assert_file_contains "$WORKFLOW" 'import -window' 'screenshot capture is present'
assert_file_contains "$WORKFLOW" 'DohFlow vault screen detected' 'vault-screen log marker is present'
assert_file_contains "$WORKFLOW" 'actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02' 'artifact upload is pinned'

case_start "Linux prerequisites and platform probes are retained"
for package in \
  'libwebkit2gtk-4.1-dev' 'libgtk-3-dev' 'libayatana-appindicator3-dev' \
  'librsvg2-dev' 'libxdo-dev' 'libssl-dev' 'patchelf' 'xvfb' 'xdotool' 'imagemagick'; do
  assert_file_contains "$WORKFLOW" "$package" "apt package: $package"
done
assert_file_contains "$WORKFLOW" 'PRAGMA cipher_version=' 'SQLCipher version probe is present'
assert_file_contains "$WORKFLOW" 'bundled-sqlcipher-vendored-openssl' 'rusqlite feature probe is present'
assert_file_contains "$WORKFLOW" 'Argon2id profile=' 'Argon2 timing probe is present'
assert_file_contains "$WORKFLOW" 'acl_coverage' 'capability regression suite is present'
assert_file_contains "$WORKFLOW" 'app.security.csp' 'CSP regression assertion is present'
assert_file_contains "$WORKFLOW" 'No Linux endpoint is configured' 'updater no-Linux-channel behavior is recorded'

case_start "research output names every acceptance leg"
for needle in \
  'personal-cfo-xcrsk' 'Run URL:' 'Artifact:' 'PRAGMA cipher_version' \
  'updater' 'Argon2id' 'arm64 VM leg' 'GO' 'NO-GO' 'DEFER' \
  'Ubuntu 22.04 LTS' 'AppImage-only' 'minisign'; do
  assert_file_contains "$DOC" "$needle" "research doc mentions $needle"
done

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
