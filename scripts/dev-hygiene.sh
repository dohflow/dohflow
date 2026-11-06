#!/usr/bin/env bash
#
# dev-hygiene.sh — stop build caches and stale worktrees from filling the build
# machine (personal-cfo-0rein.6, personal-cfo-0rein.12, personal-cfo-0rein.13).
#
# READ-ONLY BY DEFAULT. `--report` (the default) changes nothing. `--clean` is
# explicit. Without --scratch it does exactly two things, nothing else:
#
#   1. `cargo clean` for each Cargo target directory of a registered worktree
#      that has been idle for at least --idle-days days (default 3), has no
#      uncommitted changes, has no commits that exist on no remote, and has no
#      running cargo/rustc process; and
#   2. `git worktree prune` for registrations whose directory is already gone.
#
# `--clean --scratch` (Tier 2, owner-authorized 2026-11-04: "File with both
# tiers") adds, for the per-session scratch folders of the DohFlow lane checkouts
# under /private/tmp/claude-<uid>/<project slug>/<session uuid>/:
#
#   3. `cargo clean` for a cargo target directory inside a clean REGISTERED
#      linked worktree; and
#   4. a plain (never forced) `git worktree remove` of that worktree, only when
#      its sole ignored entries are target/ and node_modules/.
#
# Both act only when the worktree's pull request is MERGED or CLOSED as GitHub
# says (an unknown state, a missing gh or no network counts as unknown, never as
# merged), the whole session folder has been idle for at least --idle-days days,
# no cargo/rustc process works in it, and nothing is uncommitted or on no remote.
#
# It NEVER runs `rm`, never removes a branch, never touches an unregistered
# copy (a clone, or a directory with a stale .git file), another project's
# folder, a scratch folder or worktree it is run from, or a dirty or unpushed
# worktree (AGENTS.md section 1: non-destructive by default). Other projects'
# folders under the scratch root are measured as one size and never opened.
#
# `--cap-check` (personal-cfo-0rein.13) is read-only too: for the worktree it is
# run from, and no other, it lists each cargo target directory with its size, the
# total against the cap, and whether a cargo/rustc process works in it. It never
# cleans anything. Exit status: 0 under the cap; 1 over the cap and quiet; 2 over
# the cap and a build is running (or could not be ruled out). Usage errors exit 64.
#
# Usage:
#   scripts/dev-hygiene.sh [--report] [--json] [--no-pr] [--target-cap-gib N]
#   scripts/dev-hygiene.sh --clean [--scratch] [--idle-days N] [--yes] [--no-pr]
#   scripts/dev-hygiene.sh --cap-check [--target-cap-gib N]
#   scripts/dev-hygiene.sh --help
#
# Options:
#   --report          Print the report (default). Read-only.
#   --json            Print the report as JSON instead of text. Read-only.
#   --clean           Run the actions above after showing the plan and asking
#                     for confirmation.
#   --scratch         With --clean: also run the Tier 2 scratch actions (3, 4).
#   --yes             Skip the confirmation prompt (with --clean only).
#   --idle-days N     Idle threshold in days for --clean (default 3).
#   --no-pr           Do not ask GitHub for pull request states: every scratch
#                     worktree is then "unknown" and Tier 2 does nothing.
#   --target-cap-gib N  The cap on one worktree's cargo targets, every workspace
#                     counted together (default 16). The report flags a worktree
#                     over it; --cap-check exits non-zero over it.
#   --cap-check       Check the worktree this is run from against the cap. Never
#                     cleans. See above for the exit status.
#   --scratch-dir D   Session scratch directory (default /private/tmp/claude-<uid>).
#   --repo DIR        Repository to inspect (default: this script's repository).
#
# "Last activity" of a worktree is the newest modification time of its git
# index, HEAD and HEAD reflog, and of the directories (two levels deep) and top
# files of its cargo target directories. For a scratch session folder it is the
# newest of that, the folder's own entries two levels deep, and every checkout
# and target dir inside it. It is an approximation: a worktree where tests were
# only re-run, with nothing rebuilt, can look older than it is. That is why
# --clean also checks for running cargo/rustc processes, and why the worst
# outcome of a wrong guess is a rebuild, never lost work.
#
# Pull request state comes from `gh` (pr list by branch; the commit's pulls by
# SHA for a detached HEAD), against the checkout's own origin. A squash merge
# counts as merged through the PR, never through ancestry.
#
# Environment: DEV_HYGIENE_AFTER_CONFIRM_HOOK, DEV_HYGIENE_BIG_KIB and
# DEV_HYGIENE_CAP_KIB, honored only when DEV_HYGIENE_TESTING=1 (tests only): the
# first runs between the confirmation and the apply step; the second replaces the
# 20 GiB session flag; the third replaces the target cap with a size in KiB.
# DEV_HYGIENE_OS overrides the operating system name (tests only, with
# DEV_HYGIENE_TESTING=1), so the macOS-only snapshot line can be tested anywhere.
# DEV_HYGIENE_PROCESS_NAMES overrides the process names treated as "a build is
# running" (default "cargo rustc rust-analyzer"); the tests use it.
#
# Bash 3.2 compatible (macOS /bin/bash). No external dependencies beyond git,
# coreutils, awk, sed, and (optionally) cargo, pgrep and lsof.

set -uo pipefail

# Never let a read-only git command refresh (and so re-timestamp) the index.
export GIT_OPTIONAL_LOCKS=0

MODE=report
FORMAT=text
ASSUME_YES=0
IDLE_DAYS=3
SCRATCH_CLEAN=0
NO_PR=0
CAP_CHECK=0
TARGET_CAP_GIB=16
SCRATCH_DIR="/private/tmp/claude-$(id -u)"
REPO=""

usage() {
  sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
}

# Usage errors exit 64, so that exit status 2 keeps its one meaning under --cap-check.
die() { echo "dev-hygiene: $*" >&2; exit 64; }

while [ $# -gt 0 ]; do
  case "$1" in
    --report) MODE=report ;;
    --json) FORMAT=json ;;
    --clean) MODE=clean ;;
    --yes) ASSUME_YES=1 ;;
    --scratch) SCRATCH_CLEAN=1 ;;
    --no-pr) NO_PR=1 ;;
    --cap-check) CAP_CHECK=1 ;;
    --target-cap-gib)
      [ $# -ge 2 ] || die "--target-cap-gib needs a number"
      TARGET_CAP_GIB="$2"; shift ;;
    --idle-days)
      [ $# -ge 2 ] || die "--idle-days needs a number"
      IDLE_DAYS="$2"; shift ;;
    --scratch-dir)
      [ $# -ge 2 ] || die "--scratch-dir needs a path"
      SCRATCH_DIR="$2"; shift ;;
    --repo)
      [ $# -ge 2 ] || die "--repo needs a path"
      REPO="$2"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
  shift
done

case "$IDLE_DAYS" in
  ''|*[!0-9]*) die "--idle-days must be a whole number of days, got '$IDLE_DAYS'" ;;
esac
case "$TARGET_CAP_GIB" in
  ''|*[!0-9]*|0|0[0-9]*) die "--target-cap-gib must be a whole number of GiB, 1 or more, got '$TARGET_CAP_GIB'" ;;
esac
if [ "$CAP_CHECK" = 1 ] && { [ "$MODE" = clean ] || [ "$FORMAT" = json ] || [ "$ASSUME_YES" = 1 ] || [ "$SCRATCH_CLEAN" = 1 ]; }; then
  die "--cap-check is read-only and stands alone: it cannot be combined with --clean, --json, --yes or --scratch"
fi
if [ "$ASSUME_YES" = 1 ] && [ "$MODE" != clean ]; then
  die "--yes only makes sense with --clean"
fi
if [ "$SCRATCH_CLEAN" = 1 ] && [ "$MODE" != clean ]; then
  die "--scratch only makes sense with --clean"
fi
if [ "$FORMAT" = json ] && [ "$MODE" = clean ]; then
  die "--json is a report format; it cannot be combined with --clean"
fi

if [ -z "$REPO" ]; then
  REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
fi
git -C "$REPO" rev-parse --git-dir >/dev/null 2>&1 || die "not a git repository: $REPO"

# git and lsof report physical paths, so compare against the physical scratch root.
if [ -d "$SCRATCH_DIR" ]; then SCRATCH_DIR="$(cd "$SCRATCH_DIR" && pwd -P)"; fi

NOW="$(date +%s)"
TAB="$(printf '\t')"
US="$(printf '\037')"   # scan-record field separator: not whitespace, so empty fields survive `read`
NL="
"

# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------

OS="$(uname)"
if [ "${DEV_HYGIENE_TESTING:-}" = 1 ] && [ -n "${DEV_HYGIENE_OS:-}" ]; then OS="$DEV_HYGIENE_OS"; fi
CAP_KIB=$(( TARGET_CAP_GIB * 1024 * 1024 ))
if [ "${DEV_HYGIENE_TESTING:-}" = 1 ] && [ -n "${DEV_HYGIENE_CAP_KIB:-}" ]; then CAP_KIB="$DEV_HYGIENE_CAP_KIB"; fi

mtime() {
  if [ "$OS" = Darwin ]; then
    stat -f %m "$1" 2>/dev/null
  else
    stat -c %Y "$1" 2>/dev/null
  fi
}

du_kib() {
  du -sk "$1" 2>/dev/null | awk '{print $1}'
}

human() {
  awk -v k="${1:-0}" 'BEGIN {
    split("KiB MiB GiB TiB", u, " "); i = 1; v = k + 0
    while (v >= 1024 && i < 4) { v /= 1024; i++ }
    printf "%.1f %s", v, u[i]
  }'
}

json_str() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | awk 'BEGIN { ORS = "" } { print }'
}

# ---------------------------------------------------------------------------
# Per-worktree inspection. Sets the W_* globals.
# ---------------------------------------------------------------------------

W_PATH="" W_BRANCH="" W_HEAD="" W_MISSING=0 W_TARGETS="" W_TARGET_KIB=0
W_ACTIVITY="" W_IDLE_DAYS="" W_DIRTY="" W_UNPUSHED="" W_MERGED="" W_GONE=0
W_BUSY=0 W_CURRENT=0 W_SKIP="" W_ELIGIBLE=0 W_TSIZES="" W_OVER=0

# Cargo target directories (a directory named `target` that holds CACHEDIR.TAG)
# up to four levels down, never descending into node_modules, .git or a target,
# and never inside a NESTED checkout: a directory below the worktree that has
# its own `.git` (a file for a linked worktree, a directory for a clone) belongs
# to that checkout, which is inspected, or skipped, on its own.
find_targets() {
  local root="$1" nested n d skip
  nested="$(find "$root" -mindepth 2 -maxdepth 4 -name node_modules -prune -o \
    -name .git -print 2>/dev/null | sed 's|/\.git$||')"
  find "$root" -maxdepth 4 \( -name node_modules -o -name .git \) -prune -o \
    -type d -name target -print -prune 2>/dev/null |
    while IFS= read -r d; do
      [ -f "$d/CACHEDIR.TAG" ] || continue
      skip=0
      while IFS= read -r n; do
        [ -n "$n" ] || continue
        case "$d" in "$n"/*) skip=1; break ;; esac
      done <<NESTED
$nested
NESTED
      [ "$skip" = 0 ] && printf '%s\n' "$d"
    done
}

newest_mtime() {
  # newest_mtime FILE_OR_DIR...: the largest mtime among those that exist (one stat).
  local f m best="" out
  local -a ex
  ex=()
  for f in "$@"; do
    [ -e "$f" ] && ex[${#ex[@]}]="$f"
  done
  [ "${#ex[@]}" -gt 0 ] || return 0
  if [ "$OS" = Darwin ]; then out="$(stat -f %m "${ex[@]}" 2>/dev/null)"; else out="$(stat -c %Y "${ex[@]}" 2>/dev/null)"; fi
  while IFS= read -r m; do
    [ -n "$m" ] || continue
    if [ -z "$best" ] || [ "$m" -gt "$best" ]; then best="$m"; fi
  done <<MTIMES
$out
MTIMES
  printf '%s' "$best"
}

targets_activity() {
  # targets_activity LIST: newest mtime over a newline-separated list of cargo
  # target directories (each one's own mtime, its directories two levels deep
  # and its top files), or empty.
  local best="" m t d f
  while IFS= read -r t; do
    [ -n "$t" ] || continue
    m="$(newest_mtime "$t")"
    [ -n "$m" ] && { [ -z "$best" ] || [ "$m" -gt "$best" ]; } && best="$m"
    while IFS= read -r d; do
      m="$(mtime "$d")"
      [ -n "$m" ] && { [ -z "$best" ] || [ "$m" -gt "$best" ]; } && best="$m"
    done < <(find "$t" -maxdepth 2 -type d 2>/dev/null)
    while IFS= read -r f; do
      m="$(mtime "$f")"
      [ -n "$m" ] && { [ -z "$best" ] || [ "$m" -gt "$best" ]; } && best="$m"
    done < <(find "$t" -maxdepth 1 -type f 2>/dev/null)
  done <<EOF
$1
EOF
  printf '%s' "$best"
}

worktree_activity() {
  # worktree_activity PATH: newest activity epoch, or empty if unknowable.
  local wt="$1" gitdir best="" m
  gitdir="$(git -C "$wt" rev-parse --absolute-git-dir 2>/dev/null)"
  if [ -n "$gitdir" ]; then
    best="$(newest_mtime "$gitdir/index" "$gitdir/HEAD" "$gitdir/logs/HEAD")"
  fi
  m="$(targets_activity "$W_TARGETS")"
  [ -n "$m" ] && { [ -z "$best" ] || [ "$m" -gt "$best" ]; } && best="$m"
  printf '%s' "$best"
}

# The running cargo/rustc processes and where each works, read once per run (and
# again after the confirmation, which sets PROCS_LOADED=0): one lsof per process
# instead of one per process per worktree. Lines of PROCS are cwd<US>command.
PROCS_LOADED=0 PROCS="" PROCS_UNKNOWN=0

load_procs() {
  local name pids pid cwd cmd
  PROCS_LOADED=1 PROCS="" PROCS_UNKNOWN=0
  # Without pgrep nothing can be ruled out: assume busy (the safe direction).
  command -v pgrep >/dev/null 2>&1 || { PROCS_UNKNOWN=1; return; }
  pids=""
  for name in ${DEV_HYGIENE_PROCESS_NAMES:-cargo rustc rust-analyzer}; do
    pids="$pids $(pgrep -x "$name" 2>/dev/null)"
  done
  pids="$(printf '%s' "$pids" | tr -s ' \n' ' ')"
  for pid in $pids; do
    cmd="$(ps -p "$pid" -o command= 2>/dev/null)"
    if command -v lsof >/dev/null 2>&1; then
      cwd="$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -n 1)"
    elif [ -e "/proc/$pid/cwd" ]; then
      cwd="$(readlink "/proc/$pid/cwd" 2>/dev/null)"
    else
      cwd=""
    fi
    # Could not read where it works (not ours, or it just exited): assume busy.
    [ -n "$cwd" ] || PROCS_UNKNOWN=1
    PROCS="${PROCS}${cwd}${US}${cmd}${NL}"
  done
}

cargo_busy() {
  # cargo_busy PATH: 0 if a cargo/rustc process works in or on PATH. If this
  # cannot be determined, answer "busy" (the safe direction). Both the path as
  # registered and its physical form are compared, because lsof reports the
  # physical one (on macOS /var is /private/var).
  local wt="$1" phys cwd cmd rec
  phys="$(cd "$wt" 2>/dev/null && pwd -P)"
  [ -n "$phys" ] || phys="$wt"
  [ "$PROCS_LOADED" = 1 ] || load_procs
  [ "$PROCS_UNKNOWN" = 1 ] && return 0
  [ -n "$PROCS" ] || return 1
  while IFS= read -r rec; do
    [ -n "$rec" ] || continue
    cwd="${rec%%"$US"*}"; cmd="${rec#*"$US"}"
    case "$cmd" in *"$wt/"*|*"$phys/"*) return 0 ;; esac
    case "$cwd" in "$wt"|"$wt"/*|"$phys"|"$phys"/*) return 0 ;; esac
  done <<RECORDS
$PROCS
RECORDS
  return 1
}

measure_targets() {
  # Sizes every directory in W_TARGETS on its own: W_TSIZES (one "kib<US>path"
  # per line), their sum W_TARGET_KIB, and W_OVER when the sum passes the cap.
  local t k
  W_TARGET_KIB=0 W_TSIZES="" W_OVER=0
  while IFS= read -r t; do
    [ -n "$t" ] || continue
    k="$(du_kib "$t")"
    W_TARGET_KIB=$(( W_TARGET_KIB + ${k:-0} ))
    W_TSIZES="${W_TSIZES}${k:-0}${US}${t}${NL}"
  done <<EOF
$W_TARGETS
EOF
  [ "$W_TARGET_KIB" -gt "$CAP_KIB" ] && W_OVER=1
  return 0
}

inspect_worktree() {
  # inspect_worktree PATH BRANCH HEAD MISSING
  W_PATH="$1" W_BRANCH="$2" W_HEAD="$3" W_MISSING="$4"
  W_TARGETS="" W_TARGET_KIB=0 W_ACTIVITY="" W_IDLE_DAYS="" W_DIRTY="" W_UNPUSHED=""
  W_MERGED="" W_GONE=0 W_BUSY=0 W_CURRENT=0 W_SKIP="" W_ELIGIBLE=0 W_TSIZES="" W_OVER=0

  if [ "$W_MISSING" = 1 ]; then
    W_SKIP="directory is gone (git worktree prune handles the registration)"
    return
  fi

  local here
  here="$(git -C "$PWD" rev-parse --show-toplevel 2>/dev/null || true)"
  [ -n "$here" ] && here="$(cd "$here" 2>/dev/null && pwd -P)"
  if [ "$here" = "$(cd "$W_PATH" 2>/dev/null && pwd -P)" ]; then W_CURRENT=1; fi

  W_TARGETS="$(find_targets "$W_PATH")"
  measure_targets

  W_ACTIVITY="$(worktree_activity "$W_PATH")"
  if [ -n "$W_ACTIVITY" ]; then
    W_IDLE_DAYS=$(( (NOW - W_ACTIVITY) / 86400 ))
    [ "$W_IDLE_DAYS" -lt 0 ] && W_IDLE_DAYS=0
  fi

  local porcelain
  if porcelain="$(git -C "$W_PATH" status --porcelain 2>/dev/null)"; then
    if [ -n "$porcelain" ]; then W_DIRTY=1; else W_DIRTY=0; fi
  else
    W_DIRTY=""   # unknown: treated as unsafe
  fi

  if W_UNPUSHED="$(git -C "$W_PATH" rev-list --count HEAD --not --remotes 2>/dev/null)"; then
    :
  else
    W_UNPUSHED=""   # unknown: treated as unsafe
  fi

  if [ -n "$W_BRANCH" ]; then
    local track
    track="$(git -C "$REPO" for-each-ref --format='%(upstream:track)' "refs/heads/$W_BRANCH" 2>/dev/null)"
    case "$track" in *gone*) W_GONE=1 ;; esac
    if git -C "$REPO" rev-parse --verify -q origin/main >/dev/null 2>&1; then
      if git -C "$REPO" merge-base --is-ancestor "refs/heads/$W_BRANCH" origin/main 2>/dev/null; then
        W_MERGED=1
      else
        W_MERGED=0
      fi
    fi
  fi

  if cargo_busy "$W_PATH"; then W_BUSY=1; fi

  # Decide --clean eligibility, recording the first reason it is not eligible.
  if [ -z "$W_TARGETS" ]; then
    W_SKIP="no cargo target directory"
  elif [ "$W_CURRENT" = 1 ]; then
    W_SKIP="this is the worktree the script runs from"
  elif [ -z "$W_DIRTY" ] || [ "$W_DIRTY" = 1 ]; then
    W_SKIP="uncommitted changes (or status unreadable)"
  elif [ -z "$W_UNPUSHED" ] || [ "$W_UNPUSHED" != 0 ]; then
    W_SKIP="commits that exist on no remote (or unreadable)"
  elif [ "$W_BUSY" = 1 ]; then
    W_SKIP="a cargo/rustc process is running in it (or could not be checked)"
  elif [ -z "$W_IDLE_DAYS" ]; then
    W_SKIP="last activity unknown"
  elif [ "$W_IDLE_DAYS" -lt "$IDLE_DAYS" ]; then
    W_SKIP="active ${W_IDLE_DAYS} day(s) ago, threshold is ${IDLE_DAYS}"
  else
    W_ELIGIBLE=1
  fi
}

# Calls $1 once per registered worktree, after inspecting it. Skips bare
# entries. Parses `git worktree list --porcelain` (blank-line separated records).
for_each_worktree() {
  local cb="$1" line path="" branch="" head="" bare=0 missing=0 have=0 listing
  # Read the whole listing first so commands run per worktree can never consume
  # the loop's input.
  listing="$(git -C "$REPO" worktree list --porcelain 2>/dev/null)"
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      "worktree "*)
        if [ "$have" = 1 ] && [ "$bare" = 0 ]; then
          inspect_worktree "$path" "$branch" "$head" "$missing"; "$cb"
        fi
        path="${line#worktree }"; branch=""; head=""; bare=0; missing=0; have=1
        [ -d "$path" ] || missing=1 ;;
      "HEAD "*) head="${line#HEAD }" ;;
      "branch refs/heads/"*) branch="${line#branch refs/heads/}" ;;
      "bare") bare=1 ;;
      "prunable"*) missing=1 ;;
    esac
  done <<EOF
$listing
EOF
  if [ "$have" = 1 ] && [ "$bare" = 0 ]; then
    inspect_worktree "$path" "$branch" "$head" "$missing"; "$cb"
  fi
}

# ---------------------------------------------------------------------------
# Machine-level facts
# ---------------------------------------------------------------------------

disk_total_kib="" disk_avail_kib="" disk_pct_free=""
read_disk() {
  local line
  line="$(df -kP "$REPO" 2>/dev/null | awk 'NR == 2 { print $2, $4 }')"
  disk_total_kib="${line%% *}"
  disk_avail_kib="${line##* }"
  if [ -n "$disk_total_kib" ] && [ "$disk_total_kib" -gt 0 ] 2>/dev/null; then
    disk_pct_free="$(awk -v a="$disk_avail_kib" -v t="$disk_total_kib" 'BEGIN { printf "%.1f", a * 100 / t }')"
  fi
}

scratch_kib=""

# macOS Time Machine local snapshots (personal-cfo-0rein.13): they keep deleted
# build output alive, so freed space may not return until the owner thins them.
# Read-only (`tmutil listlocalsnapshots`), skipped silently where tmutil does not
# exist. This script never thins a snapshot and never runs sudo.
LOCAL_SNAPSHOTS=""
read_snapshots() {
  local out n
  LOCAL_SNAPSHOTS=""
  [ "$OS" = Darwin ] || return 0
  command -v tmutil >/dev/null 2>&1 || return 0
  out="$(tmutil listlocalsnapshots / 2>/dev/null </dev/null)" || return 0
  n="$(printf '%s\n' "$out" | grep -c '^com\.apple\.TimeMachine\.' || true)"
  LOCAL_SNAPSHOTS="${n:-0}"
}

snapshots_delay() {
  # True when snapshots exist and free space is under 15%.
  [ -n "$LOCAL_SNAPSHOTS" ] && [ "$LOCAL_SNAPSHOTS" -gt 0 ] && low_disk
}

low_disk() {
  [ -n "$disk_pct_free" ] && awk -v p="$disk_pct_free" 'BEGIN { exit !(p < 15) }'
}

# ---------------------------------------------------------------------------
# Session scratch (personal-cfo-0rein.12)
#
# Layout: $SCRATCH_DIR/<project slug>/<session uuid>/. DohFlow has one slug per
# lane checkout; the root also holds other projects' folders, which are measured
# as one size and never opened. scan_scratch walks it ONCE, read-only, into SCAN:
# one record per line, fields joined by $US.
#
#   S slug uuid path kib activity idle_days busy current   a session folder
#   C path kind branch head dirty unpushed pr_state pr_number ignored_ok common
#                                                          a checkout inside it
#   T path kib owner pr_state                              a cargo target dir
#   F slug kib                                             another project
#
# kind is "registered" (a linked worktree git itself lists) or "unregistered"
# (a clone, a copy, a stale .git file). Within a session every C comes before
# every T. Cost: one `du -k -d` walk per session folder (sizes of its target
# dirs come from the same walk), `git status`/`rev-list` per checkout, and one
# gh call per checkout.
# ---------------------------------------------------------------------------

SCAN="" PRCACHE=""
BIG_KIB=$((20 * 1024 * 1024))
if [ "${DEV_HYGIENE_TESTING:-}" = 1 ] && [ -n "${DEV_HYGIENE_BIG_KIB:-}" ]; then BIG_KIB="$DEV_HYGIENE_BIG_KIB"; fi

is_dohflow_slug() {
  case "$1" in *-dohflow|*-dohflow-*) return 0 ;; esac
  return 1
}

find_newest() {
  # find_newest FIND-ARGS...: the newest mtime among the paths find prints.
  if [ "$OS" = Darwin ]; then
    find "$@" -exec stat -f %m {} + 2>/dev/null
  else
    find "$@" -exec stat -c %Y {} + 2>/dev/null
  fi | sort -n | tail -n 1
}

github_slug() {
  # github_slug DIR: OWNER/NAME of the checkout's origin on github.com, or empty.
  local url
  url="$(git -C "$1" config --get remote.origin.url 2>/dev/null)"
  url="${url%.git}"
  case "$url" in
    https://github.com/*/*) printf '%s' "${url#https://github.com/}" ;;
    git@github.com:*/*) printf '%s' "${url#git@github.com:}" ;;
    ssh://git@github.com/*/*) printf '%s' "${url#ssh://git@github.com/}" ;;
  esac
}

# pr_lookup DIR BRANCH HEAD: sets PR_STATE (merged|closed|open|unknown) and
# PR_NUM. Unknown covers everything that is not a clear answer: no gh, --no-pr,
# no network, an origin that is not on github.com, no pull request for the
# branch, or a checkout that is not the work of the pull requests found. A squash
# merge is "merged" because GitHub says so, never because of ancestry.
pr_lookup() {
  local dir="$1" branch="$2" head="$3" slug rows key cached num state ph merged_at mc
  PR_STATE=unknown PR_NUM=""
  [ "$NO_PR" = 1 ] && return 0
  [ -n "$head" ] || return 0
  command -v gh >/dev/null 2>&1 || return 0
  slug="$(github_slug "$dir")"
  [ -n "$slug" ] || return 0
  key="$slug$US${branch:-@$head}"
  cached="$(printf '%s' "$PRCACHE" | awk -F"$US" -v k="$key" '$1 FS $2 == k { print $3 FS $4; exit }')"
  if [ -n "$cached" ]; then PR_STATE="${cached%%"$US"*}"; PR_NUM="${cached#*"$US"}"; return 0; fi

  local best_state="" best_num="" open_num="" any_merged="" any_closed=""
  if [ -n "$branch" ]; then
    rows="$(gh pr list -R "$slug" --head "$branch" --state all --limit 30 \
      --json number,state,headRefOid --jq '.[] | [.number,.state,.headRefOid] | @tsv' </dev/null 2>/dev/null | tr '\t' "$US")" || rows=""
    while IFS="$US" read -r num state ph; do
      [ -n "$num" ] || continue
      case "$state" in
        OPEN) open_num="$num"; continue ;;
        MERGED|CLOSED) ;;
        *) continue ;;
      esac
      # The checkout must BE this pull request's work: at its head, or at an
      # older commit of it (an ancestor of the head).
      if [ "$ph" = "$head" ] || git -C "$dir" merge-base --is-ancestor "$head" "$ph" 2>/dev/null; then
        if [ "$state" = MERGED ]; then any_merged="$num"; else any_closed="$num"; fi
      fi
    done <<ROWS
$rows
ROWS
  else
    rows="$(gh api "repos/$slug/commits/$head/pulls" \
      --jq '.[] | [.number,.state,(.merged_at // ""),(.merge_commit_sha // "")] | @tsv' </dev/null 2>/dev/null | tr '\t' "$US")" || rows=""
    while IFS="$US" read -r num state merged_at mc; do
      [ -n "$num" ] || continue
      # A commit that is itself a pull request's landing commit sits on the base
      # branch; it is not that pull request's work.
      [ "$mc" = "$head" ] && continue
      case "$state" in
        open) open_num="$num" ;;
        closed) if [ -n "$merged_at" ]; then any_merged="$num"; else any_closed="$num"; fi ;;
      esac
    done <<ROWS
$rows
ROWS
  fi
  if [ -n "$open_num" ]; then best_state=open best_num="$open_num"
  elif [ -n "$any_merged" ]; then best_state=merged best_num="$any_merged"
  elif [ -n "$any_closed" ]; then best_state=closed best_num="$any_closed"
  fi
  if [ -n "$best_state" ]; then
    PR_STATE="$best_state" PR_NUM="$best_num"
    PRCACHE="${PRCACHE}${key}${US}${PR_STATE}${US}${PR_NUM}${NL}"
  fi
}

scan_checkout() {
  # scan_checkout DIR: appends one C record to CK_BUF and raises CK_ACT to the
  # mtime of the checkout's git index, HEAD and reflog.
  local d="$1" kind=unregistered branch="" head="" dirty="" unpushed="" gitfile="" gitdir="" back="" ign ignok=1 common=""
  local st line admin m
  if [ -f "$d/.git" ]; then
    read -r gitfile < "$d/.git" 2>/dev/null || gitfile=""
    gitfile="${gitfile#gitdir: }"
    case "$gitfile" in /*) gitdir="$gitfile" ;; ?*) gitdir="$d/$gitfile" ;; esac
    # Registered = git's own back-pointer in the worktree's admin directory
    # points at this checkout's .git file, inside a repository's worktrees/.
    if [ -n "$gitdir" ] && [ -f "$gitdir/gitdir" ]; then
      read -r back < "$gitdir/gitdir" 2>/dev/null || back=""
      case "$gitdir" in
        */worktrees/*)
          if [ "$back" = "$d/.git" ] || [ "$(cd "$(dirname "$back")" 2>/dev/null && pwd -P)/.git" = "$d/.git" ]; then
            kind=registered
          fi ;;
      esac
    fi
  fi
  # One status call: branch, HEAD, changes, and ignored entries.
  if st="$(git -C "$d" status --porcelain=v2 --branch --ignored 2>/dev/null </dev/null)"; then
    dirty=0
    while IFS= read -r line; do
      case "$line" in
        "# branch.oid "*) head="${line#\# branch.oid }"; [ "$head" = "(initial)" ] && head="" ;;
        "# branch.head "*) branch="${line#\# branch.head }"; [ "$branch" = "(detached)" ] && branch="" ;;
        "# "*) ;;
        "! "*)
          ign="${line#! }"; ign="${ign%/}"
          case "${ign##*/}" in target|node_modules) ;; *) ignok=0 ;; esac ;;
        "") ;;
        *) dirty=1 ;;
      esac
    done <<STATUS
$st
STATUS
  fi
  if [ -n "$head" ]; then
    unpushed="$(git -C "$d" rev-list --count HEAD --not --remotes 2>/dev/null </dev/null)" || unpushed=""
  fi
  # admin directory and, for a registered worktree, its repository's git dir
  admin="" common=""
  {
    read -r admin
    read -r common
  } <<GITDIRS
$(git -C "$d" rev-parse --absolute-git-dir --path-format=absolute --git-common-dir 2>/dev/null </dev/null)
GITDIRS
  [ "$kind" = registered ] || common=""
  if [ -n "$admin" ]; then
    m="$(newest_mtime "$admin/index" "$admin/HEAD" "$admin/logs/HEAD")"
    [ -n "$m" ] && { [ -z "$CK_ACT" ] || [ "$m" -gt "$CK_ACT" ]; } && CK_ACT="$m"
  fi
  pr_lookup "$d" "$branch" "$head"
  CK_BUF="${CK_BUF}C${US}${d}${US}${kind}${US}${branch}${US}${head}${US}${dirty}${US}${unpushed}${US}${PR_STATE}${US}${PR_NUM}${US}${ignok}${US}${common}${NL}"
}

scan_session() {
  # scan_session DIR SLUG: appends the S, C and T records of one session folder.
  local s="$1" slug="$2" uuid="${1##*/}" du_out kib act m idle busy=0 cur=0 here
  local gits g t tkib owner best ck targets tpr
  du_out="$(du -k -d 7 "$s" 2>/dev/null)"
  kib="$(printf '%s\n' "$du_out" | awk -F'\t' -v p="$s" '$2 == p { print $1; exit }')"

  gits="$(find "$s" -maxdepth 6 \( -name node_modules -o -name target \) -prune -o \( -name .git -print -prune \) 2>/dev/null)"
  targets="$(find "$s" -maxdepth 6 \( -name node_modules -o -name .git \) -prune -o -type d -name target -print -prune 2>/dev/null |
    while IFS= read -r t; do [ -f "$t/CACHEDIR.TAG" ] && printf '%s\n' "$t"; done)"

  CK_BUF="" CK_ACT=""
  while IFS= read -r g; do
    [ -n "$g" ] || continue
    scan_checkout "${g%/.git}"
  done <<GITS
$gits
GITS

  # Last activity: the folder's own entries two levels deep, each checkout's git
  # files, and each target dir (sampled as for a worktree).
  act="$(find_newest "$s" -maxdepth 2 \( -name node_modules -o -name target \) -prune -o \( -type d -o -type f \))"
  [ -n "$CK_ACT" ] && { [ -z "$act" ] || [ "$CK_ACT" -gt "$act" ]; } && act="$CK_ACT"
  m="$(targets_activity "$targets")"
  [ -n "$m" ] && { [ -z "$act" ] || [ "$m" -gt "$act" ]; } && act="$m"
  idle=""
  if [ -n "$act" ]; then idle=$(( (NOW - act) / 86400 )); [ "$idle" -lt 0 ] && idle=0; fi

  if cargo_busy "$s"; then busy=1; fi
  here="$(cd "$PWD" 2>/dev/null && pwd -P)"
  case "$here/" in "$s"/*) cur=1 ;; esac
  case "$(cd "$REPO" 2>/dev/null && pwd -P)/" in "$s"/*) cur=1 ;; esac

  SCAN="${SCAN}S${US}${slug}${US}${uuid}${US}${s}${US}${kib}${US}${act}${US}${idle}${US}${busy}${US}${cur}${NL}${CK_BUF}"

  while IFS= read -r t; do
    [ -n "$t" ] || continue
    tkib="$(printf '%s\n' "$du_out" | awk -F'\t' -v p="$t" '$2 == p { print $1; exit }')"
    [ -n "$tkib" ] || tkib="$(du_kib "$t")"
    owner="" best=0
    while IFS= read -r g; do
      [ -n "$g" ] || continue
      ck="${g%/.git}"
      case "$t" in "$ck"/*) if [ "${#ck}" -gt "$best" ]; then owner="$ck"; best="${#ck}"; fi ;; esac
    done <<GITS
$gits
GITS
    tpr=unknown
    if [ -n "$owner" ]; then
      tpr="$(printf '%s' "$CK_BUF" | awk -F"$US" -v p="$owner" '$1 == "C" && $2 == p { print $8; exit }')"
    fi
    SCAN="${SCAN}T${US}${t}${US}${tkib}${US}${owner}${US}${tpr:-unknown}${NL}"
  done <<TARGETS
$targets
TARGETS
}

scan_scratch() {
  SCAN="" PRCACHE=""
  [ -d "$SCRATCH_DIR" ] || return 0
  local sd slug s
  for sd in "$SCRATCH_DIR"/*/; do
    sd="${sd%/}"
    [ -d "$sd" ] && [ ! -L "$sd" ] || continue
    slug="${sd##*/}"
    if is_dohflow_slug "$slug"; then
      for s in "$sd"/*/; do
        s="${s%/}"
        [ -d "$s" ] && [ ! -L "$s" ] || continue
        scan_session "$s" "$slug"
      done
    else
      SCAN="${SCAN}F${US}${slug}${US}$(du_kib "$sd")${NL}"
    fi
  done
}

# ---------------------------------------------------------------------------
# Tier 2 decisions over SCAN. Sets T2_ACTIONS (the exact actions --clean
# --scratch would take, one per line, "CARGO<US>target" or "RMWT<US>checkout",
# every CARGO before every RMWT) and T2_VERDICT (path<US>"ok" or the first reason
# the path is left alone).
# ---------------------------------------------------------------------------

T2_ACTIONS="" T2_VERDICT=""

evaluate_scratch() {
  T2_ACTIONS="" T2_VERDICT=""
  local rec tag a b c d e f g h i j sess="" why rmwt="" v
  while IFS= read -r rec; do
    [ -n "$rec" ] || continue
    IFS="$US" read -r tag a b c d e f g h i j <<<"$rec"
    case "$tag" in
      S)
        # a=slug b=uuid c=path d=kib e=activity f=idle g=busy h=current
        sess=""
        if [ "$h" = 1 ]; then sess="this is the folder (or repository) the script runs from"
        elif [ "$g" = 1 ]; then sess="a cargo/rustc process is running in it (or could not be checked)"
        elif [ -z "$f" ]; then sess="last activity unknown"
        elif [ "$f" -lt "$IDLE_DAYS" ]; then sess="active ${f} day(s) ago, threshold is ${IDLE_DAYS}"
        fi ;;
      C)
        # a=path b=kind c=branch d=head e=dirty f=unpushed g=pr_state h=pr_num i=ignored_ok j=common
        why="$sess"
        if [ -n "$why" ]; then :
        elif [ "$b" != registered ]; then why="unregistered copy: removing it stays an explicit, owner-approved act"
        elif [ "$e" != 0 ]; then why="uncommitted changes (or status unreadable)"
        elif [ "$f" != 0 ]; then why="commits that exist on no remote (or unreadable)"
        elif [ "$g" != merged ] && [ "$g" != closed ]; then why="pull request state is ${g} (only merged or closed qualifies)"
        fi
        T2_VERDICT="${T2_VERDICT}${a}${US}${why:-ok}${NL}"
        if [ -z "$why" ]; then
          # git worktree remove deletes ignored files too: only when the sole
          # ignored entries are rebuildable output.
          if [ "$i" = 1 ] && [ -n "$j" ]; then rmwt="${rmwt}RMWT${US}${a}${NL}"; fi
        fi ;;
      T)
        # a=path b=kib c=owner d=pr_state
        if [ -z "$c" ]; then why="not inside a checkout, so no pull request ties it to merged or closed work"
        else
          v="$(printf '%s' "$T2_VERDICT" | awk -F"$US" -v p="$c" '$1 == p { print $2; exit }')"
          if [ "$v" = ok ]; then why=""; else why="its checkout: ${v:-unknown}"; fi
        fi
        T2_VERDICT="${T2_VERDICT}${a}${US}${why:-ok}${NL}"
        [ -z "$why" ] && T2_ACTIONS="${T2_ACTIONS}CARGO${US}${a}${NL}" ;;
    esac
  done <<SCANNED
$SCAN
SCANNED
  T2_ACTIONS="${T2_ACTIONS}${rmwt}"
}

verdict_for() {
  # verdict_for PATH: the first verdict line for PATH ("ok" or a reason).
  printf '%s' "$T2_VERDICT" | awk -F"$US" -v p="$1" '$1 == p { print $2; exit }'
}

# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------

REPORT_N=0
text_row() {
  REPORT_N=$((REPORT_N + 1))
  echo
  echo "  $W_PATH"
  if [ "$W_MISSING" = 1 ]; then
    echo "    registration: directory is gone (prunable)"
    return
  fi
  echo "    branch:       ${W_BRANCH:-(detached at ${W_HEAD:0:8})}"
  if [ -z "$W_TARGETS" ]; then
    echo "    target:       $(human 0) (no cargo target dir)"
  else
    echo "    target:       $(human "$W_TARGET_KIB") in $(printf '%s' "$W_TSIZES" | grep -c .) cargo target dir(s), every workspace counted"
    local trec tk tp
    while IFS= read -r trec; do
      [ -n "$trec" ] || continue
      tk="${trec%%"$US"*}"; tp="${trec#*"$US"}"
      echo "                  $(human "$tk")  ${tp#"$W_PATH"/}"
    done <<TSIZES
$W_TSIZES
TSIZES
    if [ "$W_OVER" = 1 ]; then
      echo "    OVER THE CAP: $(human "$W_TARGET_KIB") against $(human "$CAP_KIB") (--target-cap-gib). Its owner session runs"
      echo "                  scripts/dev-hygiene.sh --cap-check at a quiet point and, if it says over, cargo clean"
      echo "                  in this worktree's own target dirs (docs/agent/WORKFLOW_ROLES.md). Nobody else cleans it."
    fi
  fi
  if [ -n "$W_ACTIVITY" ]; then
    echo "    last active:  $(date -r "$W_ACTIVITY" '+%Y-%m-%d %H:%M' 2>/dev/null || date -d "@$W_ACTIVITY" '+%Y-%m-%d %H:%M' 2>/dev/null) (${W_IDLE_DAYS} day(s) ago)"
  else
    echo "    last active:  unknown"
  fi
  local state="clean"
  [ "$W_DIRTY" = 1 ] && state="UNCOMMITTED CHANGES"
  [ -z "$W_DIRTY" ] && state="status unreadable"
  echo "    working tree: $state"
  if [ -z "$W_UNPUSHED" ]; then
    echo "    unpushed:     unreadable"
  else
    echo "    unpushed:     $W_UNPUSHED commit(s) on no remote"
  fi
  local br=""
  [ "$W_MERGED" = 1 ] && br="merged into origin/main"
  [ "$W_GONE" = 1 ] && br="${br:+$br; }remote branch gone (usually merged and deleted)"
  [ -n "$br" ] && echo "    branch state: $br"
  [ "$W_BUSY" = 1 ] && echo "    cargo:        a cargo/rustc process is running (or could not be checked)"
  if [ "$W_ELIGIBLE" = 1 ]; then
    echo "    --clean:      WOULD run cargo clean"
  else
    echo "    --clean:      skipped — $W_SKIP"
  fi
}

# json_tri VALUE: "" -> null, "1" -> true, anything else -> false.
json_tri() {
  if [ -z "$1" ]; then
    echo null
  elif [ "$1" = 1 ]; then
    echo true
  else
    echo false
  fi
}

JSON_FIRST=1
json_row() {
  [ "$JSON_FIRST" = 1 ] && JSON_FIRST=0 || printf ','
  printf '\n    {"path":"%s","branch":' "$(json_str "$W_PATH")"
  if [ -n "$W_BRANCH" ]; then printf '"%s"' "$(json_str "$W_BRANCH")"; else printf 'null'; fi
  printf ',"head":"%s","directory_missing":%s' "$(json_str "$W_HEAD")" "$([ "$W_MISSING" = 1 ] && echo true || echo false)"
  printf ',"target_dirs":['
  local first=1 t
  while IFS= read -r t; do
    [ -n "$t" ] || continue
    [ "$first" = 1 ] && first=0 || printf ','
    printf '"%s"' "$(json_str "$t")"
  done <<EOF
$W_TARGETS
EOF
  printf '],"targets":['
  local trec tk tp tfirst=1
  while IFS= read -r trec; do
    [ -n "$trec" ] || continue
    tk="${trec%%"$US"*}"; tp="${trec#*"$US"}"
    [ "$tfirst" = 1 ] && tfirst=0 || printf ','
    printf '{"path":"%s","kib":%s}' "$(json_str "$tp")" "$tk"
  done <<TSIZES
$W_TSIZES
TSIZES
  printf '],"target_kib":%s,"over_cap":%s' "$W_TARGET_KIB" "$([ "$W_OVER" = 1 ] && echo true || echo false)"
  printf ',"last_activity":%s,"idle_days":%s' "${W_ACTIVITY:-null}" "${W_IDLE_DAYS:-null}"
  printf ',"dirty":%s' "$(json_tri "$W_DIRTY")"
  printf ',"unpushed_commits":%s' "${W_UNPUSHED:-null}"
  printf ',"merged_into_origin_main":%s' "$(json_tri "$W_MERGED")"
  printf ',"remote_branch_gone":%s' "$([ "$W_GONE" = 1 ] && echo true || echo false)"
  printf ',"cargo_running":%s' "$([ "$W_BUSY" = 1 ] && echo true || echo false)"
  printf ',"clean_eligible":%s,"skip_reason":' "$([ "$W_ELIGIBLE" = 1 ] && echo true || echo false)"
  if [ -n "$W_SKIP" ]; then printf '"%s"' "$(json_str "$W_SKIP")"; else printf 'null'; fi
  printf '}'
}

slug_label() {
  # The part of a project slug after "Programs-", which names the lane checkout.
  local s="${1##*Programs-}"
  printf '%s' "${s:-$1}"
}

scratch_totals() {
  # Sets SC_DOH_KIB, SC_OTHER_KIB and scratch_kib (their sum) from SCAN.
  SC_DOH_KIB="$(printf '%s' "$SCAN" | awk -F"$US" '$1 == "S" { n += $5 } END { print n + 0 }')"
  SC_OTHER_KIB="$(printf '%s' "$SCAN" | awk -F"$US" '$1 == "F" { n += $3 } END { print n + 0 }')"
  scratch_kib=$(( SC_DOH_KIB + SC_OTHER_KIB ))
}

date_of() {
  date -r "$1" '+%Y-%m-%d %H:%M' 2>/dev/null || date -d "@$1" '+%Y-%m-%d %H:%M' 2>/dev/null
}

pr_text() {
  # pr_text STATE NUMBER
  if [ -n "$2" ]; then printf 'PR #%s %s' "$2" "$1"; else printf 'PR %s' "$1"; fi
}

t2_text() {
  # t2_text VERDICT ACTION-WORD: one line on what --clean --scratch does here.
  if [ "$1" = ok ]; then echo "would $2"; else echo "skipped — $1"; fi
}

scratch_text() {
  local rec tag a b c d e f g h i j v flags state br
  if [ ! -d "$SCRATCH_DIR" ]; then
    echo "Session scratch ($SCRATCH_DIR): not present"
    return
  fi
  echo "Session scratch ($SCRATCH_DIR): $(human "$scratch_kib") — DohFlow sessions $(human "$SC_DOH_KIB"), other projects $(human "$SC_OTHER_KIB") (report only unless --clean --scratch)"
  if low_disk; then
    echo "  ** Free space is under 15% and scratch holds $(human "$scratch_kib"). Ask the owner before any --clean --scratch. **"
  fi
  echo
  echo "DohFlow session folders (one du walk each; PR state from gh, unknown when it cannot be had):"
  while IFS= read -r rec; do
    [ -n "$rec" ] || continue
    IFS="$US" read -r tag a b c d e f g h i j <<<"$rec"
    case "$tag" in
      S)
        flags=""
        [ "${d:-0}" -gt "$BIG_KIB" ] && flags="$flags  ** OVER $(human "$BIG_KIB") **"
        [ "$g" = 1 ] && flags="$flags  [cargo/rustc running]"
        [ "$h" = 1 ] && flags="$flags  [the folder this runs from]"
        echo
        echo "  $(slug_label "$a") / $b   $(human "${d:-0}")${flags}"
        if [ -n "$e" ]; then echo "    last active:  $(date_of "$e") (${f} day(s) ago)"; else echo "    last active:  unknown"; fi ;;
      C)
        br="${c:-(detached at ${d:0:8})}"
        state="clean"; [ "$e" = 1 ] && state="UNCOMMITTED CHANGES"; [ -z "$e" ] && state="status unreadable"
        echo "    checkout:     $a  [$b]"
        echo "                  $br; $state; ${f:-unreadable} unpushed; $(pr_text "$g" "$h")"
        v="$(verdict_for "$a")"
        if [ "$v" = ok ] && [ "$i" != 1 ]; then
          echo "                  --clean --scratch: would not remove it: ignored files other than target/ and node_modules/ are present"
        else
          echo "                  --clean --scratch: $(t2_text "$v" 'remove the worktree (plain, never forced)')"
        fi ;;
      T)
        echo "    cargo target: $(human "$b")  $a"
        echo "                  $([ -n "$c" ] && echo "owner $c" || echo "no owning checkout"); $(pr_text "$d" "")"
        echo "                  --clean --scratch: $(t2_text "$(verdict_for "$a")" 'run cargo clean')" ;;
      F) : ;;
    esac
  done <<SCANNED
$SCAN
SCANNED
  if [ "$(printf '%s' "$SCAN" | awk -F"$US" '$1 == "S"' | wc -l | tr -d ' ')" = 0 ]; then
    echo "  (none)"
  fi
  echo
  echo "Other projects under the scratch root (size only, never opened):"
  if [ "$(printf '%s' "$SCAN" | awk -F"$US" '$1 == "F"' | wc -l | tr -d ' ')" = 0 ]; then
    echo "  (none)"
  else
    while IFS= read -r rec; do
      IFS="$US" read -r tag a b <<<"$rec"
      [ "$tag" = F ] && echo "  $(human "$b")  $(slug_label "$a")"
    done <<SCANNED
$SCAN
SCANNED
  fi
  echo
  if [ -z "$T2_ACTIONS" ]; then
    echo "Safe to clean now under --clean --scratch (idle at least ${IDLE_DAYS} day(s), PR merged or closed): nothing."
  else
    echo "Safe to clean now under --clean --scratch (idle at least ${IDLE_DAYS} day(s), PR merged or closed):"
    while IFS= read -r rec; do
      [ -n "$rec" ] || continue
      IFS="$US" read -r tag a <<<"$rec"
      if [ "$tag" = CARGO ]; then
        echo "  cargo clean            $a  ($(human "$(printf '%s' "$SCAN" | awk -F"$US" -v p="$a" '$1 == "T" && $2 == p { print $3; exit }')"))"
      else
        echo "  remove worktree        $a"
      fi
    done <<ACTIONS
$T2_ACTIONS
ACTIONS
  fi
}

# json_scratch: the "sessions", "other_projects" and "tier2_candidates" members.
json_scratch() {
  local rec tag a b c d e f g h i j first_s=1 first_c=1 first_t=1 part="" first_o=1
  printf '"sessions":['
  while IFS= read -r rec; do
    [ -n "$rec" ] || continue
    IFS="$US" read -r tag a b c d e f g h i j <<<"$rec"
    case "$tag" in
      S)
        if [ -n "$part" ]; then
          if [ "$part" = ck ]; then printf '],"targets":[]}'; else printf ']}'; fi
        fi
        [ "$first_s" = 1 ] && first_s=0 || printf ','
        printf '\n    {"slug":"%s","uuid":"%s","path":"%s","kib":%s' "$(json_str "$a")" "$(json_str "$b")" "$(json_str "$c")" "${d:-null}"
        printf ',"last_activity":%s,"idle_days":%s,"cargo_running":%s' "${e:-null}" "${f:-null}" "$([ "$g" = 1 ] && echo true || echo false)"
        printf ',"over_size_flag":%s,"run_from_here":%s,"checkouts":[' "$([ "${d:-0}" -gt "$BIG_KIB" ] && echo true || echo false)" "$([ "$h" = 1 ] && echo true || echo false)"
        part=ck first_c=1 first_t=1 ;;
      C)
        [ "$first_c" = 1 ] && first_c=0 || printf ','
        printf '{"path":"%s","registered":%s,"branch":' "$(json_str "$a")" "$([ "$b" = registered ] && echo true || echo false)"
        if [ -n "$c" ]; then printf '"%s"' "$(json_str "$c")"; else printf 'null'; fi
        printf ',"head":"%s","dirty":%s,"unpushed_commits":%s' "$(json_str "$d")" "$(json_tri "$e")" "${f:-null}"
        printf ',"pr_state":"%s","pr_number":%s' "$g" "${h:-null}"
        printf ',"tier2":"%s"}' "$(json_str "$(verdict_for "$a")")" ;;
      T)
        if [ "$part" = ck ]; then printf '],"targets":['; part=t; fi
        [ "$first_t" = 1 ] && first_t=0 || printf ','
        printf '{"path":"%s","kib":%s,"owner":' "$(json_str "$a")" "${b:-null}"
        if [ -n "$c" ]; then printf '"%s"' "$(json_str "$c")"; else printf 'null'; fi
        printf ',"pr_state":"%s","tier2":"%s"}' "$d" "$(json_str "$(verdict_for "$a")")" ;;
    esac
  done <<SCANNED
$SCAN
SCANNED
  if [ -n "$part" ]; then
    if [ "$part" = ck ]; then printf '],"targets":[]}'; else printf ']}'; fi
  fi
  printf '\n  ],\n  "other_projects":['
  while IFS= read -r rec; do
    [ -n "$rec" ] || continue
    IFS="$US" read -r tag a b <<<"$rec"
    [ "$tag" = F ] || continue
    [ "$first_o" = 1 ] && first_o=0 || printf ','
    printf '{"slug":"%s","kib":%s}' "$(json_str "$a")" "${b:-null}"
  done <<SCANNED
$SCAN
SCANNED
  printf '],\n  "tier2_candidates":['
  first_o=1
  while IFS= read -r rec; do
    [ -n "$rec" ] || continue
    IFS="$US" read -r tag a <<<"$rec"
    [ "$first_o" = 1 ] && first_o=0 || printf ','
    printf '{"action":"%s","path":"%s"}' "$([ "$tag" = CARGO ] && echo cargo_clean || echo git_worktree_remove)" "$(json_str "$a")"
  done <<ACTIONS
$T2_ACTIONS
ACTIONS
  printf ']'
}

report() {
  read_disk
  read_snapshots
  scan_scratch
  scratch_totals
  evaluate_scratch
  if [ "$FORMAT" = json ]; then
    printf '{\n  "generated_at":%s,\n  "idle_days_threshold":%s,\n' "$NOW" "$IDLE_DAYS"
    printf '  "disk":{"total_kib":%s,"avail_kib":%s,"percent_free":%s,"under_15_percent":%s},\n' \
      "${disk_total_kib:-null}" "${disk_avail_kib:-null}" "${disk_pct_free:-null}" "$(low_disk && echo true || echo false)"
    printf '  "local_snapshots":%s,"snapshots_may_delay_freed_space":%s,"target_cap_kib":%s,\n' \
      "${LOCAL_SNAPSHOTS:-null}" "$(snapshots_delay && echo true || echo false)" "$CAP_KIB"
    printf '  "scratch":{"path":"%s","kib":%s,"dohflow_kib":%s,"other_projects_kib":%s,"session_flag_kib":%s,\n  ' \
      "$(json_str "$SCRATCH_DIR")" "${scratch_kib:-null}" "$SC_DOH_KIB" "$SC_OTHER_KIB" "$BIG_KIB"
    json_scratch
    printf '},\n  "worktrees":['
    for_each_worktree json_row
    printf '\n  ]\n}\n'
    return
  fi
  echo "dev-hygiene report (read-only) — $(date '+%Y-%m-%d %H:%M')"
  echo
  if [ -n "$disk_total_kib" ]; then
    echo "Disk: $(human "$disk_total_kib") total, $(human "$disk_avail_kib") free (${disk_pct_free}% free)"
    if low_disk; then
      echo "  ** Under 15% free. Ask the owner before any --clean (docs/agent/WORKFLOW_ROLES.md). **"
    fi
  else
    echo "Disk: unreadable"
  fi
  if [ -n "$LOCAL_SNAPSHOTS" ]; then
    echo "Time Machine local snapshots: $LOCAL_SNAPSHOTS"
    if snapshots_delay; then
      echo "  ** Local snapshots exist and free space is under 15%: space freed by a clean may not return until the"
      echo "     owner thins them. See the owner-side note in docs/agent/WORKFLOW_ROLES.md (Build-machine hygiene)."
      echo "     Agents never run thinlocalsnapshots or sudo. **"
    fi
  fi
  echo
  echo "Per-worktree cargo target cap: $(human "$CAP_KIB"), every workspace counted together (--target-cap-gib)."
  echo "Registered worktrees:"
  for_each_worktree text_row
  echo
  scratch_text
  echo
  echo "Idle threshold for --clean: ${IDLE_DAYS} day(s). Nothing was changed."
}

# ---------------------------------------------------------------------------
# Clean
# ---------------------------------------------------------------------------

PLAN=""          # one line per planned worktree: PATH<TAB>TARGET-LIST (newline-joined as |)
CLEANED=0 CLEANED_KIB=0

target_key() {
  # The targets of the current worktree on one line, for exact comparison.
  printf '%s' "$W_TARGETS" | tr '\n' '|'
}

plan_row() {
  if [ "$W_ELIGIBLE" = 1 ]; then
    PLAN="${PLAN}${W_PATH}${TAB}$(target_key)${NL}"
    echo "  clean   $W_PATH  ($(human "$W_TARGET_KIB"), idle ${W_IDLE_DAYS} day(s))"
  else
    echo "  skip    $W_PATH  — $W_SKIP"
  fi
}

# Runs after the confirmation, with a FRESH inspection of every worktree, because
# the prompt can wait indefinitely and a worktree can change meanwhile. It cleans
# exactly the targets the plan showed, and only if the worktree is still eligible.
apply_row() {
  local planned t d line key
  planned=""
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    case "$line" in "$W_PATH$TAB"*) planned="${line#*"$TAB"}"; break ;; esac
  done <<PLANNED
$PLAN
PLANNED
  [ -n "$planned" ] || return 0
  key="$(target_key)"
  if [ "$W_ELIGIBLE" != 1 ] || [ "$key" != "$planned" ]; then
    echo "  skipped $W_PATH — changed since the plan (${W_SKIP:-its target directories changed})"
    return 0
  fi
  while IFS= read -r t; do
    [ -n "$t" ] || continue
    d="$(dirname "$t")"
    if [ ! -f "$d/Cargo.toml" ]; then
      echo "  skipped $t — no Cargo.toml beside it"
      continue
    fi
    # --target-dir is explicit so a CARGO_TARGET_DIR in the environment can
    # never redirect this to some other directory.
    if cargo clean --manifest-path "$d/Cargo.toml" --target-dir "$t"; then
      CLEANED=$((CLEANED + 1))
    else
      echo "  cargo clean failed in $d" >&2
    fi
  done <<TARGETS
$W_TARGETS
TARGETS
  CLEANED_KIB=$((CLEANED_KIB + W_TARGET_KIB))
}

scratch_plan_text() {
  # What --clean --scratch will do, and why it leaves the rest alone.
  local rec tag a b c d e f g h i j v
  echo "Scratch (--scratch) under $SCRATCH_DIR:"
  while IFS= read -r rec; do
    [ -n "$rec" ] || continue
    IFS="$US" read -r tag a b c d e f g h i j <<<"$rec"
    case "$tag" in
      C|T)
        v="$(verdict_for "$a")"
        if [ "$v" != ok ]; then
          echo "  skip    $a  — $v"
        elif [ "$tag" = C ] && [ "$i" != 1 ]; then
          echo "  keep    $a  — cargo clean only: ignored files other than target/ and node_modules/ are present"
        fi ;;
    esac
  done <<SCANNED
$SCAN
SCANNED
  while IFS= read -r rec; do
    [ -n "$rec" ] || continue
    IFS="$US" read -r tag a <<<"$rec"
    if [ "$tag" = CARGO ]; then
      echo "  clean   $a  ($(human "$(printf '%s' "$SCAN" | awk -F"$US" -v p="$a" '$1 == "T" && $2 == p { print $3; exit }')"))"
    else
      echo "  remove  $a  (plain worktree removal, never forced)"
    fi
  done <<ACTIONS
$T2_ACTIONS
ACTIONS
  [ -n "$T2_ACTIONS" ] || echo "  nothing qualifies (merged or closed PR, idle, clean, pushed, registered, not running)."
}

# Runs after the confirmation, against a FRESH scan: the prompt can wait
# indefinitely. An action runs only if the fresh scan still plans it.
apply_scratch() {
  local planned="$T2_ACTIONS" rec tag a d common cleaned=0 removed=0 left=0
  scan_scratch
  evaluate_scratch
  while IFS= read -r rec; do
    [ -n "$rec" ] || continue
    case "$NL$T2_ACTIONS" in
      *"$NL$rec$NL"*) ;;
      *) echo "  skipped ${rec#*"$US"} — changed since the plan"; continue ;;
    esac
    IFS="$US" read -r tag a <<<"$rec"
    case "$a" in "$SCRATCH_DIR"/*) ;; *) echo "  skipped $a — outside the scratch directory"; continue ;; esac
    [ ! -L "$a" ] || { echo "  skipped $a — a symbolic link"; continue; }
    if [ "$tag" = CARGO ]; then
      d="$(dirname "$a")"
      if [ ! -f "$d/Cargo.toml" ]; then echo "  skipped $a — no Cargo.toml beside it"; continue; fi
      if cargo clean --manifest-path "$d/Cargo.toml" --target-dir "$a"; then
        cleaned=$((cleaned + 1))
      else
        echo "  cargo clean failed in $d" >&2; left=$((left + 1))
      fi
    else
      common="$(printf '%s' "$SCAN" | awk -F"$US" -v p="$a" '$1 == "C" && $2 == p { print $11; exit }')"
      if [ -z "$common" ]; then echo "  skipped $a — its repository could not be found"; continue; fi
      # Never --force: git itself refuses a dirty or locked worktree.
      if git --git-dir="$common" worktree remove "$a"; then
        removed=$((removed + 1))
      else
        echo "  worktree removal refused or failed for $a" >&2; left=$((left + 1))
      fi
    fi
  done <<PLANNED
$planned
PLANNED
  echo "Scratch: cargo clean ran on $cleaned target dir(s); worktree removal ran on $removed worktree(s); $left failed."
}

clean() {
  command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH; nothing to clean with"

  echo "dev-hygiene --clean plan"
  echo
  for_each_worktree plan_row
  echo
  local prune_list
  prune_list="$(git -C "$REPO" worktree prune --dry-run -v 2>&1)"
  if [ -n "$prune_list" ]; then
    echo "Registrations to prune (directory already gone):"
    printf '%s\n' "$prune_list" | sed 's/^/  /'
  else
    echo "No registrations to prune."
  fi
  echo
  if [ "$SCRATCH_CLEAN" = 1 ]; then
    scan_scratch
    evaluate_scratch
    scratch_plan_text
    echo
    echo "This runs only 'cargo clean' in the worktrees marked 'clean', 'git worktree prune',"
    echo "and, for the scratch items above, 'cargo clean' and a plain worktree removal."
    echo "It removes no branch, no unregistered copy and no other directory."
  else
    echo "This runs only 'cargo clean' in the worktrees marked 'clean' and"
    echo "'git worktree prune'. It removes no worktree, branch or scratch directory."
  fi

  if [ -z "$PLAN" ] && [ -z "$prune_list" ] && [ -z "$T2_ACTIONS" ]; then
    echo "Nothing to do."
    return
  fi

  if [ "$ASSUME_YES" != 1 ]; then
    if [ ! -t 0 ]; then
      echo "Not a terminal and --yes was not given: nothing done." >&2
      return 1
    fi
    printf 'Proceed? [y/N] '
    local answer
    read -r answer
    case "$answer" in y|Y|yes|YES) ;; *) echo "Cancelled; nothing done."; return 0 ;; esac
  fi

  # Test hook only: an executable run between the confirmation and the apply
  # step, so the tests can change a worktree "while the prompt waited".
  if [ "${DEV_HYGIENE_TESTING:-}" = 1 ] && [ -n "${DEV_HYGIENE_AFTER_CONFIRM_HOOK:-}" ]; then
    "$DEV_HYGIENE_AFTER_CONFIRM_HOOK" || true
  fi

  NOW="$(date +%s)"
  PROCS_LOADED=0
  for_each_worktree apply_row
  if [ "$SCRATCH_CLEAN" = 1 ]; then apply_scratch; fi
  echo "cargo clean ran on $CLEANED target dir(s); about $(human "$CLEANED_KIB") was in them."

  if [ -n "$prune_list" ]; then
    git -C "$REPO" worktree prune -v
  fi
  read_disk
  [ -n "$disk_avail_kib" ] && echo "Disk now: $(human "$disk_avail_kib") free (${disk_pct_free}% free). Time Machine local snapshots may hold freed space for a while (docs/agent/WORKFLOW_ROLES.md)."
}

# --cap-check (personal-cfo-0rein.13): the worktree this is run from, and no
# other. Read-only. Exit 0 under the cap, 1 over it and quiet, 2 over it with a
# build running (or one that could not be ruled out).
cap_check() {
  local top trec tk tp d
  top="$(git -C "$PWD" rev-parse --show-toplevel 2>/dev/null)" || die "--cap-check must be run inside a git worktree"
  top="$(cd "$top" && pwd -P)"
  W_PATH="$top"
  W_TARGETS="$(find_targets "$top")"
  measure_targets
  echo "dev-hygiene --cap-check (read-only) — $top"
  if [ -z "$W_TARGETS" ]; then
    echo "  no cargo target dir"
  else
    while IFS= read -r trec; do
      [ -n "$trec" ] || continue
      tk="${trec%%"$US"*}"; tp="${trec#*"$US"}"
      echo "  $(human "$tk")  ${tp#"$top"/}"
    done <<TSIZES
$W_TSIZES
TSIZES
  fi
  echo "  total $(human "$W_TARGET_KIB") against the cap of $(human "$CAP_KIB")"
  if cargo_busy "$top"; then
    echo "  cargo/rustc: a process is running in this worktree (or that could not be ruled out)"
    W_BUSY=1
  else
    echo "  cargo/rustc: none running"
  fi
  if [ "$W_OVER" != 1 ]; then
    echo "  Under the cap. Nothing to do."
    return 0
  fi
  if [ "$W_BUSY" = 1 ]; then
    echo "  OVER THE CAP, and a build is running: do not clean. Check again when it has finished."
    return 2
  fi
  echo "  OVER THE CAP, and quiet. Clean your own target dirs (nothing else, never rm):"
  while IFS= read -r tp; do
    [ -n "$tp" ] || continue
    d="$(dirname "$tp")"
    echo "    cargo clean --manifest-path \"$d/Cargo.toml\" --target-dir \"$tp\""
  done <<TARGETS
$W_TARGETS
TARGETS
  echo "  This script did not clean anything."
  return 1
}

if [ "$CAP_CHECK" = 1 ]; then
  cap_check
  exit $?
fi
case "$MODE" in
  report) report ;;
  clean) clean ;;
esac
