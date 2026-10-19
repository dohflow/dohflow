#!/usr/bin/env bash
#
# Tests for scripts/dev-hygiene.sh (personal-cfo-0rein.6).
#
# CI-safe: builds throwaway git repositories in a temp directory, puts a stub
# `cargo` on PATH that only records its arguments, and never touches a real
# worktree, branch or cargo target. No network, no bead database.
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
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

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
export PATH="$BIN:$PATH"

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
# busy: a process named cargo whose cwd is inside the worktree
# (a symlink to sleep with a unique name; the script is told to treat that name
# as "a build is running". A copied binary would be killed by macOS signing.)
mkdir -p "$TMP/busybin"
ln -s "$(command -v sleep)" "$TMP/busybin/hygtest-cargo"
export DEV_HYGIENE_PROCESS_NAMES="hygtest-cargo"
( cd "$TMP/wt-busy" && exec "$TMP/busybin/hygtest-cargo" 120 ) >/dev/null 2>&1 </dev/null &
BUSY_PID=$!
sleep 1
# gone: registered, directory removed behind git's back
GONE_DIR="$TMP/wt-gone"
find "$GONE_DIR" -depth \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null
find "$GONE_DIR" -depth -type d -exec rmdir {} \; 2>/dev/null

cleanup_busy() { kill "$BUSY_PID" 2>/dev/null; wait "$BUSY_PID" 2>/dev/null; }
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
printf '%s' "$out" | grep -q "Nothing was changed" && pass "says nothing was changed" || fail "missing the read-only line"
printf '%s' "$out" | grep -q "wt-idle" && pass "lists the worktrees" || fail "idle worktree not listed"
printf '%s' "$out" | grep -q "UNCOMMITTED CHANGES" && pass "flags the dirty worktree" || fail "dirty worktree not flagged"
printf '%s' "$out" | grep -q "1 commit(s) on no remote" && pass "flags the unpushed commit" || fail "unpushed commit not flagged"
printf '%s' "$out" | grep -q "directory is gone" && pass "flags the registration whose directory is gone" || fail "missing-directory registration not flagged"
printf '%s' "$out" | grep -q "Disk:" && pass "reports disk space" || fail "no disk line"

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
printf '%s' "$out" | grep -q "worktrees/n1.*uncommitted changes" && pass "the plan skips the dirty nested worktree" || fail "nested worktree not shown as skipped: $out"
grep -q -- "--target-dir $TMP/wt-idle/target" "$CALLS" && pass "cleaned the idle worktree, with an explicit --target-dir" || fail "wrong or no clean: $(cat "$CALLS")"
grep -q -- "--manifest-path $TMP/wt-idle/Cargo.toml" "$CALLS" && pass "passed the manifest beside the target dir" || fail "no --manifest-path"
printf '%s' "$out" | grep -q "wt-dirty.*uncommitted changes" && pass "refused the dirty worktree" || fail "dirty worktree not refused: $out"
printf '%s' "$out" | grep -q "wt-unpushed.*no remote" && pass "refused the unpushed worktree" || fail "unpushed worktree not refused"
printf '%s' "$out" | grep -q "wt-fresh.*active" && pass "skipped the recently active worktree" || fail "fresh worktree not skipped"
printf '%s' "$out" | grep -q "wt-busy.*cargo/rustc process" && pass "skipped the worktree with a running cargo process" || fail "busy worktree not skipped"
printf '%s' "$out" | grep -q "pruned\|Removing" && pass "pruned the registration whose directory is gone" || fail "gone registration not pruned"
git -C "$MAIN" worktree list --porcelain | grep -q "wt-gone" && fail "wt-gone is still registered" || pass "wt-gone is no longer registered"

case_start "--clean removes no worktree, branch or file of the user"
for name in idle dirty unpushed fresh busy; do
  [ -d "$TMP/wt-$name" ] && pass "wt-$name directory still exists" || fail "wt-$name directory was removed"
done
for name in idle dirty unpushed fresh busy gone; do
  git -C "$MAIN" rev-parse --verify -q "refs/heads/br-$name" >/dev/null && pass "branch br-$name still exists" || fail "branch br-$name was removed"
done
[ -f "$TMP/wt-dirty/Cargo.toml" ] && grep -q edit "$TMP/wt-dirty/Cargo.toml" && pass "the uncommitted edit is intact" || fail "uncommitted edit was lost"
git -C "$TMP/wt-unpushed" log --oneline -1 | grep -q local-only && pass "the unpushed commit is intact" || fail "unpushed commit lost"

case_start "--clean never touches the worktree it is run from"
: > "$CALLS"
out="$( cd "$TMP/wt-idle" && "$SCRIPT" --repo "$MAIN" --scratch-dir "$TMP/no-scratch" --clean --yes 2>&1 )"
if grep -q -- "--target-dir $TMP/wt-idle/target" "$CALLS"; then fail "cleaned the worktree it was run from"; else pass "did not clean its own worktree"; fi
printf '%s' "$out" | grep -q "wt-idle.*runs from" && pass "said why" || fail "no explanation for skipping the current worktree"

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
out="$(DEV_HYGIENE_AFTER_CONFIRM_HOOK="$HOOK" run --clean --yes 2>&1)"
if grep -q -- "--target-dir $TMP/wt-idle/target" "$CALLS"; then fail "cleaned a worktree that became dirty after the plan: $(cat "$CALLS")"; else pass "did not clean the worktree that became dirty after the plan"; fi
printf '%s' "$out" | grep -q "wt-idle — changed since the plan" && pass "said it changed since the plan" || fail "no 'changed since the plan' message: $out"
grep -q -- "--target-dir $TMP/wt-outer/target" "$CALLS" && pass "still cleaned the unchanged outer worktree" || fail "outer worktree not cleaned"

case_start "a long idle threshold cleans nothing"
: > "$CALLS"
out="$(run --clean --yes --idle-days 100000 2>&1)"
[ ! -s "$CALLS" ] && pass "ran no cargo clean" || fail "cleaned despite a huge threshold: $(cat "$CALLS")"

case_start "the script cannot delete user files"
code="$(grep -v '^[[:space:]]*#' "$SCRIPT")"
if printf '%s\n' "$code" | grep -E -q '(^|[^[:alnum:]_.-])rm([[:space:]]|$)'; then fail "script calls rm"; else pass "no rm"; fi
if printf '%s\n' "$code" | grep -E -q 'rmdir|-delete|shred|truncate'; then fail "script calls a deleting command"; else pass "no rmdir, find -delete, shred or truncate"; fi
if printf '%s\n' "$code" | grep -E -q 'git (branch|worktree (remove|move))|git push|git reset|git checkout|git clean|git stash'; then fail "script changes branches, worktrees or history"; else pass "no branch, worktree remove, push, reset, checkout, clean or stash"; fi
bash -n "$SCRIPT" && pass "bash -n is clean" || fail "bash -n failed"

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
