#!/usr/bin/env bash
#
# scripts/ci-apt-install.sh (personal-cfo-56w.6): a stub apt-get stands in for the network. The stub
# fails (or hangs) while the apt sources still name the runner's default mirror and works once they
# name the canonical one, so these cases show the retries are asked for, a hung attempt is cut off,
# the fallback path is taken when the first mirror fails, the sources are rewritten, and the log says
# which mirror served the install.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/ci-apt-install.sh"
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

# A stub apt-get. MODE decides how it behaves while the sources name azure.archive.ubuntu.com:
#   fail  -> exit 100 (the 2026-11-11 outage)    hang -> sleep    (a hung mirror)
#   slow  -> 0.7 s per apt-get call, then success (so update plus install exceed a 1 s attempt cap, though neither call does alone)
# With the canonical mirror in the sources it succeeds, unless MODE is "alldown" (both are down).
mkdir -p "$TMP/bin"
cat >"$TMP/bin/apt-get" <<'STUB'
#!/usr/bin/env bash
echo "apt-get $*" >>"$APT_LOG"
[ "$MODE" = "ok" ] && exit 0
if [ "$MODE" = "slow" ] && grep -q 'azure.archive.ubuntu.com' "$SOURCES_FILE"; then sleep 0.7; exit 0; fi
if grep -q 'azure.archive.ubuntu.com' "$SOURCES_FILE" || [ "$MODE" = "alldown" ]; then
  if [ "$MODE" = "hang" ]; then exec sleep 30 >/dev/null 2>&1; fi
  echo "E: Temporary failure resolving 'azure.archive.ubuntu.com'" >&2
  exit 100
fi
exit 0
STUB
chmod +x "$TMP/bin/apt-get"
# GNU `timeout` is on every hosted Linux runner but not on macOS: where it is missing, a perl alarm stands in so the
# suite also runs on a developer's Mac. The shim is only ever on the test's PATH.
if ! command -v timeout >/dev/null 2>&1; then
  printf '#!/usr/bin/env bash\nexec perl -e '"'"'alarm shift; exec @ARGV'"'"' "$@"\n' >"$TMP/bin/timeout"
  chmod +x "$TMP/bin/timeout"
fi

# Runs the script against a fresh sources file. Sets OUT, CODE; the log is $TMP/<case>/apt.log.
run_case() {
  local name="$1" mode="$2"
  shift 2
  mkdir -p "$TMP/$name"
  printf 'URIs: http://azure.archive.ubuntu.com/ubuntu/\nURIs: http://security.ubuntu.com/ubuntu/\n' >"$TMP/$name/ubuntu.sources"
  : >"$TMP/$name/apt.log"
  OUT="$(PATH="$TMP/bin:$PATH" APT_LOG="$TMP/$name/apt.log" SOURCES_FILE="$TMP/$name/ubuntu.sources" MODE="$mode" \
    CI_APT_SUDO="" CI_APT_SOURCES="$TMP/$name/ubuntu.sources" CI_APT_ATTEMPT_SECONDS="${ATTEMPT_SECONDS:-5}" \
    bash "$SCRIPT" "$@" 2>&1)"
  CODE=$?
}
logged() { grep -c -- "$2" "$TMP/$1/apt.log"; }
out_has() { grep -Fq -- "$1" <<<"$OUT"; }

# 1. A working default mirror: installed on the first attempt, nothing rewritten, retries and timeouts asked for.
run_case ok ok libfoo xvfb
check "default mirror up: exit 0" [ "$CODE" -eq 0 ]
check "default mirror up: the log says the default mirror served it" out_has "default (the runner's own mirror) served"
check "default mirror up: sources untouched" grep -q 'azure.archive.ubuntu.com' "$TMP/ok/ubuntu.sources"
check "apt is asked for retries and a connection timeout" grep -q 'Acquire::Retries=3.*Acquire::http::Timeout=20' "$TMP/ok/apt.log"
check "the packages reach apt-get install" grep -q 'install -y libfoo xvfb' "$TMP/ok/apt.log"

# 2. The default mirror is down: the fallback path is taken, the sources are rewritten, the log says so.
run_case fallback fail libfoo
check "FALLBACK: default mirror down, canonical up: exit 0" [ "$CODE" -eq 0 ]
check "FALLBACK: the log says the fallback served it" out_has "FALLBACK) served the install"
check "FALLBACK: the warning names the switch" out_has "switching to archive.ubuntu.com and security.ubuntu.com"
check "FALLBACK: the sources now name archive.ubuntu.com, not the azure mirror" bash -c "! grep -q azure '$TMP/fallback/ubuntu.sources' && grep -q 'http://archive.ubuntu.com/ubuntu/' '$TMP/fallback/ubuntu.sources'"
check "FALLBACK: the security mirror stays security.ubuntu.com" grep -q 'http://security.ubuntu.com/ubuntu/' "$TMP/fallback/ubuntu.sources"
check "FALLBACK: apt ran update twice (the failed attempt, then the retry on the new mirror)" [ "$(logged fallback ' update')" -eq 2 ]

# 3. A hung mirror is cut off by the attempt cap, then the fallback serves the install, in bounded time.
ATTEMPT_SECONDS=1 run_case hang hang libfoo
check "HANG: a hung first attempt is killed and the fallback succeeds" [ "$CODE" -eq 0 ]
check "HANG: the log says the fallback served it" out_has "FALLBACK) served the install"

# 3b. The cap is on the WHOLE attempt: update plus install take 1.4 s here, each under the 1 s cap, and the attempt is still cut off.
ATTEMPT_SECONDS=1 run_case slow slow libfoo
check "ONE CAP: an attempt whose update and install each fit the cap but not together is cut off, and the fallback serves it" out_has "FALLBACK) served the install"

# 3c. A hanging first attempt still reaches the fallback inside the cap, and the whole run stays within two caps.
START=$SECONDS
ATTEMPT_SECONDS=2 run_case bound hang libfoo
ELAPSED=$((SECONDS - START))
check "BOUND: a hanging first attempt reaches the fallback (exit 0)" [ "$CODE" -eq 0 ]
check "BOUND: the run took under two caps plus margin (took ${ELAPSED}s, cap 2s)" [ "$ELAPSED" -le 6 ]

# 4. Both mirrors down: a red result after exactly two attempts, never a stall.
run_case down alldown libfoo
check "BOTH DOWN: exit 1" [ "$CODE" -eq 1 ]
check "BOTH DOWN: the error says both were tried" out_has "failed against the default mirror and against archive.ubuntu.com"
check "BOTH DOWN: two attempts, no more" [ "$(logged down ' update')" -eq 2 ]
check "BOTH DOWN: no install was run" [ "$(logged down ' install')" -eq 0 ]

# 4b. The default cap fits twice into the workflow steps' timeout with a minute to spare (the script's own comment promises it).
# shellcheck disable=SC2016
DEFAULT_CAP="$(sed -n 's/^attempt_seconds="\${CI_APT_ATTEMPT_SECONDS:-\([0-9]*\)}"$/\1/p' "$SCRIPT")"
check "the default attempt cap is a number (found '${DEFAULT_CAP}')" [ -n "$DEFAULT_CAP" ]
STEP_MINUTES=10
check "two default attempts plus 60 s fit the ${STEP_MINUTES}-minute step timeout" [ $((2 * ${DEFAULT_CAP:-9999} + 60)) -le $((STEP_MINUTES * 60)) ]
# (That every apt step carries timeout-minutes is the guard's job: scripts/check-workflow-timeouts.sh.)

# 5. No packages is a usage error.
bash "$SCRIPT" >/dev/null 2>&1
check "no packages: usage error (exit 2)" [ $? -eq 2 ]

echo "ci-apt-install: $CASES cases, $FAILURES failed"
[ "$FAILURES" -eq 0 ]
