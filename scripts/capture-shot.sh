#!/usr/bin/env bash
#
# Capture one DohFlow product screenshot (personal-cfo-n76x.13). Handles the
# window mechanics only — sizing/positioning the frontmost DohFlow window
# toward the convention's 1440x900 and shelling out to `screencapture` with
# the right flags (no cursor) — so a capture session is "navigate by hand,
# then run one command" per row of dohflow-site/docs/screenshot-shot-list.md,
# which is the authoritative list of valid <slug> values and what each one
# should show on screen before you run this.
#
# Convention (fixture, theme, naming, derived sizes): see
# dohflow-site/docs/screenshots.md. This script does not know about the
# Polish Demo vault, themes, or the app's UI at all — reseed the vault,
# unlock it, navigate to the right surface, and set the right theme (once
# personal-cfo-17u1 ships dark mode) yourself before each shot; this script
# only captures whatever is already on screen.
#
# WHY -R, NOT -l <window-id>: the obvious approach — read the window's id via
# System Events and pass it to `screencapture -l` — does not work against
# DohFlow's window: both `id of window 1` and the `AXWindowNumber` attribute
# return "Can't get id/attribute ... (-1728)" for this app's window (verified
# 2026-09-09; not an Accessibility-permission problem — position/size on the
# same window work fine). So this script instead reads back the window's
# ACTUAL on-screen rectangle after positioning it (see WHY READ BACK below)
# and captures that exact region with `screencapture -R`, which takes
# points and renders at the display's native backing resolution (verified:
# a 1440x879-point region produced a 2880x1758-pixel PNG on this machine's
# 2x Retina display) — no window id needed at all.
#
# WHY READ BACK THE SIZE INSTEAD OF ASSUMING 1440x900: on a screen where the
# Dock is visible (not auto-hidden), macOS silently clamps how tall a window
# can grow — this script does NOT touch Dock/menu-bar settings itself (a
# system-settings change), so a full 900pt-tall window may not fit. Rather
# than hardcode a shorter height that would only be correct for one specific
# screen's Dock size, the script asks System Events to position+size the
# window, then reads back whatever ACTUALLY landed and captures exactly
# that — self-adapting per machine. A warning prints when the result is
# shorter than requested, naming the likely cause.
#
# Requires: DohFlow.app running with at least one window, and Accessibility
# access granted to whatever runs this script (Terminal, iTerm, etc.) —
# System Settings > Privacy & Security > Accessibility. Without that grant,
# the resize/position step fails with an actionable error (see below); a
# one-time approval dialog appears on first run instead if the grant has
# never been decided either way.
#
# Usage:
#   ./scripts/capture-shot.sh <slug>            # capture, e.g. dashboard-light
#   ./scripts/capture-shot.sh --list             # print every valid slug
#   ./scripts/capture-shot.sh --help
#
# Output: apps/desktop/screenshots/raw/<slug>.png (directory created if
# missing). Re-running the same slug overwrites its file — that is
# deliberate (recapture without manual cleanup); `git status` shows you
# what actually changed before you commit.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# CAPTURE_SHOT_OUT_DIR overrides the output directory — testing only, never
# production (see scripts/tests/capture-shot.test.sh). Unset in real use.
out_dir="${CAPTURE_SHOT_OUT_DIR:-$repo_root/apps/desktop/screenshots/raw}"
process_name="DohFlow"
target_width=1440
target_height=900
# Just below the menu bar and away from the left edge — a position near
# (0,0) gets clamped in unpredictable ways on some displays; this offset has
# been verified to position cleanly.
target_x=20
target_y=38

# The seven surfaces x two themes from dohflow-site/docs/screenshot-shot-list.md.
# Kept here too (not sourced from the site repo, a separate checkout that may
# not exist on every machine that runs this script) so --list and validation
# work standalone. If the shot list changes, update both — they are meant to
# describe the same fourteen rows.
valid_slugs=(
  dashboard-light dashboard-dark
  scenarios-light scenarios-dark
  money-inbox-light money-inbox-dark
  accounts-debt-light accounts-debt-dark
  import-light import-dark
  vault-backup-light vault-backup-dark
  settings-about-light settings-about-dark
)

print_help() {
  awk 'NR>1 && /^#/ { sub(/^# ?/, ""); print; next } NR>1 { exit }' "${BASH_SOURCE[0]}"
}

is_valid_slug() {
  local candidate="$1"
  for s in "${valid_slugs[@]}"; do
    [[ "$s" == "$candidate" ]] && return 0
  done
  return 1
}

if [[ $# -eq 0 || "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  print_help
  exit 0
fi

if [[ "${1:-}" == "--list" ]]; then
  printf '%s\n' "${valid_slugs[@]}"
  exit 0
fi

if [[ $# -ne 1 ]]; then
  echo "error: expected exactly one <slug> argument (try --help)" >&2
  exit 2
fi

slug="$1"

if ! is_valid_slug "$slug"; then
  echo "error: unknown slug '$slug'" >&2
  echo "Valid slugs:" >&2
  printf '  %s\n' "${valid_slugs[@]}" >&2
  exit 2
fi

if ! command -v screencapture >/dev/null 2>&1; then
  echo "error: 'screencapture' not found — this script only runs on macOS." >&2
  exit 1
fi

# Bring DohFlow forward, then resize + reposition its window, then read back
# the ACTUAL resulting rectangle (see the header's WHY READ BACK). A failure
# here is almost always the one-time Accessibility grant not yet given to
# whatever runs this script — surface that distinctly rather than a bare
# AppleScript error.
# Wrapped in `if !` deliberately: under `set -e`, a failing command
# substitution inside a plain assignment (`x="$(cmd)"`) exits the script
# immediately, before any code after it runs — including a custom error
# message. Testing it as an `if` condition is one of the constructs `-e`
# recognizes as "the exit status is being handled," so the message below
# actually gets a chance to print.
if ! actual_rect="$(osascript <<EOF
tell application "System Events"
  if not (exists process "$process_name") then
    error "DohFlow is not running — launch it first."
  end if
  tell process "$process_name"
    if (count of windows) is 0 then
      error "DohFlow has no open window."
    end if
    set frontmost to true
    set position of front window to {$target_x, $target_y}
    set size of front window to {$target_width, $target_height}
    set {winX, winY} to position of front window
    set {winW, winH} to size of front window
    return (winX as string) & "," & (winY as string) & "," & (winW as string) & "," & (winH as string)
  end tell
end tell
EOF
)"; then
  echo "error: could not resize/position the DohFlow window via System Events." >&2
  echo "       Most likely cause: Accessibility access has not been granted to" >&2
  echo "       whatever is running this script (Terminal, iTerm, ...). Grant it at" >&2
  echo "       System Settings > Privacy & Security > Accessibility, then re-run." >&2
  exit 1
fi
if [[ -z "$actual_rect" ]]; then
  echo "error: System Events returned no window rectangle." >&2
  exit 1
fi

IFS=',' read -r actual_x actual_y actual_w actual_h <<< "$actual_rect"

if [[ "$actual_w" -ne "$target_width" || "$actual_h" -ne "$target_height" ]]; then
  echo "warning: requested ${target_width}x${target_height}, got ${actual_w}x${actual_h}." >&2
  echo "         Likely cause: a visible Dock and/or menu bar reserves screen space on" >&2
  echo "         this display, and this script does not change system Dock/display" >&2
  echo "         settings itself. Capturing the ACTUAL rectangle rather than failing —" >&2
  echo "         update dohflow-site/docs/screenshots.md's convention numbers if this" >&2
  echo "         becomes the standing size for this machine's captures." >&2
fi

# Give the app a moment to finish laying out at the new size before capture.
sleep 0.3

mkdir -p "$out_dir"
out_path="$out_dir/$slug.png"

# -o: do not capture the cursor image (screencapture's window-shadow flag is
# -l-mode-only; -R is a plain rectangular grab, so there is no separate
# shadow to suppress — the rectangle is exactly the window's own frame).
if ! screencapture -o -R "${actual_x},${actual_y},${actual_w},${actual_h}" "$out_path"; then
  echo "error: screencapture failed for region ${actual_x},${actual_y},${actual_w},${actual_h}." >&2
  exit 1
fi

echo "Captured: $out_path"
echo "Reminder: this only captured what was on screen — confirm it actually"
echo "shows '$slug' (surface + theme) before moving to the next row, and run"
echo "the privacy check in dohflow-site/docs/screenshots.md before committing."
