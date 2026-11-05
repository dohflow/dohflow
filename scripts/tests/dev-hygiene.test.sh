#!/usr/bin/env bash
#
# Tests for scripts/dev-hygiene.sh (personal-cfo-0rein.6).
#
# CI-safe: builds throwaway git repositories in a temp directory, puts a stub
# `cargo` on PATH that only records its arguments, and never touches a real
# worktree, branch or cargo target. No network, no bead database.
#
# (personal-cfo-0rein.12 adds a fixture scratch tree below: two DohFlow slugs and a
# foreign project, registered and unregistered checkouts, target dirs, a stub gh
# that answers merged / closed / open / unknown, and a running build.)
#
# The properties under test are the safety ones the script promises:
#   - the default (report) mode changes nothing and never runs cargo;
#   - --clean refuses a worktree with uncommitted changes, with unpushed
#     commits, with a running cargo process, that is recently active, or that it
#     is run from;
#   - --clean cleans a clean, pushed, idle worktree, with an explicit
#     --target-dir, and prunes a registration whose directory is gone;
#   - --clean without --yes and without a terminal does nothing;
#   - the script contains no `rm`, `unlink` of a user path, or `find -delete`.

# The `check && pass "…" || fail "…"` lines below are if-then-else on purpose:
# pass() always returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/dev-hygiene.sh"
[ -x "$SCRIPT" ] || { echo "missing or not executable: $SCRIPT" >&2; exit 1; }

TMP="$(cd "$(mktemp -d)" && pwd -P)"   # physical path: git and lsof report it
trap 'cd /; unlink "$TMP/.done" 2>/dev/null; chmod -R u+w "$TMP" 2>/dev/null; find "$TMP" -depth -mindepth 1 \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null; find "$TMP" -depth -type d -exec rmdir {} \; 2>/dev/null' EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $* (${SECONDS}s elapsed)"; }

export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.test
export GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.test
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null

# --- a stub cargo that only records what it was asked to do ----------------
BIN="$TMP/bin"; mkdir -p "$BIN"
CALLS="$TMP/cargo-calls"; : > "$CALLS"
cat > "$BIN/cargo" <<STUB
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$CALLS"
STUB
chmod +x "$BIN/cargo"
# --- a stub gh that answers from canned files and records every call ---------
# gh pr list ... --head BRANCH    -> $GHDATA/pr-BRANCH   (tab-separated: number, STATE, head sha)
# gh api repos/O/R/commits/SHA/pulls -> $GHDATA/api-SHA (number, state, merged_at, merge_commit_sha)
# A data file holding the single word FAIL makes the call exit 1 (no network).
# A missing file prints nothing: no pull request.
GHDATA="$TMP/ghdata"; mkdir -p "$GHDATA"
GHCALLS="$TMP/gh-calls"; : > "$GHCALLS"
cat > "$BIN/gh" <<STUB
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$GHCALLS"
key=""
if [ "\$1" = pr ] && [ "\$2" = list ]; then
  shift 2
  while [ \$# -gt 0 ]; do [ "\$1" = --head ] && key="pr-\$2"; shift; done
elif [ "\$1" = api ]; then
  sha="\${2#repos/*/*/commits/}"; key="api-\${sha%/pulls}"
fi
[ -n "\$key" ] && [ -f "$GHDATA/\$key" ] || exit 0
if [ "\$(cat "$GHDATA/\$key")" = FAIL ]; then exit 1; fi
cat "$GHDATA/\$key"
STUB
chmod +x "$BIN/gh"
# --- a stub df that can report a nearly full disk (HYGTEST_LOW_DISK=1) -------
REALDF="$(command -v df)"
cat > "$BIN/df" <<STUB
#!/usr/bin/env bash
case "\${HYGTEST_LOW_DISK:-}" in
  1) printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\nx 1000000 950000 50000 95%% /\n' ;;
  0) printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\nx 1000000 200000 800000 20%% /\n' ;;
  *) exec "$REALDF" "\$@" ;;
esac
STUB
chmod +x "$BIN/df"
export PATH="$BIN:$PATH"
export HYGTEST_LOW_DISK=0   # the real disk may be full; the cases that need a low disk say so

# --- a repository with an origin and several worktrees ---------------------
ORIGIN="$TMP/origin.git"
MAIN="$TMP/main"
git init -q --bare "$ORIGIN"
git init -q -b main "$MAIN"
(
  cd "$MAIN" || exit 1
  printf 'target/\n.claude/worktrees/\n' > .gitignore
  printf '[package]\nname = "x"\nversion = "0.0.0"\n' > Cargo.toml
  git add -A && git commit -q -m init
  git remote add origin "$ORIGIN"
  git push -q -u origin main
)

make_wt() {
  # make_wt NAME: a pushed branch checked out in its own worktree, with a fake
  # cargo target dir whose timestamps are 2020 (idle).
  local name="$1" wt="$TMP/wt-$1"
  git -C "$MAIN" worktree add -q -b "br-$name" "$wt" main
  git -C "$wt" push -q -u origin "br-$name" 2>/dev/null
  mkdir -p "$wt/target/debug/deps"
  : > "$wt/target/CACHEDIR.TAG"
  : > "$wt/target/debug/deps/lib.rlib"
  age_wt "$wt" 202001010000
}

age_wt() {
  # age_wt WT STAMP: set every timestamp the script looks at to STAMP.
  local wt="$1" stamp="$2" gitdir
  gitdir="$(git -C "$wt" rev-parse --absolute-git-dir)"
  touch -t "$stamp" "$gitdir/index" "$gitdir/HEAD" "$gitdir/logs/HEAD" 2>/dev/null
  find "$wt/target" -exec touch -t "$stamp" {} \; 2>/dev/null
}

make_wt idle
make_wt dirty
make_wt unpushed
make_wt fresh
make_wt busy
make_wt gone
make_wt outer
# nested: a linked worktree INSIDE the eligible outer one (as .claude/worktrees
# are inside the primary checkout), dirty, with a cargo target of its own
NESTED="$TMP/wt-outer/.claude/worktrees/n1"
mkdir -p "$TMP/wt-outer/.claude/worktrees"
git -C "$MAIN" worktree add -q -b br-n1 "$NESTED" main
mkdir -p "$NESTED/target/debug/deps"
: > "$NESTED/target/CACHEDIR.TAG"
: > "$NESTED/target/debug/deps/lib.rlib"
printf 'edit\n' >> "$NESTED/Cargo.toml"
age_wt "$NESTED" 202001010000
age_wt "$TMP/wt-outer" 202001010000

# dirty: an uncommitted edit (made, then everything re-aged so only the dirty
# check can refuse it)
printf 'edit\n' >> "$TMP/wt-dirty/Cargo.toml"
age_wt "$TMP/wt-dirty" 202001010000
# unpushed: a commit that is on no remote
( cd "$TMP/wt-unpushed" && printf 'x\n' > new.txt && git add new.txt && git commit -q -m local-only )
age_wt "$TMP/wt-unpushed" 202001010000
# fresh: target dir touched now (recent activity)
touch "$TMP/wt-fresh/target/debug/deps/lib.rlib" "$TMP/wt-fresh/target/debug/deps"
# busy: a process named cargo whose cwd is inside the worktree. The script is told to treat the
# name "hygtest-cargo" as "a build is running". Two ways to make a process with that name:
#  1. a symlink to sleep with a unique name (a copied binary would be killed by macOS signing).
#     This fails where `sleep` is a multicall binary that dispatches on argv[0] (the Rust
#     uutils coreutils of Ubuntu 26.04): `hygtest-cargo` is then an unknown applet and exits.
#  2. a tiny shell script of that name that runs sleep as a child. On Linux a script's process
#     name is the script's file name, so `pgrep -x hygtest-cargo` finds it.
# The test tries 1, checks that the process really appears under that name, and falls back to 2.
# HYGTEST_FORCE_SCRIPT=1 forces 2 (Linux only: macOS names a script's process after its
# interpreter, so the fallback is never needed or usable there).
mkdir -p "$TMP/busybin"
export DEV_HYGIENE_PROCESS_NAMES="hygtest-cargo"
start_busy() {
  ( cd "$TMP/wt-busy" && exec "$TMP/busybin/hygtest-cargo" 120 ) >/dev/null 2>&1 </dev/null &
  BUSY_PID=$!
  sleep 1
}
stop_busy() { pkill -P "$BUSY_PID" 2>/dev/null; kill "$BUSY_PID" 2>/dev/null; wait "$BUSY_PID" 2>/dev/null; }
BUSY_PID=""
if [ "${HYGTEST_FORCE_SCRIPT:-0}" != 1 ]; then
  ln -s "$(command -v sleep)" "$TMP/busybin/hygtest-cargo"
  start_busy
  if ! pgrep -x hygtest-cargo >/dev/null 2>&1; then
    stop_busy
    unlink "$TMP/busybin/hygtest-cargo"
  fi
fi
if [ ! -e "$TMP/busybin/hygtest-cargo" ]; then
  # shellcheck disable=SC2016  # the script's own $1, not ours
  printf '#!/bin/sh\nsleep "$1"\n' > "$TMP/busybin/hygtest-cargo"
  chmod +x "$TMP/busybin/hygtest-cargo"
  start_busy
fi
if ! pgrep -x hygtest-cargo >/dev/null 2>&1; then
  echo "    FAIL — test setup: no process named hygtest-cargo is running (the busy-worktree cases cannot be tested here)" >&2
  exit 1
fi
# gone: registered, directory removed behind git's back
GONE_DIR="$TMP/wt-gone"
find "$GONE_DIR" -depth \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null
find "$GONE_DIR" -depth -type d -exec rmdir {} \; 2>/dev/null

# shellcheck disable=SC2329  # invoked by the EXIT trap below
cleanup_busy() { stop_busy; }
trap 'cleanup_busy; cd /; chmod -R u+w "$TMP" 2>/dev/null; find "$TMP" -depth -mindepth 1 \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null; find "$TMP" -depth -type d -exec rmdir {} \; 2>/dev/null' EXIT

run() { ( cd "$TMP" && "$SCRIPT" --repo "$MAIN" --scratch-dir "$TMP/no-scratch" "$@" ); }

snapshot() {
  # A fingerprint of everything --report must not change.
  {
    git -C "$MAIN" worktree list --porcelain
    find "$TMP" -path "$TMP/bin" -prune -o -path "$TMP/origin.git" -prune -o -type f -print 2>/dev/null | sort
    git -C "$MAIN" for-each-ref
  } | cksum
}

# ---------------------------------------------------------------------------
case_start "the default mode is a read-only report"
before="$(snapshot)"
out="$(run 2>&1)"; rc=$?
after="$(snapshot)"
[ "$rc" = 0 ] && pass "exits 0" || fail "exit status $rc"
[ "$before" = "$after" ] && pass "changes nothing (worktrees, files, refs identical)" || fail "report changed something"
[ ! -s "$CALLS" ] && pass "never ran cargo" || fail "report ran cargo: $(cat "$CALLS")"
grep -q "Nothing was changed" <<<"$out" && pass "says nothing was changed" || fail "missing the read-only line"
grep -q "wt-idle" <<<"$out" && pass "lists the worktrees" || fail "idle worktree not listed"
grep -q "UNCOMMITTED CHANGES" <<<"$out" && pass "flags the dirty worktree" || fail "dirty worktree not flagged"
grep -q "1 commit(s) on no remote" <<<"$out" && pass "flags the unpushed commit" || fail "unpushed commit not flagged"
grep -q "directory is gone" <<<"$out" && pass "flags the registration whose directory is gone" || fail "missing-directory registration not flagged"
grep -q "Disk:" <<<"$out" && pass "reports disk space" || fail "no disk line"

case_start "--json is valid JSON and agrees with the text report"
if command -v jq >/dev/null 2>&1; then
  json="$(run --json 2>&1)"
  if printf '%s' "$json" | jq -e . >/dev/null 2>&1; then
    pass "valid JSON"
  else
    fail "invalid JSON: $json"
  fi
  [ "$(printf '%s' "$json" | jq -r '[.worktrees[] | select(.path|endswith("wt-idle")) | .clean_eligible] | first')" = "true" ] && pass "idle worktree is eligible" || fail "idle worktree not eligible in JSON"
  [ "$(printf '%s' "$json" | jq -r '[.worktrees[] | select(.path|endswith("wt-dirty")) | .dirty] | first')" = "true" ] && pass "dirty worktree is dirty in JSON" || fail "dirty flag wrong in JSON"
  [ "$(printf '%s' "$json" | jq -r '[.worktrees[] | select(.path|endswith("wt-unpushed")) | .unpushed_commits] | first')" = "1" ] && pass "unpushed count is 1 in JSON" || fail "unpushed count wrong in JSON"
  [ "$(printf '%s' "$json" | jq -r '[.worktrees[] | select(.path|endswith("wt-gone")) | .directory_missing] | first')" = "true" ] && pass "gone directory flagged in JSON" || fail "directory_missing wrong in JSON"
else
  echo "    skip — jq not installed"
fi

case_start "--clean refuses every unsafe worktree and cleans only the safe one"
: > "$CALLS"
out="$(run --clean --yes 2>&1)"; rc=$?
[ "$rc" = 0 ] && pass "exits 0" || fail "exit status $rc: $out"
n="$(wc -l < "$CALLS" | tr -d ' ')"
[ "$n" = 2 ] && pass "ran cargo clean exactly twice (the idle and the outer worktree)" || fail "ran cargo $n times: $(cat "$CALLS")"
grep -q -- "--target-dir $TMP/wt-outer/target" "$CALLS" && pass "cleaned the outer worktree's own target" || fail "outer worktree not cleaned: $(cat "$CALLS")"
if grep -q "worktrees/n1" "$CALLS"; then fail "cleaned the DIRTY nested worktree's target through its outer worktree: $(cat "$CALLS")"; else pass "did not descend into the nested worktree"; fi
grep -q "worktrees/n1.*uncommitted changes" <<<"$out" && pass "the plan skips the dirty nested worktree" || fail "nested worktree not shown as skipped: $out"
grep -q -- "--target-dir $TMP/wt-idle/target" "$CALLS" && pass "cleaned the idle worktree, with an explicit --target-dir" || fail "wrong or no clean: $(cat "$CALLS")"
grep -q -- "--manifest-path $TMP/wt-idle/Cargo.toml" "$CALLS" && pass "passed the manifest beside the target dir" || fail "no --manifest-path"
grep -q "wt-dirty.*uncommitted changes" <<<"$out" && pass "refused the dirty worktree" || fail "dirty worktree not refused: $out"
grep -q "wt-unpushed.*no remote" <<<"$out" && pass "refused the unpushed worktree" || fail "unpushed worktree not refused"
grep -q "wt-fresh.*active" <<<"$out" && pass "skipped the recently active worktree" || fail "fresh worktree not skipped"
grep -q "wt-busy.*cargo/rustc process" <<<"$out" && pass "skipped the worktree with a running cargo process" || fail "busy worktree not skipped"
grep -q "pruned\|Removing" <<<"$out" && pass "pruned the registration whose directory is gone" || fail "gone registration not pruned"
grep -q "wt-gone" <<<"$(git -C "$MAIN" worktree list --porcelain)" && fail "wt-gone is still registered" || pass "wt-gone is no longer registered"

case_start "--clean removes no worktree, branch or file of the user"
for name in idle dirty unpushed fresh busy; do
  [ -d "$TMP/wt-$name" ] && pass "wt-$name directory still exists" || fail "wt-$name directory was removed"
done
for name in idle dirty unpushed fresh busy gone; do
  git -C "$MAIN" rev-parse --verify -q "refs/heads/br-$name" >/dev/null && pass "branch br-$name still exists" || fail "branch br-$name was removed"
done
[ -f "$TMP/wt-dirty/Cargo.toml" ] && grep -q edit "$TMP/wt-dirty/Cargo.toml" && pass "the uncommitted edit is intact" || fail "uncommitted edit was lost"
grep -q local-only <<<"$(git -C "$TMP/wt-unpushed" log --oneline -1)" && pass "the unpushed commit is intact" || fail "unpushed commit lost"

case_start "--clean never touches the worktree it is run from"
: > "$CALLS"
out="$( cd "$TMP/wt-idle" && "$SCRIPT" --repo "$MAIN" --scratch-dir "$TMP/no-scratch" --clean --yes 2>&1 )"
if grep -q -- "--target-dir $TMP/wt-idle/target" "$CALLS"; then fail "cleaned the worktree it was run from"; else pass "did not clean its own worktree"; fi
grep -q "wt-idle.*runs from" <<<"$out" && pass "said why" || fail "no explanation for skipping the current worktree"

case_start "--clean needs a terminal or --yes"
: > "$CALLS"
out="$(run --clean </dev/null 2>&1)"; rc=$?
[ "$rc" != 0 ] && pass "exits non-zero without --yes and without a terminal" || fail "exit status $rc"
[ ! -s "$CALLS" ] && pass "ran nothing" || fail "ran cargo without confirmation: $(cat "$CALLS")"

case_start "options are validated"
run --idle-days abc >/dev/null 2>&1 && fail "accepted --idle-days abc" || pass "rejects a non-numeric --idle-days"
run --yes >/dev/null 2>&1 && fail "accepted --yes without --clean" || pass "rejects --yes without --clean"
run --clean --json >/dev/null 2>&1 && fail "accepted --clean --json" || pass "rejects --clean with --json"
run --bogus >/dev/null 2>&1 && fail "accepted an unknown option" || pass "rejects an unknown option"

case_start "--clean re-inspects right before cleaning (the prompt can wait)"
HOOK="$TMP/hook.sh"
printf '#!/bin/sh\nprintf "late edit\\n" >> "%s/Cargo.toml"\n' "$TMP/wt-idle" > "$HOOK"
chmod +x "$HOOK"
: > "$CALLS"
out="$(DEV_HYGIENE_TESTING=1 DEV_HYGIENE_AFTER_CONFIRM_HOOK="$HOOK" run --clean --yes 2>&1)"
if grep -q -- "--target-dir $TMP/wt-idle/target" "$CALLS"; then fail "cleaned a worktree that became dirty after the plan: $(cat "$CALLS")"; else pass "did not clean the worktree that became dirty after the plan"; fi
grep -q "wt-idle — changed since the plan" <<<"$out" && pass "said it changed since the plan" || fail "no 'changed since the plan' message: $out"
grep -q -- "--target-dir $TMP/wt-outer/target" "$CALLS" && pass "still cleaned the unchanged outer worktree" || fail "outer worktree not cleaned"

case_start "a long idle threshold cleans nothing"
: > "$CALLS"
out="$(run --clean --yes --idle-days 100000 2>&1)"
[ ! -s "$CALLS" ] && pass "ran no cargo clean" || fail "cleaned despite a huge threshold: $(cat "$CALLS")"

# ===========================================================================
# Session scratch (personal-cfo-0rein.12)
# ===========================================================================
# A scratch root with two DohFlow slugs per lane checkout and a foreign project.
# Its worktrees are registered in a SECOND repository, so the repository cases
# above stay about $MAIN. Its origin URL reads https://github.com/dohflow/dohflow
# (rewritten to a local bare repository for pushes), which is what the script
# hands to gh.
SORIGIN="$TMP/sorigin.git"
SMAIN="$TMP/smain"
git init -q --bare "$SORIGIN"
git init -q -b main "$SMAIN"
(
  cd "$SMAIN" || exit 1
  printf 'target/\nsecret.local\n' > .gitignore
  printf '[package]\nname = "x"\nversion = "0.0.0"\n' > Cargo.toml
  git add -A && git commit -q -m init
  git remote add origin https://github.com/dohflow/dohflow.git
  git config url."$SORIGIN".insteadOf https://github.com/dohflow/dohflow.git
  git push -q -u origin main
)

# The scratch cases run against a repository of their own with no worktrees, so
# the repository part of each run costs nothing; the scratch tree is the subject.
TINY="$TMP/tiny"
git init -q -b main "$TINY"
( cd "$TINY" && printf 'x\n' > f && git add f && git commit -q -m init )

SCR="$TMP/scratch"
PFX="-tmp-x-Programs"
S_OLD="$SCR/$PFX-dohflow-review-b/old-1111"         # idle, many worktrees
S_BUSY="$SCR/$PFX-dohflow-review-b/busy-3333"       # idle, but a build runs in it
S_RECENT="$SCR/$PFX-dohflow/recent-2222"            # merged, but active now
S_CUR="$SCR/$PFX-dohflow-escalation-c/cur-4444"     # merged, idle, but the script runs from it
S_FOREIGN="$SCR/$PFX-trophal/for-5555"              # another project
mkdir -p "$S_OLD" "$S_BUSY" "$S_RECENT" "$S_CUR" "$S_FOREIGN"

sw() {
  # sw SESSION NAME PRSTATE [extra]: a registered worktree SESSION/NAME on its own
  # pushed branch with a cargo target dir. PRSTATE merged|closed|open|none|fail
  # sets what the stub gh says about its branch. "extra" adds a pushed commit, so
  # the branch is NOT an ancestor of main (as after a squash merge).
  local s="$1" n="$2" pr="$3" wt="$1/$2" br st
  br="sc-$(basename "$1")-$2"
  git -C "$SMAIN" worktree add -q -b "$br" "$wt" main
  if [ "${4:-}" = extra ]; then
    ( cd "$wt" && printf '%s\n' "$n" > "change-$n.txt" && git add . && git commit -q -m "work $n" )
  fi
  git -C "$wt" push -q -u origin "$br" 2>/dev/null
  mkdir -p "$wt/target/debug/deps"
  : > "$wt/target/CACHEDIR.TAG"
  : > "$wt/target/debug/deps/lib.rlib"
  case "$pr" in merged) st=MERGED ;; closed) st=CLOSED ;; open) st=OPEN ;; *) st="" ;; esac
  if [ -n "$st" ]; then
    printf '7\t%s\t%s\n' "$st" "$(git -C "$wt" rev-parse HEAD)" > "$GHDATA/pr-$br"
  elif [ "$pr" = fail ]; then
    printf 'FAIL\n' > "$GHDATA/pr-$br"
  fi
}

age_session() {
  # age_session DIR: every timestamp the script reads, in the folder and in the
  # git admin directories of the worktrees in it, set to 2020.
  local s="$1" w
  for w in "$s"/*/; do
    [ -e "${w}.git" ] && age_wt "${w%/}" 202001010000
  done
  find "$s" -exec touch -t 202001010000 {} + 2>/dev/null
}

sw "$S_OLD" wt-merged merged extra
sw "$S_OLD" wt-closed closed extra
sw "$S_OLD" wt-open open extra
sw "$S_OLD" wt-unknown none           # a branch at main's tip: merged by ancestry, unknown by PR
sw "$S_OLD" wt-gherr fail extra       # gh fails, as with no network
sw "$S_OLD" wt-dirty merged extra
sw "$S_OLD" wt-unpushed merged extra
sw "$S_OLD" wt-ignored merged extra
sw "$S_OLD" wt-late merged extra
sw "$S_OLD" wt-flip merged extra      # its PR turns open after the plan
sw "$S_OLD" wt-detm merged extra      # detached HEAD, found by SHA
sw "$S_OLD" wt-landing none           # detached at the commit a PR landed as
sw "$S_OLD" wt-moved merged extra     # its branch moved on after the PR merged
sw "$S_OLD" wt-badidx merged extra    # git status cannot read it
printf 'dirty\n' >> "$S_OLD/wt-dirty/Cargo.toml"
# the PR was merged at the head the stub recorded; the branch then gained a pushed commit,
# so the checkout is NOT the merged PR's work and its head is no ancestor of the PR's head
( cd "$S_OLD/wt-moved" && printf 'later\n' > later.txt && git add later.txt && git commit -q -m later && git push -q )
# a corrupt index makes git status fail, which the script must treat as dirty
printf 'not an index\n' > "$(git -C "$S_OLD/wt-badidx" rev-parse --absolute-git-dir)/index"
( cd "$S_OLD/wt-unpushed" && printf 'x\n' > local.txt && git add local.txt && git commit -q -m local-only )
printf '7\tMERGED\t%s\n' "$(git -C "$S_OLD/wt-unpushed" rev-parse HEAD)" > "$GHDATA/pr-sc-old-1111-wt-unpushed"
printf 'keep me\n' > "$S_OLD/wt-ignored/secret.local"
# detached worktrees: the stub gh answers the commits/SHA/pulls call, not pr list
git -C "$S_OLD/wt-detm" checkout -q --detach
DETM_SHA="$(git -C "$S_OLD/wt-detm" rev-parse HEAD)"
rm_data() { unlink "$GHDATA/$1" 2>/dev/null; }
rm_data "pr-sc-old-1111-wt-detm"
printf '9\tclosed\t2026-01-01T00:00:00Z\tffff000000000000000000000000000000000000\n' > "$GHDATA/api-$DETM_SHA"
git -C "$S_OLD/wt-landing" checkout -q --detach
LAND_SHA="$(git -C "$S_OLD/wt-landing" rev-parse HEAD)"
printf '8\tclosed\t2026-01-01T00:00:00Z\t%s\n' "$LAND_SHA" > "$GHDATA/api-$LAND_SHA"
# an unregistered clone of the repository, with a merged PR and a target dir, and
# a loose cargo target dir that belongs to no checkout
git clone -q "$SORIGIN" "$S_OLD/copy-clone"
git -C "$S_OLD/copy-clone" checkout -q -b sc-copy
printf 'copy\n' > "$S_OLD/copy-clone/c.txt"; git -C "$S_OLD/copy-clone" add c.txt
git -C "$S_OLD/copy-clone" commit -q -m copy
git -C "$S_OLD/copy-clone" push -q origin sc-copy 2>/dev/null
git -C "$S_OLD/copy-clone" remote set-url origin https://github.com/dohflow/dohflow.git
git -C "$S_OLD/copy-clone" config url."$SORIGIN".insteadOf https://github.com/dohflow/dohflow.git
printf '5\tMERGED\t%s\n' "$(git -C "$S_OLD/copy-clone" rev-parse HEAD)" > "$GHDATA/pr-sc-copy"
mkdir -p "$S_OLD/copy-clone/target/debug" "$S_OLD/loose/target/debug"
: > "$S_OLD/copy-clone/target/CACHEDIR.TAG"; : > "$S_OLD/loose/target/CACHEDIR.TAG"
: > "$S_OLD/copy-clone/target/debug/a.rlib"; : > "$S_OLD/loose/target/debug/a.rlib"
printf 'x\n' > "$S_OLD/loose/Cargo.toml"
mkdir -p "$S_OLD/scratchpad"; printf 'note\n' > "$S_OLD/scratchpad/notes.txt"

sw "$S_BUSY" wt-b merged extra
sw "$S_RECENT" wt-r merged extra
sw "$S_FOREIGN" wt-f merged extra
for s in "$S_OLD" "$S_BUSY" "$S_FOREIGN"; do age_session "$s"; done
# S_RECENT stays active: its worktree was just made
# a build running inside S_BUSY (a second process named like the first one)
( cd "$S_BUSY" && exec "$TMP/busybin/hygtest-cargo" 120 ) >/dev/null 2>&1 </dev/null &
BUSY2_PID=$!
sleep 1
age_session "$S_BUSY"
# shellcheck disable=SC2329  # invoked by the EXIT trap below
cleanup_busy() { stop_busy; pkill -P "$BUSY2_PID" 2>/dev/null; kill "$BUSY2_PID" 2>/dev/null; wait "$BUSY2_PID" 2>/dev/null; }

scratch_calls() { grep -- "--target-dir $SCR/" "$CALLS"; }
run_s() { ( cd "$TMP" && "$SCRIPT" --repo "$TINY" --scratch-dir "$SCR" "$@" ); }
scratch_fp() {
  # Everything under the scratch root that --report must not change: names,
  # contents, timestamps and the git state of the worktrees.
  {
    find "$SCR" -type f -exec cksum {} + 2>/dev/null | sort
    if [ "$(uname)" = Darwin ]; then find "$SCR" -exec stat -f '%N %m' {} + 2>/dev/null | sort
    else find "$SCR" -exec stat -c '%n %Y' {} + 2>/dev/null | sort; fi
    git -C "$SMAIN" worktree list --porcelain
    git -C "$SMAIN" for-each-ref
  } | cksum
}

# Debugging aid: build the fixture, print where it is, and stop. The caller owns the cleanup.
if [ "${HYGTEST_STOP_AFTER_FIXTURE:-}" = 1 ]; then
  echo "fixture: $TMP (scratch root $SCR, repository $MAIN)"
  trap - EXIT
  exit 0
fi

# ---------------------------------------------------------------------------
case_start "--report lists each DohFlow session folder, what is inside it, and PR state"
before="$(scratch_fp)"; : > "$CALLS"
out="$(run_s --report 2>&1)"; rc=$?
after="$(scratch_fp)"
[ "$rc" = 0 ] && pass "exits 0" || fail "exit status $rc: $out"
[ "$before" = "$after" ] && pass "changed nothing under the scratch root or in its repository" || fail "the report changed the scratch tree"
[ ! -s "$CALLS" ] && pass "ran no cargo" || fail "the report ran cargo: $(cat "$CALLS")"
for sess in old-1111 busy-3333 recent-2222 cur-4444; do
  grep -q "/ $sess " <<<"$out" && pass "lists session $sess" || fail "session $sess missing: $out"
done
grep -q "dohflow-review-b / old-1111" <<<"$out" && grep -q "dohflow-escalation-c / cur-4444" <<<"$out" && pass "names the lane checkout (slug) of each session" || fail "slug not shown"
grep -q "wt-merged.*\[registered\]" <<<"$out" && pass "marks a registered worktree" || fail "registered worktree not marked"
grep -q "copy-clone.*\[unregistered\]" <<<"$out" && pass "marks an unregistered copy" || fail "unregistered copy not marked"
grep -q "sc-old-1111-wt-merged; clean; 0 unpushed; PR #7 MERGED\|sc-old-1111-wt-merged; clean; 0 unpushed; PR #7 merged" <<<"$out" && pass "shows branch, state, unpushed count and the merged PR" || fail "merged worktree line wrong: $out"
grep -q "sc-old-1111-wt-dirty; UNCOMMITTED CHANGES" <<<"$out" && pass "flags uncommitted changes in a scratch worktree" || fail "dirty scratch worktree not flagged"
grep -q "sc-old-1111-wt-unpushed; .*; 1 unpushed" <<<"$out" && pass "counts unpushed commits in a scratch worktree" || fail "unpushed scratch commit not counted"
grep -q "PR #7 closed" <<<"$out" && pass "shows a closed PR" || fail "closed PR not shown"
grep -q "PR #7 open" <<<"$out" && pass "shows an open PR" || fail "open PR not shown"
grep -q "wt-merged/target" <<<"$out" && grep -q "loose/target" <<<"$out" && pass "lists cargo target dirs, including one in no checkout" || fail "target dirs missing"
grep -q "no owning checkout" <<<"$out" && pass "says when a target dir has no owning checkout" || fail "ownerless target not described"
grep -q "wt-detm; clean\|(detached at" <<<"$out" && pass "reports a detached HEAD" || fail "detached worktree not reported"
grep -q "PR #9 merged" <<<"$out" && pass "found a detached worktree's PR by commit SHA (closed with a merge date = merged)" || fail "detached worktree PR not found"

case_start "a squash merge counts as merged through the PR, and unknown is never merged"
# wt-unknown's branch sits at main's tip, so ancestry alone would call it merged.
grep -q "sc-old-1111-wt-unknown; clean; 0 unpushed; PR unknown" <<<"$out" && pass "a branch merged by ancestry only, with no PR, is unknown" || fail "ancestry made it merged: $out"
grep -q "sc-old-1111-wt-gherr; clean; 0 unpushed; PR unknown" <<<"$out" && pass "a failing gh gives unknown" || fail "gh failure not unknown"
grep -q "wt-landing; clean; 0 unpushed; PR unknown\|(detached at ${LAND_SHA:0:8}); clean; 0 unpushed; PR unknown" <<<"$out" && pass "a commit that is a PR's landing commit is not that PR's work: unknown" || fail "landing commit counted as the PR's work"
grep -q "sc-old-1111-wt-moved; clean; 0 unpushed; PR unknown" <<<"$out" && pass "a branch that moved past its merged PR's head is unknown: the checkout is not that PR's work" || fail "moved-on branch counted as merged: $out"
grep -q "status unreadable" <<<"$out" && pass "a checkout whose git status fails is reported as status unreadable, not clean" || fail "unreadable status not reported: $out"
# the PR state for a branch is asked with gh, never derived from ancestry
grep -q "pr list -R dohflow/dohflow --head sc-old-1111-wt-merged" "$GHCALLS" && pass "asked gh for the branch's PR" || fail "gh was not asked: $(cat "$GHCALLS")"
n_before="$(wc -l < "$GHCALLS" | tr -d ' ')"
out2="$(run_s --report --no-pr 2>&1)"
n_after="$(wc -l < "$GHCALLS" | tr -d ' ')"
if grep -E -q "unpushed; PR (#[0-9]+ )?merged" <<<"$out2"; then fail "--no-pr still reported a merged PR"; else pass "--no-pr: nothing is merged"; fi
[ "$n_before" = "$n_after" ] && pass "--no-pr makes no gh call" || fail "--no-pr called gh"

case_start "another project's folder is one size line and is never examined"
grep -q "Other projects under the scratch root" <<<"$out" && grep -Eq "[0-9.]+ [KMG]iB  trophal" <<<"$out" && pass "a size line for the foreign project" || fail "no size line for trophal: $out"
if grep -q "for-5555\|/wt-f/" <<<"$out"; then fail "the report opened the foreign folder"; else pass "nothing inside the foreign folder is listed"; fi
if grep -q "sc-for-5555-wt-f" "$GHCALLS"; then fail "gh was asked about a foreign worktree"; else pass "gh was never asked about the foreign worktree"; fi

case_start "--json carries the same facts"
if command -v jq >/dev/null 2>&1; then
  json="$(run_s --json 2>&1)"
  printf '%s' "$json" | jq -e . >/dev/null 2>&1 && pass "valid JSON" || fail "invalid JSON: $json"
  [ "$(printf '%s' "$json" | jq -r '[.scratch.sessions[] | select(.uuid=="old-1111") | .checkouts[] | select(.path|endswith("wt-merged")) | .pr_state] | first')" = merged ] && pass "merged PR in JSON" || fail "pr_state wrong"
  [ "$(printf '%s' "$json" | jq -r '[.scratch.sessions[] | select(.uuid=="old-1111") | .checkouts[] | select(.path|endswith("copy-clone")) | .registered] | first')" = false ] && pass "unregistered copy in JSON" || fail "registered flag wrong"
  [ "$(printf '%s' "$json" | jq -r '[.scratch.sessions[] | select(.uuid=="old-1111") | .checkouts[] | select(.path|endswith("wt-unknown")) | .pr_state] | first')" = unknown ] && pass "unknown in JSON" || fail "unknown not in JSON"
  [ "$(printf '%s' "$json" | jq -r '[.scratch.sessions[] | select(.uuid=="old-1111") | .targets[] | select(.path|endswith("loose/target")) | .owner] | first')" = null ] && pass "ownerless target has a null owner" || fail "owner wrong"
  [ "$(printf '%s' "$json" | jq -r '.scratch.other_projects | length')" = 1 ] && pass "one foreign project, size only" || fail "other_projects wrong"
  [ "$(printf '%s' "$json" | jq -r '[.scratch.tier2_candidates[] | select(.path|test("wt-merged"))] | length')" = 2 ] && pass "names wt-merged's cargo clean and worktree remove as candidates" || fail "tier2_candidates wrong"
  [ "$(printf '%s' "$json" | jq -r '[.scratch.sessions[] | select(.uuid=="busy-3333") | .cargo_running] | first')" = true ] && pass "a running build is reported" || fail "cargo_running wrong"
else
  echo "    skip — jq not installed"
fi

case_start "the report flags a session folder over the size line, and the scratch total when disk is low"
out3="$(DEV_HYGIENE_TESTING=1 DEV_HYGIENE_BIG_KIB=1 run_s --report 2>&1)"
grep -q "old-1111.*OVER" <<<"$out3" && pass "flags a session over the line" || fail "OVER flag missing"
if grep -q "OVER" <<<"$out"; then fail "flagged a small session by default (20 GiB line)"; else pass "no flag under 20 GiB"; fi
out4="$(HYGTEST_LOW_DISK=1 run_s --report 2>&1)"
grep -q "Free space is under 15% and scratch holds" <<<"$out4" && pass "flags the scratch total when free space is under 15%" || fail "low-disk scratch flag missing: $out4"
if grep -q "Free space is under 15% and scratch" <<<"$out"; then fail "low-disk flag shown with plenty of space"; else pass "no low-disk flag with plenty of space"; fi
grep -q "Safe to clean now under --clean --scratch" <<<"$out" && grep -q "cargo clean *.*wt-merged/target" <<<"$out" && pass "names the candidates the policy calls safe" || fail "candidates not named: $out"

case_start "--clean without --scratch leaves scratch alone"
: > "$CALLS"
out="$(run_s --clean --yes 2>&1)"; rc=$?
[ "$rc" = 0 ] && pass "exits 0" || fail "exit status $rc: $out"
if [ -n "$(scratch_calls)" ]; then fail "cleaned something under scratch without --scratch: $(scratch_calls)"; else pass "no scratch path was cleaned"; fi
[ -d "$S_OLD/wt-merged" ] && pass "no scratch worktree was removed" || fail "removed a scratch worktree without --scratch"
run_s --scratch >/dev/null 2>&1 && fail "accepted --scratch without --clean" || pass "--scratch needs --clean"

case_start "--clean --scratch re-inspects right before acting (the prompt can wait)"
HOOK2="$TMP/hook2.sh"
printf '#!/bin/sh\nprintf "late edit\\n" >> "%s/Cargo.toml"\ntouch -t 202001010000 "%s/Cargo.toml"\nprintf "7\\tOPEN\\t%s\\n" > "%s/pr-sc-old-1111-wt-flip"\n' "$S_OLD/wt-late" "$S_OLD/wt-late" "$(git -C "$S_OLD/wt-flip" rev-parse HEAD)" "$GHDATA" > "$HOOK2"
chmod +x "$HOOK2"
: > "$CALLS"
out="$(DEV_HYGIENE_TESTING=1 DEV_HYGIENE_AFTER_CONFIRM_HOOK="$HOOK2" run_s --clean --scratch --yes 2>&1)"
late_calls="$(scratch_calls)"
if grep -q "wt-late/target\|wt-flip/target" <<<"$late_calls"; then fail "cleaned a worktree that changed after the plan"; else pass "did not clean the worktree that became dirty, nor the one whose PR became open"; fi
[ -d "$S_OLD/wt-late" ] && [ -d "$S_OLD/wt-flip" ] && pass "did not remove either" || fail "removed a worktree that changed after the plan"
grep -q "wt-late.*changed since the plan" <<<"$out" && pass "said the dirty one changed since the plan" || fail "no 'changed since the plan' message for wt-late: $out"
grep -q "wt-flip.*changed since the plan" <<<"$out" && pass "said the one whose PR turned open changed since the plan" || fail "no 'changed since the plan' message for wt-flip: $out"
# the same run also did the real work: that is checked next
SCRATCH_RUN_OUT="$out"; SCRATCH_RUN_CALLS="$(scratch_calls)"

case_start "--clean --scratch acts only where every condition holds"
out="$SCRATCH_RUN_OUT"
for ok in wt-merged wt-closed wt-detm wt-ignored; do
  grep -q -- "--target-dir $S_OLD/$ok/target" <<<"$SCRATCH_RUN_CALLS" && pass "cargo clean ran in $ok" || fail "$ok not cleaned: $SCRATCH_RUN_CALLS"
done
grep -q -- "--manifest-path $S_OLD/wt-merged/Cargo.toml" <<<"$SCRATCH_RUN_CALLS" && pass "with the manifest beside the target dir" || fail "no --manifest-path"
for gone in wt-merged wt-closed wt-detm; do
  [ ! -d "$S_OLD/$gone" ] && pass "$gone was removed with git worktree remove" || fail "$gone still exists"
  grep -q "$S_OLD/$gone" <<<"$(git -C "$SMAIN" worktree list --porcelain)" && fail "$gone is still registered" || pass "$gone is no longer registered"
done
for br in merged closed detm ignored; do
  git -C "$SMAIN" rev-parse --verify -q "refs/heads/sc-old-1111-wt-$br" >/dev/null && pass "branch of wt-$br still exists" || fail "branch of wt-$br was removed"
done
[ -d "$S_OLD/wt-ignored" ] && [ -f "$S_OLD/wt-ignored/secret.local" ] && pass "a worktree holding other ignored files stays, with its file" || fail "wt-ignored or its ignored file was lost"
grep -q "wt-ignored.*cargo clean only" <<<"$out" && pass "the plan says the ignored-files worktree gets cargo clean only" || fail "wt-ignored not explained: $out"

case_start "--clean --scratch refuses everything else, and says why"
n="$(printf '%s\n' "$SCRATCH_RUN_CALLS" | grep -c -- "--target-dir")"
[ "$n" = 4 ] && pass "exactly four cargo clean calls (merged, closed, detached-merged, ignored)" || fail "$n cargo clean calls: $SCRATCH_RUN_CALLS"
refused() {
  # refused PATH PATTERN WHY
  if grep -q -- "--target-dir $1" <<<"$SCRATCH_RUN_CALLS"; then fail "cleaned $3: $1"; else pass "did not clean $3"; fi
  [ -e "$1" ] && pass "left $3 in place" || fail "removed $3: $1"
  grep -q "skip[a-z]* .*$2.*— " <<<"$out" && pass "explained the refusal for $3" || fail "no reason shown for $3: $out"
}
refused "$S_OLD/wt-open/target" "wt-open" "a worktree whose PR is open"
refused "$S_OLD/wt-unknown/target" "wt-unknown" "a worktree whose PR is unknown"
refused "$S_OLD/wt-gherr/target" "wt-gherr" "a worktree when gh failed"
refused "$S_OLD/wt-landing/target" "wt-landing" "a worktree at a PR's landing commit"
refused "$S_OLD/wt-moved/target" "wt-moved" "a worktree whose branch moved past its merged PR"
refused "$S_OLD/wt-badidx/target" "wt-badidx" "a worktree whose git status cannot be read"
refused "$S_OLD/wt-dirty/target" "wt-dirty" "a worktree with uncommitted changes"
refused "$S_OLD/wt-unpushed/target" "wt-unpushed" "a worktree with an unpushed commit"
refused "$S_OLD/wt-late/target" "wt-late" "a worktree that became dirty after the plan"
refused "$S_OLD/wt-flip/target" "wt-flip" "a worktree whose PR turned open after the plan"
refused "$S_OLD/copy-clone/target" "copy-clone" "an unregistered copy, even with a merged PR"
refused "$S_OLD/loose/target" "loose/target" "a target dir that belongs to no checkout"
refused "$S_RECENT/wt-r/target" "wt-r" "a recently active session folder"
refused "$S_BUSY/wt-b/target" "wt-b" "a session folder with a build running"
if grep -q -- "$S_FOREIGN" <<<"$SCRATCH_RUN_CALLS"; then fail "cleaned another project's folder"; else pass "did not clean another project's folder"; fi
[ -d "$S_FOREIGN/wt-f" ] && grep -q "$S_FOREIGN/wt-f" <<<"$(git -C "$SMAIN" worktree list --porcelain)" && pass "left another project's registered worktree in place" || fail "touched another project's worktree"
if grep -q "for-5555\|/wt-f/" <<<"$out"; then fail "the plan opened another project's folder"; else pass "the plan never mentions another project's folder"; fi
grep -q "unregistered copy" <<<"$out" && pass "the unregistered-copy reason names the owner-approval rule" || fail "no unregistered-copy reason"
grep -q "loose/target.*not inside a checkout" <<<"$out" && pass "a target with no owning checkout is refused for that reason, not by a fallback" || fail "ownerless target reason missing: $out"
grep -q "wt-moved.*pull request state is unknown" <<<"$out" && pass "the moved-on branch is refused because its PR does not count, not for another reason" || fail "moved-on branch reason wrong: $out"
grep -q "wt-badidx.*uncommitted changes (or status unreadable)" <<<"$out" && pass "an unreadable git status is refused as dirty, ahead of the PR state" || fail "unreadable status reason wrong: $out"
grep -q "pull request state is open" <<<"$out" && grep -q "pull request state is unknown" <<<"$out" && pass "the PR reasons name the state" || fail "PR reasons missing"
grep -q "uncommitted changes" <<<"$out" && grep -q "commits that exist on no remote" <<<"$out" && pass "dirty and unpushed reasons shown" || fail "dirty/unpushed reasons missing"
grep -q "cargo/rustc process is running" <<<"$out" && pass "the running-build reason shown" || fail "no running-build reason"
grep -q "active 0 day(s) ago" <<<"$out" && pass "the recent-activity reason shown" || fail "no recent-activity reason"
[ -f "$S_OLD/scratchpad/notes.txt" ] && pass "the session's own notes are untouched" || fail "scratchpad notes lost"
[ -f "$S_OLD/wt-dirty/Cargo.toml" ] && grep -q dirty "$S_OLD/wt-dirty/Cargo.toml" && pass "the uncommitted edit is intact" || fail "uncommitted edit lost"
unpushed_log="$(git -C "$S_OLD/wt-unpushed" log --oneline -1)"
grep -q local-only <<<"$unpushed_log" && pass "the unpushed commit is intact" || fail "unpushed commit lost"

case_start "--clean --scratch never touches the folder it is run from"
: > "$CALLS"
sw "$S_CUR" wt-c merged extra; age_session "$S_CUR"
out="$( cd "$S_CUR" && "$SCRIPT" --repo "$TINY" --scratch-dir "$SCR" --clean --scratch --yes 2>&1 )"
grep -q -- "--target-dir $S_CUR/wt-c/target" "$CALLS" && fail "cleaned the session folder it ran from" || pass "did not clean the session it ran from"
[ -d "$S_CUR/wt-c" ] && pass "did not remove its worktree" || fail "removed a worktree of the folder it ran from"
grep -q "runs from" <<<"$out" && pass "said why" || fail "no reason for the current folder: $out"

case_start "--clean --scratch with --no-pr or a long idle threshold does nothing"
: > "$CALLS"
sw "$S_OLD" wt-fresh2 merged extra; age_session "$S_OLD"
run_s --clean --scratch --yes --no-pr >/dev/null 2>&1
[ -z "$(scratch_calls)" ] && pass "--no-pr: every state is unknown, nothing is cleaned" || fail "cleaned with --no-pr: $(scratch_calls)"
[ -d "$S_OLD/wt-fresh2" ] && pass "--no-pr: nothing was removed" || fail "removed with --no-pr"
run_s --clean --scratch --yes --idle-days 100000 >/dev/null 2>&1
[ -z "$(scratch_calls)" ] && pass "a huge idle threshold: nothing is cleaned" || fail "cleaned despite the threshold: $(scratch_calls)"
[ -d "$S_OLD/wt-fresh2" ] && pass "a huge idle threshold: nothing was removed" || fail "removed despite the threshold"

case_start "--clean --scratch needs a terminal or --yes"
: > "$CALLS"
run_s --clean --scratch </dev/null >/dev/null 2>&1; rc=$?
[ "$rc" != 0 ] && pass "exits non-zero without --yes and without a terminal" || fail "exit status $rc"
[ -z "$(scratch_calls)" ] && [ -d "$S_OLD/wt-fresh2" ] && pass "ran nothing" || fail "acted without confirmation"

case_start "a gh that is not installed means unknown, never merged"
NOGH="$TMP/nogh"; mkdir -p "$NOGH"
for tool in bash sh git cargo awk sed tr sort tail head find du df stat date uname cat sleep ps pgrep lsof dirname basename wc grep mkdir id cksum touch readlink cut tee env; do
  src="$(command -v "$tool" 2>/dev/null)"
  [ -n "$src" ] && [ ! -e "$NOGH/$tool" ] && ln -s "$src" "$NOGH/$tool"
done
unlink "$NOGH/gh" 2>/dev/null
: > "$CALLS"
[ -z "$(PATH="$NOGH" command -v gh)" ] && pass "test setup: no gh on this PATH" || fail "test setup: gh is on the stripped PATH"
out="$( cd "$TMP" && PATH="$NOGH" "$SCRIPT" --repo "$TINY" --scratch-dir "$SCR" --clean --scratch --yes 2>&1 )"
[ -z "$(scratch_calls)" ] && [ -d "$S_OLD/wt-fresh2" ] && pass "cleaned nothing without gh" || fail "cleaned without gh: $(scratch_calls)"

case_start "the script cannot delete user files"
code="$(grep -v '^[[:space:]]*#' "$SCRIPT")"
if grep -E -q '(^|[^[:alnum:]_.-])rm([[:space:]]|$)' <<<"$code"; then fail "script calls rm"; else pass "no rm"; fi
if grep -E -q 'rmdir|-delete|shred|truncate|unlink' <<<"$code"; then fail "script calls a deleting command"; else pass "no rmdir, find -delete, shred, truncate or unlink"; fi
if grep -E -q 'git (branch|push|reset|checkout|clean|stash|gc)|worktree (move|lock|unlock|repair)|--force|git [^|;]* -f( |$)' <<<"$code"; then fail "script changes branches or history, or forces something"; else pass "no branch, push, reset, checkout, clean, stash, gc, move, --force or -f"; fi
n_remove="$(grep -c "worktree remove" <<<"$code")"
[ "$n_remove" = 1 ] && pass "git worktree remove appears exactly once" || fail "git worktree remove appears $n_remove times"
remove_line="$(grep 'worktree remove' <<<"$code")"
grep -q -- '--git-dir="\$common" worktree remove "\$a"; then$' <<<"$remove_line" && pass "and it is the plain call: no flags after remove" || fail "the worktree remove call is not the plain, unforced one"
bash -n "$SCRIPT" && pass "bash -n is clean" || fail "bash -n failed"

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
