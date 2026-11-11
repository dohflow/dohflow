#!/usr/bin/env bash
#
# scripts/check-workflow-timeouts.sh (personal-cfo-56w.6): the guard is only worth having if it
# fails on each thing it forbids, so every rule has a negative fixture here, and the real
# workflows must pass.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
GUARD="$REPO_ROOT/scripts/check-workflow-timeouts.sh"
CASES=0
FAILURES=0

check() {
  local label="$1"
  shift
  CASES=$((CASES + 1))
  if "$@"; then
    echo "    ok   — $label"
  else
    echo "    FAIL — $label" >&2
    FAILURES=$((FAILURES + 1))
  fi
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Writes a one-file workflow directory from stdin and prints its path.
fixture() {
  local name="$1"
  mkdir -p "$TMP/$name"
  cat >"$TMP/$name/ci.yml"
  echo "$TMP/$name"
}

# Runs the guard on a directory; sets OUT and CODE.
run() {
  OUT="$(bash "$GUARD" "$1" 2>&1)"
  CODE=$?
}
passes() { run "$1"; [ "$CODE" -eq 0 ]; }
fails_with() { run "$1"; [ "$CODE" -eq 1 ] && grep -Fq -- "$2" <<<"$OUT"; }

GOOD="$(fixture good <<'YML'
name: ci
on: push
jobs:
  build:
    runs-on: ubuntu-24.04
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - name: Install
        timeout-minutes: 10
        run: scripts/ci-apt-install.sh xvfb
  call:
    uses: ./.github/workflows/other.yml
YML
)"
check "a workflow with a timeout on every job, a bounded apt step and a reusable-workflow call passes" passes "$GOOD"

NO_TIMEOUT="$(fixture notimeout <<'YML'
jobs:
  quick:
    runs-on: ubuntu-24.04
    timeout-minutes: 5
    steps:
      - run: echo hi
  forgotten:
    runs-on: ubuntu-24.04
    steps:
      - run: echo hi
YML
)"
check "NEGATIVE: a job with no timeout-minutes fails, and the message names the job" fails_with "$NO_TIMEOUT" 'job "forgotten" has no timeout-minutes'

STEP_ONLY="$(fixture steponly <<'YML'
jobs:
  sneaky:
    runs-on: ubuntu-24.04
    steps:
      - run: echo hi
        timeout-minutes: 5
YML
)"
check "NEGATIVE: a step-level timeout does not stand in for the job's" fails_with "$STEP_ONLY" 'job "sneaky" has no timeout-minutes'

DIRECT_APT="$(fixture directapt <<'YML'
jobs:
  deps:
    runs-on: ubuntu-24.04
    timeout-minutes: 10
    steps:
      - run: |
          sudo apt-get update
          sudo apt-get install -y xvfb
YML
)"
check "NEGATIVE: a direct apt-get call fails" fails_with "$DIRECT_APT" 'calls apt-get directly'

UNBOUNDED_STEP="$(fixture unbounded <<'YML'
jobs:
  deps:
    runs-on: ubuntu-24.04
    timeout-minutes: 10
    steps:
      - name: Install
        run: scripts/ci-apt-install.sh xvfb
      - name: Next
        timeout-minutes: 10
        run: echo the timeout here must not be credited to the step above
YML
)"
check "NEGATIVE: an apt step with no timeout of its own fails, even when the next step has one" fails_with "$UNBOUNDED_STEP" 'no timeout-minutes of its own'

COMMENTED="$(fixture commented <<'YML'
jobs:
  deps:
    runs-on: ubuntu-24.04
    timeout-minutes: 10
    steps:
      # sudo apt-get update used to be here
      - run: echo hi
YML
)"
check "a comment that mentions apt-get is not a call" passes "$COMMENTED"

LAST_JOB="$(fixture lastjob <<'YML'
jobs:
  first:
    runs-on: ubuntu-24.04
    timeout-minutes: 5
    steps:
      - run: echo hi
  last:
    runs-on: ubuntu-24.04
    steps:
      - run: echo hi
YML
)"
check "NEGATIVE: the last job in a file is checked too" fails_with "$LAST_JOB" 'job "last" has no timeout-minutes'

mkdir -p "$TMP/empty"
check "an empty directory is a usage error (exit 2), not a pass" bash -c "bash '$GUARD' '$TMP/empty' >/dev/null 2>&1; [ \$? -eq 2 ]"
check "a missing directory is a usage error (exit 2)" bash -c "bash '$GUARD' '$TMP/nope' >/dev/null 2>&1; [ \$? -eq 2 ]"

# The workflows in this repository obey the rule.
check "the real workflows under .github/workflows pass" bash -c "bash '$GUARD' >/dev/null 2>&1"

echo "$CASES cases, $FAILURES failures"
[ "$FAILURES" -eq 0 ]
