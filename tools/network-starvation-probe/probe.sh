#!/usr/bin/env bash
# network-starvation-probe — reproduces and gates bug-704: an idle, unauthenticated
# raw TCP connection against `network-server --tls --thread` must not starve every
# other client's TLS handshake.
#
# Usage: bash tools/network-starvation-probe/probe.sh <mfb-exe>
#
# <mfb-exe> is the compiler binary to build the two example projects with
# (typically target/release/mfb). Exits 0 when both phases below pass, non-zero
# otherwise, and always kills every process it started (server + idle socket)
# before exiting.
#
# Phase A — attack: with one idle raw TCP connection held open against the
# server, a well-behaved TLS client must still complete its handshake and
# receive its Hello/Update lines within ATTACK_TIMEOUT seconds. Before
# bug-704's fix this hangs past the client's own 3000 ms connect timeout
# because the single accept thread is parked inside the idle peer's
# handshake read forever; after the fix, tls::accept(server, timeoutMs)
# gives up on the idle peer and the loop moves on to the queued legitimate
# client well within the bound.
#
# Phase B — recovery: after the idle connection is closed (or, post-fix, is
# evicted on its own by the bounded accept), a following client run must
# succeed within RECOVERY_TIMEOUT seconds. This phase already passes today;
# it guards against a fix that "solves" starvation by wedging the accept
# loop some other way.
#
# Readiness: a bind-then-release ephemeral port has a window where a
# concurrent process on this shared machine can steal it (.ai/testing-gates.md
# "ephemeral port picked with bind-then-release hands a case the WRONG
# server"). This script's readiness condition is the LIVE CHILD's own stdout —
# it greps the started server's real stdout for its "Listening tls" line
# rather than merely probing "is something listening", and retries on a fresh
# port if the child exited (lost the bind) before printing it.

set -u

if [ $# -lt 1 ]; then
  echo "usage: $0 <mfb-exe>" >&2
  exit 2
fi

MFB="$1"
if [ ! -x "$MFB" ]; then
  echo "not an executable: $MFB" >&2
  exit 2
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SERVER_DIR="$REPO_ROOT/examples/network-server"
CLIENT_DIR="$REPO_ROOT/examples/network-client"

ATTACK_TIMEOUT=15     # seconds a well-behaved client gets while the idle peer holds the accept thread
RECOVERY_TIMEOUT=15   # seconds a client gets once the idle peer is gone
# A full, successful client run against this server (TCP's 5s silent-timeout +
# a UDP attempt + TLS connect/handshake + five 500ms-apart Update lines)
# measures ~10.5s end to end (measured directly, this session) — both bounds
# above carry real margin over that, not just over the TLS phase alone.
BIND_RETRIES=5

WORK="$(mktemp -d /tmp/net-starvation-probe.XXXXXX)"
SERVER_LOG="$WORK/server.log"
CLIENT_LOG="$WORK/client.log"

SERVER_PID=""
IDLE_PY_PID=""

cleanup() {
  [ -n "$IDLE_PY_PID" ] && kill -9 "$IDLE_PY_PID" >/dev/null 2>&1
  [ -n "$SERVER_PID" ] && kill -9 "$SERVER_PID" >/dev/null 2>&1
  wait >/dev/null 2>&1
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

echo "== building network-server and network-client =="
"$MFB" build "$SERVER_DIR" || fail "network-server did not build"
"$MFB" build "$CLIENT_DIR" || fail "network-client did not build"

SERVER_BIN="$SERVER_DIR/build/network-server.out"
CLIENT_BIN="$CLIENT_DIR/build/network-client.out"
[ -x "$SERVER_BIN" ] || fail "missing $SERVER_BIN"
[ -x "$CLIENT_BIN" ] || fail "missing $CLIENT_BIN"

# Start the threaded TLS server on a scratch port, verified free and verified
# bound by the LIVE CHILD (its own "Listening tls" line), not by a
# bind-then-release probe a sibling process could have re-taken in the
# meantime.
start_server() {
  local attempt=0
  while [ "$attempt" -lt "$BIND_RETRIES" ]; do
    attempt=$((attempt + 1))
    PORT=$(( (RANDOM % 20000) + 20000 ))
    : > "$SERVER_LOG"
    "$SERVER_BIN" --tls "$SERVER_DIR/certs/cert.pem" "$SERVER_DIR/certs/key.pem" --thread --port "$PORT" >> "$SERVER_LOG" 2>&1 &
    SERVER_PID=$!

    local waited=0
    while [ "$waited" -lt 50 ]; do
      if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
        break # child exited — lost the bind or failed; retry on a new port
      fi
      if grep -q "Listening tls .*:$PORT" "$SERVER_LOG" 2>/dev/null; then
        return 0
      fi
      sleep 0.1
      waited=$((waited + 1))
    done

    # Either the child exited, or it never printed readiness in time: reap it
    # and try a fresh port.
    kill -9 "$SERVER_PID" >/dev/null 2>&1
    wait "$SERVER_PID" >/dev/null 2>&1
    SERVER_PID=""
  done
  return 1
}

start_server || fail "could not start network-server after $BIND_RETRIES attempts (see $SERVER_LOG)"
echo "server pid=$SERVER_PID port=$PORT"

run_client() {
  local timeout_s="$1"
  : > "$CLIENT_LOG"
  "$CLIENT_BIN" --port "$PORT" --server-name localhost --allow-self-signed >> "$CLIENT_LOG" 2>&1 &
  local cpid=$!
  local waited=0
  local max=$(( timeout_s * 10 ))
  while [ "$waited" -lt "$max" ]; do
    if ! kill -0 "$cpid" >/dev/null 2>&1; then
      wait "$cpid" >/dev/null 2>&1
      return $?
    fi
    sleep 0.1
    waited=$((waited + 1))
  done
  kill -9 "$cpid" >/dev/null 2>&1
  wait "$cpid" >/dev/null 2>&1
  return 124 # our own timeout marker, distinct from a real exit code
}

client_saw_five_updates() {
  grep -q "Update .* 05" "$CLIENT_LOG"
}

echo "== phase A: attack (idle raw TCP connection held open) =="
python3 - "$PORT" > "$WORK/idle.log" 2>&1 <<'PYEOF' &
import socket, sys, time
port = int(sys.argv[1])
s = socket.create_connection(("127.0.0.1", port), timeout=10)
time.sleep(3600)
PYEOF
IDLE_PY_PID=$!

# Give the idle connection a moment to actually land in the server's accept
# path before we measure the legitimate client against it.
sleep 0.5
if ! kill -0 "$IDLE_PY_PID" >/dev/null 2>&1; then
  fail "idle raw-socket helper exited immediately (see $WORK/idle.log)"
fi

run_client "$ATTACK_TIMEOUT"
attack_rc=$?
if [ "$attack_rc" -eq 0 ] && client_saw_five_updates; then
  echo "PASS: phase A — legitimate TLS client completed within ${ATTACK_TIMEOUT}s despite the idle connection"
  ATTACK_OK=1
else
  echo "attack-phase client log:"
  sed 's/^/  /' "$CLIENT_LOG"
  echo "FAIL (expected pre-fix): phase A — legitimate TLS client did not complete within ${ATTACK_TIMEOUT}s (rc=$attack_rc)"
  ATTACK_OK=0
fi

echo "== phase B: recovery (idle connection closed) =="
kill -9 "$IDLE_PY_PID" >/dev/null 2>&1
wait "$IDLE_PY_PID" >/dev/null 2>&1
IDLE_PY_PID=""

run_client "$RECOVERY_TIMEOUT"
recovery_rc=$?
if [ "$recovery_rc" -eq 0 ] && client_saw_five_updates; then
  echo "PASS: phase B — a following client recovers within ${RECOVERY_TIMEOUT}s"
  RECOVERY_OK=1
else
  echo "recovery-phase client log:"
  sed 's/^/  /' "$CLIENT_LOG"
  echo "FAIL: phase B — recovery did not happen within ${RECOVERY_TIMEOUT}s (rc=$recovery_rc)"
  RECOVERY_OK=0
fi

if [ "$ATTACK_OK" -eq 1 ] && [ "$RECOVERY_OK" -eq 1 ]; then
  echo "OVERALL PASS"
  exit 0
else
  echo "OVERALL FAIL"
  exit 1
fi
