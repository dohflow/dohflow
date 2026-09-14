#!/usr/bin/env bash
#
# Regression test for scripts/git-hooks/ — beads personal-cfo-apesm and
# personal-cfo-bb96v.
#
# WHAT IT COVERS
#   apesm moved this repo's hand-written git hook logic (branch protection,
#   AGENTS.md §16; the bead-graph mirror backup, ADR 0064) out of
#   .beads/hooks/pre-push (untracked, ADR 0064 — absent from a public clone)
#   into TRACKED scripts/git-hooks/pre-push. apesm's original design chained
#   to bd's own managed hook at .beads/hooks/pre-push when it existed —
#   bb96v found that `bd hooks install --beads` copies whatever file
#   core.hooksPath currently resolves to into ITS OWN generated
#   .beads/hooks/*, which made that chain-to-file step resolve to itself and
#   recurse forever. The fix: invoke `bd hooks run <name>` DIRECTLY, with no
#   file reference to .beads/hooks/ at all, plus a defense-in-depth
#   recursion guard. Behaviors that must hold:
#     (a) a push to main is refused via the tracked hook, with the existing
#         message
#     (b) `bd hooks run pre-push` is invoked directly when `bd` is on PATH,
#         and the hook is a clean no-op when it is not
#     (c) a mirror-backup failure warns on stderr but does not block the push
#     (d) all of the above still work from a LINKED WORKTREE
#     (e) the recursion guard (DOHFLOW_GIT_HOOK_ACTIVE) exits promptly
#         instead of re-running the hook's own logic, for pre-push and
#         pre-commit
#     (f) the bb96v REPRODUCTION SCENARIO ITSELF, for pre-push AND
#         pre-commit (the two failure modes differed — a forked recursion
#         for pre-push's plain-call chain, a single-process `exec` loop for
#         pre-commit's): a contaminated .beads/hooks/<name> (a
#         self-referential copy of the OLD, pre-fix tracked content —
#         exactly what `bd hooks install --beads` used to produce) sitting
#         on disk does not cause the NEW hook to hang or recurse, because
#         the new hook never looks at that file at all — and `bd hooks run`
#         still fires exactly once (not zero, not recursively). Run under a
#         watchdog so a regression fails loudly rather than hanging the
#         test suite.
#
# HOW IT IS TESTED
#   Entirely inside a temporary directory: a throwaway bare "product.git", a
#   real clone with the REAL, VERBATIM scripts/git-hooks/* files from this
#   repository installed via `git config core.hooksPath scripts/git-hooks`
#   (a relative path, exactly as this repo sets it), and — for case (d) — a
#   real linked worktree off that clone. Nothing touches the real
#   repository, the real ~/.beads-mirror, GitHub, the real `bd`, the real
#   bead database, or the network. HOME is redirected too. A deterministic
#   STUB `bd` (never the real binary) is placed on PATH per-case, only for
#   the specific push that needs it — never globally — so cases that must
#   prove a clean no-op with no `bd` on PATH at all stay that way by
#   default. The bb96v reproduction (case f) never runs the real
#   `bd hooks install --beads` — it recreates its documented OUTPUT (a
#   contaminated .beads/hooks/pre-push) by hand, which is both safe and
#   exactly what's being guarded against.
#
# RUN
#   bash scripts/tests/git-hooks.test.sh
#
#   KEEP_WORKDIR=1 leaves the temp tree in place for inspection.

set -uo pipefail   # deliberately not -e: every case must run and report.

# This test manipulates GIT_DIR indirectly via linked worktrees. Start from a
# clean git environment so a run from inside a hook, or from CI, cannot
# inherit one and skew results (same guard as scripts/tests/backup-beads.test.sh).
_inherited_env_vars="$(git rev-parse --local-env-vars 2>/dev/null || true)"
# shellcheck disable=SC2086
[ -n "$_inherited_env_vars" ] && unset $_inherited_env_vars

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
HOOKS_SRC="$REPO_ROOT/scripts/git-hooks"
[ -d "$HOOKS_SRC" ] || { echo "no scripts/git-hooks/ at $HOOKS_SRC" >&2; exit 1; }

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/git-hooks-test.XXXXXX")" || exit 1
cleanup() {
  if [ -n "${KEEP_WORKDIR:-}" ]; then
    echo "workdir kept: $WORKDIR"
    return
  fi
  # Guarded: only ever removes the directory this run created, under the
  # temp root, with the name this script chose (AGENTS.md §1).
  case "$WORKDIR" in
    */git-hooks-test.??????) rm -rf "$WORKDIR" ;;
    *) echo "refusing to remove unexpected workdir: $WORKDIR" >&2 ;;
  esac
}
trap cleanup EXIT

export HOME="$WORKDIR/home"
mkdir -p "$HOME"
export GIT_TERMINAL_PROMPT=0
export GIT_AUTHOR_NAME="git-hooks test"
export GIT_AUTHOR_EMAIL="test@example.invalid"
export GIT_COMMITTER_NAME="$GIT_AUTHOR_NAME"
export GIT_COMMITTER_EMAIL="$GIT_AUTHOR_EMAIL"
# `bd` deliberately left off the DEFAULT PATH — see header comment. A stub
# is prepended to PATH per-case, only where a case needs one.
BASE_PATH="/usr/bin:/bin:/usr/sbin:/sbin"
export PATH="$BASE_PATH"

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

# Deterministic `bd` stub. Recognizes only `bd hooks run <name>`, prints
# "BD-HOOKS-RUN-<name>" to stderr (so a test can tell it ran, and which
# hook name it was invoked with), and exits $BD_STUB_EXIT (default 0).
# Anything else is an error — this test never needs the real bd's other
# behavior, and a stub that silently accepted unexpected invocations would
# hide a real mismatch between what the hook sends and what's expected.
write_bd_stub() {  # <bin-dir>
  cat > "$1/bd" <<'BD_STUB_EOF'
#!/usr/bin/env sh
if [ "$1" = "hooks" ] && [ "$2" = "run" ]; then
  echo "BD-HOOKS-RUN-${3:-}" >&2
  exit "${BD_STUB_EXIT:-0}"
fi
echo "unexpected bd invocation: $*" >&2
exit 9
BD_STUB_EOF
  chmod +x "$1/bd"
}

# ---------------------------------------------------------------------------
# Fixture: bare "product.git" + a clone with the REAL scripts/git-hooks/*
# installed via a RELATIVE core.hooksPath, exactly as this repo configures it.
# ---------------------------------------------------------------------------
make_clone() {  # <case-dir>  -> writes $BARE $CLONE
  local case_dir="$1"
  BARE="$case_dir/product.git"
  CLONE="$case_dir/clone"
  git init --quiet --bare -b main "$BARE"
  git clone --quiet "$BARE" "$CLONE" 2>/dev/null

  mkdir -p "$CLONE/scripts/git-hooks"
  cp "$HOOKS_SRC"/* "$CLONE/scripts/git-hooks/"
  chmod +x "$CLONE"/scripts/git-hooks/*
  git -C "$CLONE" config core.hooksPath scripts/git-hooks

  printf '# throwaway\n' > "$CLONE/README.md"
  git -C "$CLONE" add -A
  git -C "$CLONE" commit --quiet -m "initial"
  git -C "$CLONE" push --quiet -u origin main
}

# Pushes <ref> from <repo-dir> (a clone or a linked worktree) after making a
# throwaway commit, capturing stdout/stderr separately. Sets $PUSH_RC,
# $STDOUT_LOG, $STDERR_LOG. <target> defaults to the same name as <ref>.
# Respects the CURRENT $PATH at call time, so a case can prepend a stub bin
# dir just for one push.
push_change() {  # <repo-dir> <case-dir> <local-branch> [<target-ref>]
  local repo_dir="$1" case_dir="$2" branch="$3" target="${4:-$3}"
  if [ "$branch" != "main" ]; then
    git -C "$repo_dir" checkout --quiet -b "$branch" 2>/dev/null \
      || git -C "$repo_dir" checkout --quiet "$branch"
  fi
  echo "change-$RANDOM" > "$repo_dir/file-$RANDOM.txt"
  git -C "$repo_dir" add -A
  git -C "$repo_dir" commit --quiet -m "change on $branch"

  STDOUT_LOG="$case_dir/push-stdout-$RANDOM.txt"
  STDERR_LOG="$case_dir/push-stderr-$RANDOM.txt"
  ( cd "$repo_dir" && git push origin "$branch:$target" ) \
    >"$STDOUT_LOG" 2>"$STDERR_LOG"
  PUSH_RC=$?
}

# ===========================================================================
case_start "(a) direct push to main is refused, with the existing message"
# ===========================================================================
CASE="$WORKDIR/case-a"; mkdir -p "$CASE"
make_clone "$CASE"

push_change "$CLONE" "$CASE" main
assert_eq "push to main exits 1" "$PUSH_RC" "1"
assert_contains "refusal message on stderr" \
  "$(cat "$STDERR_LOG")" "BLOCKED: direct push to 'main'"
assert_contains "override instructions on stderr" \
  "$(cat "$STDERR_LOG")" "ALLOW_PUSH_TO_MAIN=1 git push origin main"
assert_contains "cites AGENTS.md §16" "$(cat "$STDERR_LOG")" "AGENTS.md §16"

# Override is honored, and only skips this one guard.
push_change "$CLONE" "$CASE" main
ALLOW_PUSH_TO_MAIN=1 sh -c \
  "cd '$CLONE' && git push origin main:main" \
  >"$CASE/override-stdout.txt" 2>"$CASE/override-stderr.txt"
OVERRIDE_RC=$?
assert_eq "ALLOW_PUSH_TO_MAIN=1 lets the push through" "$OVERRIDE_RC" "0"
assert_not_contains "no BLOCKED message when overridden" \
  "$(cat "$CASE/override-stderr.txt")" "BLOCKED"

# Feature branches are never touched by the guard.
push_change "$CLONE" "$CASE" feature-ok feature-ok
assert_eq "push to a feature branch succeeds" "$PUSH_RC" "0"
assert_not_contains "no BLOCKED message for a feature branch" \
  "$(cat "$STDERR_LOG")" "BLOCKED"

# ===========================================================================
case_start "(b) bd hooks run pre-push is invoked directly when bd is on PATH, no-ops when not"
# ===========================================================================
CASE="$WORKDIR/case-b1"; mkdir -p "$CASE"
make_clone "$CASE"
# `bd` NOT on PATH at all — the direct invocation must be a silent no-op.
push_change "$CLONE" "$CASE" no-bd no-bd
assert_eq "push succeeds with no bd on PATH" "$PUSH_RC" "0"
assert_not_contains "bd was never invoked (no marker)" \
  "$(cat "$STDERR_LOG")" "BD-HOOKS-RUN-"

CASE="$WORKDIR/case-b2"; mkdir -p "$CASE"
make_clone "$CASE"
STUB_BIN="$CASE/bin"; mkdir -p "$STUB_BIN"
write_bd_stub "$STUB_BIN"
PATH="$STUB_BIN:$BASE_PATH" push_change "$CLONE" "$CASE" with-bd with-bd
assert_eq "push succeeds when bd hooks run exits 0" "$PUSH_RC" "0"
assert_contains "bd hooks run pre-push was invoked directly, no file involved" \
  "$(cat "$STDERR_LOG")" "BD-HOOKS-RUN-pre-push"

# bd hooks run exiting non-zero (a real hook failure) propagates and blocks
# the push — this is bd's own hook logic refusing, not a mirror-backup
# concern, so it SHOULD be able to block.
CASE="$WORKDIR/case-b3"; mkdir -p "$CASE"
make_clone "$CASE"
STUB_BIN="$CASE/bin"; mkdir -p "$STUB_BIN"
write_bd_stub "$STUB_BIN"
PATH="$STUB_BIN:$BASE_PATH" BD_STUB_EXIT=1 push_change "$CLONE" "$CASE" bd-refuses bd-refuses
assert_eq "push is refused when bd hooks run exits non-zero" "$PUSH_RC" "1"
assert_contains "bd was invoked before refusing" \
  "$(cat "$STDERR_LOG")" "BD-HOOKS-RUN-pre-push"

# bd hooks run exiting 3 (database not initialized) is treated as "continue
# without beads" — never blocks.
CASE="$WORKDIR/case-b4"; mkdir -p "$CASE"
make_clone "$CASE"
STUB_BIN="$CASE/bin"; mkdir -p "$STUB_BIN"
write_bd_stub "$STUB_BIN"
PATH="$STUB_BIN:$BASE_PATH" BD_STUB_EXIT=3 push_change "$CLONE" "$CASE" bd-no-db bd-no-db
assert_eq "push succeeds when bd reports 'database not initialized'" "$PUSH_RC" "0"
assert_contains "the 'skipping' message is shown" \
  "$(cat "$STDERR_LOG")" "database not initialized — skipping hook 'pre-push'"

# ===========================================================================
case_start "(c) mirror-backup: fails-loud when set up, fully silent when not"
# ===========================================================================
CASE="$WORKDIR/case-c"; mkdir -p "$CASE"
make_clone "$CASE"
mkdir -p "$CLONE/scripts"
cat > "$CLONE/scripts/backup-beads.sh" <<'BACKUP_EOF'
#!/usr/bin/env sh
echo "some diagnostic on stderr" >&2
exit 1
BACKUP_EOF
chmod +x "$CLONE/scripts/backup-beads.sh"

# Script present but NO .beads/ at all (a fresh clone, or the public clone,
# before any bead database exists here) — completely silent, not even the
# generic warning. This is the scenario the -d "./.beads" guard exists for:
# without it, backup-beads.sh's own "no .beads/" refusal would surface as a
# warning on every push, purely from having cloned the repo.
push_change "$CLONE" "$CASE" no-beads-dir no-beads-dir
assert_eq "push succeeds with no .beads/ present" "$PUSH_RC" "0"
# git's own push status ("To <url> ... * [new branch] ...") legitimately goes
# to stderr for every push; "silent" here means the mirror-backup machinery
# never spoke up, not that stderr is literally empty.
assert_not_contains "no mirror-backup warning — .beads/, script never invoked" \
  "$(cat "$STDERR_LOG")" "mirror backup failed"
assert_not_contains "backup-beads.sh's own refusal never printed either" \
  "$(cat "$STDERR_LOG")" "no .beads/"

# Script present AND .beads/ present, but the backup genuinely fails — warns,
# still does not block.
mkdir -p "$CLONE/.beads"
push_change "$CLONE" "$CASE" backup-fails backup-fails
assert_eq "push exits 0 even though the backup script failed" "$PUSH_RC" "0"
assert_contains "generic mirror-backup warning reached stderr" \
  "$(cat "$STDERR_LOG")" "mirror backup failed"

# BEADS_SKIP_MIRROR=1 skips the backup call entirely — no warning at all,
# even with .beads/ present and a script that would otherwise fail.
push_change "$CLONE" "$CASE" backup-skipped backup-skipped
BEADS_SKIP_MIRROR=1 sh -c \
  "cd '$CLONE' && git push origin backup-skipped:backup-skipped" \
  >"$CASE/skip-stdout.txt" 2>"$CASE/skip-stderr.txt"
SKIP_RC=$?
assert_eq "push succeeds with BEADS_SKIP_MIRROR=1" "$SKIP_RC" "0"
assert_not_contains "no mirror-backup warning when skipped" \
  "$(cat "$CASE/skip-stderr.txt")" "mirror backup failed"

# ===========================================================================
case_start "(d) the hook works from a linked worktree"
# ===========================================================================
CASE="$WORKDIR/case-d"; mkdir -p "$CASE"
make_clone "$CASE"
STUB_BIN="$CASE/bin"; mkdir -p "$STUB_BIN"
write_bd_stub "$STUB_BIN"

LINKED="$CASE/linked-wt"
git -C "$CLONE" worktree add --quiet -b linked-branch "$LINKED" main 2>"$CASE/wt-add-stderr.txt"
WT_ADD_RC=$?
assert_eq "git worktree add succeeds" "$WT_ADD_RC" "0"

# core.hooksPath is shared (not worktree-scoped) — confirm the linked
# worktree resolves the same relative value without being configured again.
LINKED_HOOKSPATH="$(git -C "$LINKED" config --get core.hooksPath 2>/dev/null || true)"
assert_eq "linked worktree inherits core.hooksPath" "$LINKED_HOOKSPATH" "scripts/git-hooks"

# Direct push to main FROM the linked worktree is still refused.
push_change "$LINKED" "$CASE" main
assert_eq "push to main from linked worktree exits 1" "$PUSH_RC" "1"
assert_contains "refusal message from linked worktree" \
  "$(cat "$STDERR_LOG")" "BLOCKED: direct push to 'main'"

# A non-main push from the linked worktree invokes bd directly too — no
# --git-common-dir resolution needed any more (there is no file to find),
# so this also proves the fix does not regress worktree behavior.
PATH="$STUB_BIN:$BASE_PATH" push_change "$LINKED" "$CASE" linked-feature linked-feature
assert_eq "non-main push from linked worktree succeeds" "$PUSH_RC" "0"
assert_contains "bd hooks run pre-push invoked directly from the linked worktree" \
  "$(cat "$STDERR_LOG")" "BD-HOOKS-RUN-pre-push"

# ===========================================================================
case_start "(e) recursion guard (DOHFLOW_GIT_HOOK_ACTIVE) exits promptly"
# ===========================================================================
# Defense in depth (personal-cfo-bb96v): if this exact hook is somehow
# already "active" in an ancestor process, it must not re-run ANY of its own
# logic — not the ref-capture, not the protect-main guard, not the
# mirror-backup call. Simulated by pre-setting the env var the guard checks;
# a real accidental recursion would set it the same way (export, inherited
# by the child).
for HOOK in pre-push pre-commit; do
  CASE="$WORKDIR/case-e-$HOOK"; mkdir -p "$CASE"
  make_clone "$CASE"
  STDOUT_LOG="$CASE/stdout.txt"; STDERR_LOG="$CASE/stderr.txt"
  ( cd "$CLONE" && DOHFLOW_GIT_HOOK_ACTIVE="$HOOK" \
      "./scripts/git-hooks/$HOOK" origin "https://example.invalid/x.git" \
      </dev/null >"$STDOUT_LOG" 2>"$STDERR_LOG" )
  RC=$?
  assert_eq "$HOOK: guard causes a prompt, successful exit" "$RC" "0"
  assert_contains "$HOOK: guard message on stderr" \
    "$(cat "$STDERR_LOG")" "already running in an ancestor process"
  assert_not_contains "$HOOK: BLOCKED never printed — hand-written blocks never ran" \
    "$(cat "$STDERR_LOG")" "BLOCKED"
done

# ===========================================================================
case_start "(f) bb96v reproduction: a contaminated .beads/hooks/<name> does not hang, recurse, or double-invoke bd"
# ===========================================================================
# Recreates the DOCUMENTED OUTPUT of `bd hooks install --beads` on a
# contaminated checkout (a self-referential copy of the pre-fix tracked
# hook, with a trailing exec/call back to itself) — without ever running
# the real `bd hooks install --beads` (never against a real database, never
# risking the real backup mirrors, per the bead's own explicit caution).
# The NEW tracked hook must ignore this file entirely: it no longer
# references .beads/hooks/ at all, so contamination sitting there is inert.
# Run for BOTH pre-push (a plain-call chain in the old design — forked a new
# process per recursion level) and pre-commit (an `exec` chain — a
# single-process infinite loop instead), since the two failure modes were
# confirmed to differ during the original discovery.
#
# A counting `bd` stub proves not just "didn't hang" but "ran exactly
# once" — zero would mean the hook silently swallowed the invocation
# entirely (a different regression), and more than once would mean the old
# recursion survived in some other form.
for HOOK in pre-push pre-commit; do
  CASE="$WORKDIR/case-f-$HOOK"; mkdir -p "$CASE"
  make_clone "$CASE"
  mkdir -p "$CLONE/.beads/hooks"
  case "$HOOK" in
    pre-push)
      # The OLD tracked pre-push's chain block: a plain subprocess call
      # back to "itself" (this same file, once copied to .beads/hooks/).
      cat > "$CLONE/.beads/hooks/$HOOK" <<CONTAMINATED_EOF
#!/usr/bin/env sh
_common="\$(git rev-parse --git-common-dir 2>/dev/null)" || _common=""
if [ -n "\$_common" ]; then
  _bdhook="\$_common/../.beads/hooks/$HOOK"
  if [ -x "\$_bdhook" ]; then
    "\$_bdhook" "\$@"
    exit \$?
  fi
fi
exit 0
CONTAMINATED_EOF
      ;;
    pre-commit)
      # The OLD tracked pre-commit's chain block: `exec` back to "itself"
      # (a single-process loop, not a fork — the other failure mode).
      cat > "$CLONE/.beads/hooks/$HOOK" <<CONTAMINATED_EOF
#!/usr/bin/env sh
_common="\$(git rev-parse --git-common-dir 2>/dev/null)" || _common=""
if [ -n "\$_common" ]; then
  _bdhook="\$_common/../.beads/hooks/$HOOK"
  [ -x "\$_bdhook" ] && exec "\$_bdhook" "\$@"
fi
exit 0
CONTAMINATED_EOF
      ;;
  esac
  chmod +x "$CLONE/.beads/hooks/$HOOK"

  STUB_BIN="$CASE/bin"; mkdir -p "$STUB_BIN"
  BD_CALL_LOG="$CASE/bd-calls.log"; : > "$BD_CALL_LOG"
  cat > "$STUB_BIN/bd" <<STUB_EOF
#!/usr/bin/env sh
if [ "\$1" = "hooks" ] && [ "\$2" = "run" ]; then
  echo "\$3" >> "$BD_CALL_LOG"
  exit 0
fi
exit 9
STUB_EOF
  chmod +x "$STUB_BIN/bd"

  STDOUT_LOG="$CASE/stdout.txt"; STDERR_LOG="$CASE/stderr.txt"
  LOCAL_SHA="$(git -C "$CLONE" rev-parse HEAD)"
  (
    printf 'refs/heads/feature %s refs/heads/feature %s\n' "$LOCAL_SHA" "$LOCAL_SHA" \
      | ( cd "$CLONE" && PATH="$STUB_BIN:$BASE_PATH" "./scripts/git-hooks/$HOOK" origin "https://example.invalid/x.git" ) \
      >"$STDOUT_LOG" 2>"$STDERR_LOG" &
    HOOK_PID=$!
    ( sleep 10; kill -9 "$HOOK_PID" 2>/dev/null && echo "WATCHDOG-KILLED" >> "$CASE/watchdog.txt" ) &
    WATCHDOG_PID=$!
    wait "$HOOK_PID" 2>/dev/null
    echo $? > "$CASE/hook-rc.txt"
    kill "$WATCHDOG_PID" 2>/dev/null
  )
  HOOK_RC="$(cat "$CASE/hook-rc.txt" 2>/dev/null || echo "unknown")"
  BD_CALL_COUNT="$(wc -l < "$BD_CALL_LOG" | tr -d ' ')"
  assert_eq "$HOOK: exits promptly (not killed by the watchdog)" \
    "$([ -f "$CASE/watchdog.txt" ] && echo killed || echo not-killed)" "not-killed"
  assert_eq "$HOOK: exits 0 — the contaminated file is never even reached" "$HOOK_RC" "0"
  assert_eq "$HOOK: bd hooks run invoked exactly once (not zero, not recursively)" \
    "$BD_CALL_COUNT" "1"
  assert_eq "$HOOK: invoked with the correct hook name" \
    "$(cat "$BD_CALL_LOG")" "$HOOK"
done

# ---------------------------------------------------------------------------
echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
