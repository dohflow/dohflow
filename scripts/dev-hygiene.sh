#!/usr/bin/env bash
#
# dev-hygiene.sh — stop build caches and stale worktrees from filling the build
# machine (personal-cfo-0rein.6).
#
# READ-ONLY BY DEFAULT. `--report` (the default) changes nothing. `--clean` is
# explicit and does exactly two things, nothing else:
#
#   1. `cargo clean` for each Cargo target directory of a registered worktree
#      that has been idle for at least --idle-days days (default 3), has no
#      uncommitted changes, has no commits that exist on no remote, and has no
#      running cargo/rustc process; and
#   2. `git worktree prune` for registrations whose directory is already gone.
#
# It NEVER removes a worktree directory, a branch, or a scratch directory; it
# never runs `rm`; it never touches a dirty or unpushed worktree; and it never
# touches the worktree it is run from (AGENTS.md section 1: non-destructive by
# default). Scratch directories under /private/tmp are only measured.
#
# Usage:
#   scripts/dev-hygiene.sh [--report] [--json]
#   scripts/dev-hygiene.sh --clean [--idle-days N] [--yes]
#   scripts/dev-hygiene.sh --help
#
# Options:
#   --report          Print the report (default). Read-only.
#   --json            Print the report as JSON instead of text. Read-only.
#   --clean           Run the two actions above after showing the plan and
#                     asking for confirmation.
#   --yes             Skip the confirmation prompt (with --clean only).
#   --idle-days N     Idle threshold in days for --clean (default 3).
#   --scratch-dir D   Session scratch directory to measure (report only;
#                     default /private/tmp/claude-<uid>).
#   --repo DIR        Repository to inspect (default: this script's repository).
#
# "Last activity" of a worktree is the newest modification time of its git
# index, HEAD and HEAD reflog, and of the directories (two levels deep) and top
# files of its cargo target directories. It is an approximation: a worktree
# where tests were only re-run, with nothing rebuilt, can look older than it is.
# That is why --clean also checks for running cargo/rustc processes, and why the
# worst outcome of a wrong guess is a rebuild, never lost work.
#
# Environment: DEV_HYGIENE_AFTER_CONFIRM_HOOK, honored only when
# DEV_HYGIENE_TESTING=1 (tests only), runs between the confirmation and the
# apply step. DEV_HYGIENE_PROCESS_NAMES overrides the process names treated as
# "a build is running" (default "cargo rustc rust-analyzer"); the tests use it.
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
SCRATCH_DIR="/private/tmp/claude-$(id -u)"
REPO=""

usage() {
  sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
}

die() { echo "dev-hygiene: $*" >&2; exit 2; }

while [ $# -gt 0 ]; do
  case "$1" in
    --report) MODE=report ;;
    --json) FORMAT=json ;;
    --clean) MODE=clean ;;
    --yes) ASSUME_YES=1 ;;
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
if [ "$ASSUME_YES" = 1 ] && [ "$MODE" != clean ]; then
  die "--yes only makes sense with --clean"
fi
if [ "$FORMAT" = json ] && [ "$MODE" = clean ]; then
  die "--json is a report format; it cannot be combined with --clean"
fi

if [ -z "$REPO" ]; then
  REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
fi
git -C "$REPO" rev-parse --git-dir >/dev/null 2>&1 || die "not a git repository: $REPO"

NOW="$(date +%s)"
TAB="$(printf '\t')"
NL="
"

# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------

mtime() {
  if [ "$(uname)" = Darwin ]; then
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
W_BUSY=0 W_CURRENT=0 W_SKIP="" W_ELIGIBLE=0

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
  # newest_mtime FILE_OR_DIR...: the largest mtime among those that exist.
  local best="" m f
  for f in "$@"; do
    [ -e "$f" ] || continue
    m="$(mtime "$f")"
    [ -n "$m" ] || continue
    if [ -z "$best" ] || [ "$m" -gt "$best" ]; then best="$m"; fi
  done
  printf '%s' "$best"
}

worktree_activity() {
  # worktree_activity PATH: newest activity epoch, or empty if unknowable.
  local wt="$1" gitdir best="" m t d f
  gitdir="$(git -C "$wt" rev-parse --absolute-git-dir 2>/dev/null)"
  if [ -n "$gitdir" ]; then
    best="$(newest_mtime "$gitdir/index" "$gitdir/HEAD" "$gitdir/logs/HEAD")"
  fi
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
$W_TARGETS
EOF
  printf '%s' "$best"
}

cargo_busy() {
  # cargo_busy PATH: 0 if a cargo/rustc process works in or on PATH. If this
  # cannot be determined, answer "busy" (the safe direction). Both the path as
  # registered and its physical form are compared, because lsof reports the
  # physical one (on macOS /var is /private/var).
  local wt="$1" phys pid cwd cmd pids name
  phys="$(cd "$wt" 2>/dev/null && pwd -P)"
  [ -n "$phys" ] || phys="$wt"
  command -v pgrep >/dev/null 2>&1 || return 0
  pids=""
  for name in ${DEV_HYGIENE_PROCESS_NAMES:-cargo rustc rust-analyzer}; do
    pids="$pids $(pgrep -x "$name" 2>/dev/null)"
  done
  pids="$(printf '%s' "$pids" | tr -s ' \n' ' ')"
  [ -n "${pids// /}" ] || return 1
  for pid in $pids; do
    cmd="$(ps -p "$pid" -o command= 2>/dev/null)"
    case "$cmd" in *"$wt/"*|*"$phys/"*) return 0 ;; esac
    if command -v lsof >/dev/null 2>&1; then
      cwd="$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -n 1)"
    elif [ -e "/proc/$pid/cwd" ]; then
      cwd="$(readlink "/proc/$pid/cwd" 2>/dev/null)"
    else
      return 0   # cannot tell where it works: assume it is in this worktree
    fi
    [ -n "$cwd" ] || return 0   # could not read it (not ours, or it just exited): assume busy
    case "$cwd" in "$wt"|"$wt"/*|"$phys"|"$phys"/*) return 0 ;; esac
  done
  return 1
}

inspect_worktree() {
  # inspect_worktree PATH BRANCH HEAD MISSING
  W_PATH="$1" W_BRANCH="$2" W_HEAD="$3" W_MISSING="$4"
  W_TARGETS="" W_TARGET_KIB=0 W_ACTIVITY="" W_IDLE_DAYS="" W_DIRTY="" W_UNPUSHED=""
  W_MERGED="" W_GONE=0 W_BUSY=0 W_CURRENT=0 W_SKIP="" W_ELIGIBLE=0

  if [ "$W_MISSING" = 1 ]; then
    W_SKIP="directory is gone (git worktree prune handles the registration)"
    return
  fi

  local here
  here="$(git -C "$PWD" rev-parse --show-toplevel 2>/dev/null || true)"
  [ -n "$here" ] && here="$(cd "$here" 2>/dev/null && pwd -P)"
  if [ "$here" = "$(cd "$W_PATH" 2>/dev/null && pwd -P)" ]; then W_CURRENT=1; fi

  W_TARGETS="$(find_targets "$W_PATH")"
  local t k
  while IFS= read -r t; do
    [ -n "$t" ] || continue
    k="$(du_kib "$t")"
    W_TARGET_KIB=$(( W_TARGET_KIB + ${k:-0} ))
  done <<EOF
$W_TARGETS
EOF

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
read_scratch() {
  if [ -d "$SCRATCH_DIR" ]; then scratch_kib="$(du_kib "$SCRATCH_DIR")"; fi
}

low_disk() {
  [ -n "$disk_pct_free" ] && awk -v p="$disk_pct_free" 'BEGIN { exit !(p < 15) }'
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
  echo "    target:       $(human "$W_TARGET_KIB")$( [ -z "$W_TARGETS" ] && echo ' (no cargo target dir)')"
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
  printf '],"target_kib":%s' "$W_TARGET_KIB"
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

report() {
  read_disk
  read_scratch
  if [ "$FORMAT" = json ]; then
    printf '{\n  "generated_at":%s,\n  "idle_days_threshold":%s,\n' "$NOW" "$IDLE_DAYS"
    printf '  "disk":{"total_kib":%s,"avail_kib":%s,"percent_free":%s,"under_15_percent":%s},\n' \
      "${disk_total_kib:-null}" "${disk_avail_kib:-null}" "${disk_pct_free:-null}" "$(low_disk && echo true || echo false)"
    printf '  "scratch":{"path":"%s","kib":%s},\n' "$(json_str "$SCRATCH_DIR")" "${scratch_kib:-null}"
    printf '  "worktrees":['
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
  if [ -d "$SCRATCH_DIR" ]; then
    echo "Session scratch ($SCRATCH_DIR): $(human "${scratch_kib:-0}") (report only; never cleaned by this script)"
  else
    echo "Session scratch ($SCRATCH_DIR): not present"
  fi
  echo
  echo "Registered worktrees:"
  for_each_worktree text_row
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
  echo "This runs only 'cargo clean' in the worktrees marked 'clean' and"
  echo "'git worktree prune'. It removes no worktree, branch or scratch directory."

  if [ -z "$PLAN" ] && [ -z "$prune_list" ]; then
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
  for_each_worktree apply_row
  echo "cargo clean ran on $CLEANED target dir(s); about $(human "$CLEANED_KIB") was in them."

  if [ -n "$prune_list" ]; then
    git -C "$REPO" worktree prune -v
  fi
  read_disk
  [ -n "$disk_avail_kib" ] && echo "Disk now: $(human "$disk_avail_kib") free (${disk_pct_free}% free). Time Machine local snapshots may hold freed space for a while (docs/agent/WORKFLOW_ROLES.md)."
}

case "$MODE" in
  report) report ;;
  clean) clean ;;
esac
