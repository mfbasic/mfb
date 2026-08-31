#!/usr/bin/env bash
# Self-test for the four networking proof harnesses (plan-110-F Phase 1 box 3).
#
# A check that only ever reports PASS is indistinguishable from a check that
# cannot fail. Each harness here is run twice: once as shipped (must PASS), and
# once against a deliberately WRONG expectation (must FAIL, and fail for the
# injected reason rather than by crashing on the edit).
#
# The injection is done on a COPY under a temp dir; the checked-in scripts are
# never modified, so an interrupted run cannot leave a sabotaged harness behind.
#
#   check-tcp-connect-timeout.sh  expect a different error code than ErrTimeout
#   check-udp-echo.sh             expect a payload the echo peer will not return
#   check-tls-loopback.sh         expect the wrong echoed body from the TLS server
#   check-icmp-permission.sh      expect a PingStatus where the contract says raise
#
# The TLS harness carries a second, built-in negative that needs no injection: an
# `openssl s_client` given an unrelated CA must fail verification. That one runs
# on every ordinary invocation.
#
# Usage: check-net-harness-selftest.sh <mfb-exe>
set -u

if [ "$#" -lt 1 ]; then
  echo "usage: check-net-harness-selftest.sh <mfb-exe>" >&2
  exit 2
fi
MFB_EXE=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
ROOT=$(cd "$(dirname "$0")/.." && pwd)

# The sabotaged copies must live in scripts/ because each harness resolves its
# peers (net_blackhole_server.py, gen-test-tls-identity.sh) relative to its own
# location. They are named with a dot prefix and removed on every exit path,
# including an interrupt.
work=$(mktemp -d)
cleanup() { rm -rf "$work"; rm -f "$ROOT"/scripts/.selftest-*; }
trap cleanup EXIT INT TERM
# Also clear anything a previously killed run left behind.
rm -f "$ROOT"/scripts/.selftest-*
failures=0

# Run a harness copy, expecting success or failure. Prints one verdict line.
# usage: expect <pass|fail> <label> <script-path> [extra-grep-for-fail-output]
expect() {
  want=$1; label=$2; script=$3; needle=${4:-}
  out=$(cd "$ROOT" && bash "$script" "$MFB_EXE" 2>&1)
  status=$?
  if [ "$want" = pass ]; then
    if [ "$status" -eq 0 ]; then
      echo "  ok   $label (as shipped: PASS)"
    else
      echo "  BAD  $label (as shipped: expected PASS, got exit $status)"
      printf '%s\n' "$out" | sed 's/^/       | /'
      failures=$((failures + 1))
    fi
    return
  fi
  if [ "$status" -eq 0 ]; then
    echo "  BAD  $label (sabotaged: expected FAIL, but it PASSED -- the check cannot detect a wrong result)"
    failures=$((failures + 1))
    return
  fi
  if [ -n "$needle" ] && ! printf '%s\n' "$out" | grep -q "$needle"; then
    echo "  BAD  $label (sabotaged: failed, but not for the injected reason)"
    printf '%s\n' "$out" | sed 's/^/       | /'
    failures=$((failures + 1))
    return
  fi
  echo "  ok   $label (sabotaged: FAIL, as it must)"
}

# Write a sabotaged copy beside the original, at the fixed path
# `scripts/.selftest-<name>`. It must live in scripts/ because each harness
# resolves its peers relative to its own location. Sets no output and is NOT
# called in a command substitution: a subshell would discard `failures`.
#
# Verifies the injection actually changed something -- a sed that silently
# matched nothing would otherwise "prove" the harness detects a wrong result
# when it never saw one.
sabotage() { # <source> <name> <sed-expression>
  dest="$ROOT/scripts/.selftest-$2"
  sed "$3" "$1" >"$dest"
  chmod +x "$dest"
  if cmp -s "$1" "$dest"; then
    echo "  BAD  injection changed nothing in $(basename "$1")"
    failures=$((failures + 1))
  fi
}

echo "tcp connect timeout:"
expect pass "check-tcp-connect-timeout.sh" "$ROOT/scripts/check-tcp-connect-timeout.sh"
sabotage "$ROOT/scripts/check-tcp-connect-timeout.sh" "tcp.sh" \
    's/^ERR_TIMEOUT=77050008$/ERR_TIMEOUT=77050009/'
expect fail "check-tcp-connect-timeout.sh" "$ROOT/scripts/.selftest-tcp.sh" "expected ErrTimeout"

echo "udp echo:"
expect pass "check-udp-echo.sh" "$ROOT/scripts/check-udp-echo.sh"
sabotage "$ROOT/scripts/check-udp-echo.sh" "udp.sh" \
    's/^lens=1,2,3$/lens=9,9,9/'
expect fail "check-udp-echo.sh" "$ROOT/scripts/.selftest-udp.sh" "round trip mismatch"

echo "tls loopback:"
expect pass "check-tls-loopback.sh" "$ROOT/scripts/check-tls-loopback.sh"
sabotage "$ROOT/scripts/check-tls-loopback.sh" "tls.sh" \
    's/\*"echo:hello-tls"\*)/*"echo:NOT-WHAT-THE-SERVER-SENDS"*)/'
expect fail "check-tls-loopback.sh" "$ROOT/scripts/.selftest-tls.sh" "did not round-trip"

echo "icmp permission:"
expect pass "check-icmp-permission.sh" "$ROOT/scripts/check-icmp-permission.sh"
sabotage "$ROOT/scripts/check-icmp-permission.sh" "icmp.sh" \
    's/if \[ "\$denied" != "raised" \]; then/if [ "$denied" != "status:Ok" ]; then/'
expect fail "check-icmp-permission.sh" "$ROOT/scripts/.selftest-icmp.sh" "must RAISE"

echo
if [ "$failures" -ne 0 ]; then
  echo "FAIL: $failures harness self-test(s) did not behave as required" >&2
  exit 1
fi
echo "PASS: every networking harness passes as shipped and fails on an injected wrong result"
