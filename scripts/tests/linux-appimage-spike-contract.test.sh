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
assert_file_not_contains() {
  if grep -Fq -- "$2" "$1"; then fail "$3 (unexpected '$2' in $1)"; else pass "$3"; fi
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
assert_file_contains "$WORKFLOW" 'for attempt in $(seq 1 10)' 'screenshot capture retries transient X11 failures'
assert_file_contains "$WORKFLOW" 'identify -format '\''%[fx:mean]'\''' 'screenshot mean pixels are measured'
assert_file_contains "$WORKFLOW" 'identify -format '\''%k'\''' 'screenshot color count is measured'
assert_file_contains "$WORKFLOW" 'mean > 0.01 && colors > 1' 'all-black or empty screenshots are rejected'
assert_file_contains "$WORKFLOW" 'rendered=1' 'rendered-pixel safeguard gates the smoke marker'
assert_file_contains "$WORKFLOW" 'Unable to capture a rendered X11 window' 'rendered screenshot failures are explicit'
assert_file_contains "$WORKFLOW" 'if [ "$captured" -ne 1 ] || [ "$rendered" -ne 1 ]; then' 'rendered-pixel guard gates success'
assert_file_contains "$WORKFLOW" 'mktemp -d "${RUNNER_TEMP}/dohflow-linux-data.XXXXXX"' 'launch smoke uses a fresh data directory'
render_guard_line="$(grep -nF 'if [ "$captured" -ne 1 ] || [ "$rendered" -ne 1 ]; then' "$WORKFLOW" | head -n1 | cut -d: -f1)"
marker_line="$(grep -nF 'DohFlow vault screen detected' "$WORKFLOW" | head -n1 | cut -d: -f1)"
if [ -n "$render_guard_line" ] && [ -n "$marker_line" ] && [ "$render_guard_line" -lt "$marker_line" ]; then
  pass 'rendered-pixel guard appears before the vault-screen marker'
else
  fail 'rendered-pixel guard must appear before the vault-screen marker'
fi
assert_file_contains "$WORKFLOW" 'DohFlow vault screen detected' 'vault-screen log marker is present'
assert_file_contains "$WORKFLOW" 'actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02' 'artifact upload is pinned'
assert_file_contains "$WORKFLOW" 'printf '\''%s\n'\'' "${packages[@]}" > "$artifact_dir/linux-spike-apt-packages.txt"' 'apt package record is written into the artifact directory'
assert_file_contains "$WORKFLOW" 'tee "$artifact_dir/linux-spike-versions.txt"' 'runner-version record is written into the artifact directory'
assert_file_contains "$WORKFLOW" 'path: ${{ runner.temp }}/linux-appimage-spike/' 'artifact upload path includes runner evidence records'
assert_file_not_contains "$WORKFLOW" '$RUNNER_TEMP/linux-spike-apt-packages.txt' 'apt package record is not stranded outside the artifact directory'
assert_file_not_contains "$WORKFLOW" '$RUNNER_TEMP/linux-spike-versions.txt' 'runner-version record is not stranded outside the artifact directory'

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
  'Ubuntu 22.04 LTS' 'AppImage-only' 'minisign' '35550000687' \
  '10618516214' '493.19' '2712.39' '4.5.7 community' 'Ubuntu 24.04.5 LTS' \
  'rendered_pixels=0.952062 colors=1449' 'fresh temporary `PCFO_DATA_DIR`'; do
  assert_file_contains "$DOC" "$needle" "research doc mentions $needle"
done

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
