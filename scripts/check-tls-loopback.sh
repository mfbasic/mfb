#!/usr/bin/env bash
# Deterministic local TLS proof: a real handshake and a real data exchange over
# loopback, with no public-network dependency (plan-110-F Phase 1).
#
# The acceptance suite only compiles `tls::listen`/`accept` (tests/syntax/tls);
# the one runtime TLS fixture, `rt-behavior/tls/tls-connect-google-rt`, needs the
# public internet and proves only the client half. This check closes both gaps
# locally.
#
# ## What it proves, and why the peers are what they are
#
# 1. **Server half, against a foreign client.** An MFBASIC `tls::listen` /
#    `accept` / `read` / `write` server is dialled by `openssl s_client`, which is
#    told to trust the generated test CA. A handshake that completes and a payload
#    that round-trips prove the server presents a valid chain and speaks TLS on the
#    wire -- not merely that our client and our server agree with each other.
#
# 2. **Certificate rejection.** The same server, dialled by a client that is NOT
#    given the CA, must fail verification. A proof that only ever shows success
#    cannot tell a working verifier from an absent one.
#
# 3. **Client half, where the backend allows a local trust anchor.** The OpenSSL
#    backend calls `SSL_CTX_set_default_verify_paths`, which honours `SSL_CERT_FILE`,
#    so on that backend an MFBASIC `tls::connect` can be pointed at the test CA and
#    the whole exchange runs MFBASIC-to-MFBASIC. macOS (Network.framework) and
#    Windows (Schannel) take their anchors from the system trust store with no such
#    hook, so this step reports SKIP there rather than pretending: on those targets
#    the client half is covered by the server-side proof plus the public fixture.
#    Installing a CA into a system trust store is not something a test may do.
#
# Usage: check-tls-loopback.sh <mfb-exe>
set -u

if [ "$#" -lt 1 ]; then
  echo "usage: check-tls-loopback.sh <mfb-exe>" >&2
  exit 2
fi
MFB_EXE=$1
ROOT=$(cd "$(dirname "$0")/.." && pwd)
PORT=${TLS_LOOPBACK_PORT:-18443}

command -v openssl >/dev/null 2>&1 || { echo "FAIL: openssl not found" >&2; exit 1; }

work=$(mktemp -d)
server_pid=""
cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null
    wait "$server_pid" 2>/dev/null
  fi
  rm -rf "$work"
}
trap cleanup EXIT

bash "$ROOT/scripts/gen-test-tls-identity.sh" "$work/id" >/dev/null || exit 1
# A second, UNRELATED CA: the negative cases need a trust store that is valid
# but does not contain our anchor. `-CAfile /dev/null` is not that -- openssl
# fails to LOAD it, which is a different error and would pass a broken
# verifier just as happily.
bash "$ROOT/scripts/gen-test-tls-identity.sh" "$work/other" >/dev/null || exit 1

# ---------------------------------------------------------------------------
# The MFBASIC server: accept one connection, echo one message back, exit.
# ---------------------------------------------------------------------------
mkdir -p "$work/server/src"
cat >"$work/server/project.json" <<'EOF'
{ "name": "tls_loopback_server", "version": "0.1.0", "mfb": "1.0",
  "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
EOF
cat >"$work/server/src/main.mfb" <<EOF
IMPORT encoding
IMPORT io
IMPORT tls

FUNC main AS Integer
  RES listener = tls::listen("127.0.0.1", $PORT, "$work/id/chain.pem", "$work/id/server-key.pem")
  io::print("listening")
  RES conn = tls::accept(listener, 20000)
  LET got = encoding::utf8Decode(tls::read(conn, 1024))
  tls::write(conn, "echo:" & got)
  tls::close(conn)
  tls::close(listener)
  RETURN 0
END FUNC
EOF

build_output=$("$MFB_EXE" build "$work/server" 2>&1) || {
  echo "FAIL: server build error" >&2; printf '%s\n' "$build_output" >&2; exit 1; }
server_exe=$(printf '%s\n' "$build_output" | sed -n 's/^Wrote executable to //p' | tail -n 1)

start_server() {
  "$server_exe" >"$work/server.out" 2>&1 &
  server_pid=$!
  for _ in $(seq 1 100); do
    grep -q listening "$work/server.out" 2>/dev/null && return 0
    kill -0 "$server_pid" 2>/dev/null || return 1
    sleep 0.1
  done
  return 1
}
stop_server() {
  [ -n "$server_pid" ] && { kill "$server_pid" 2>/dev/null; wait "$server_pid" 2>/dev/null; }
  server_pid=""
}

fail() { echo "FAIL: $1" >&2; [ -s "$work/server.out" ] && { echo "--- server output" >&2; cat "$work/server.out" >&2; }; exit 1; }

# ---------------------------------------------------------------------------
# 1. Foreign client, CA trusted: handshake completes and the payload round-trips.
# ---------------------------------------------------------------------------
start_server || fail "server did not start (trusted-client case)"
client_out=$(printf 'hello-tls\n' | openssl s_client -connect "127.0.0.1:$PORT" \
  -CAfile "$work/id/ca.pem" -verify_return_error -quiet 2>"$work/client.err")
client_status=$?
stop_server
if [ "$client_status" -ne 0 ]; then
  echo "--- s_client stderr" >&2; cat "$work/client.err" >&2
  fail "openssl s_client could not complete the handshake against tls::listen"
fi
case "$client_out" in
  *"echo:hello-tls"*) ;;
  *) echo "--- s_client stdout" >&2; printf '%s\n' "$client_out" >&2
     fail "payload did not round-trip through the MFBASIC TLS server" ;;
esac
echo "PASS: tls::listen/accept/read/write served a foreign TLS client (chain verified)"

# ---------------------------------------------------------------------------
# 2. Same server, client NOT given the CA: verification must fail.
# ---------------------------------------------------------------------------
start_server || fail "server did not start (untrusted-client case)"
untrusted_err=$(printf 'hello-tls\n' | openssl s_client -connect "127.0.0.1:$PORT" \
  -CAfile "$work/other/ca.pem" -verify_return_error -quiet 2>&1 >/dev/null)
untrusted_status=$?
stop_server
if [ "$untrusted_status" -eq 0 ]; then
  fail "a client with no trust anchor ACCEPTED the self-signed chain -- verification is not happening"
fi
case "$untrusted_err" in
  *"unable to verify"*|*"self-signed"*|*"self signed"*|*"unable to get local issuer"*|*"certificate verify failed"*) ;;
  *) echo "--- s_client stderr" >&2; printf '%s\n' "$untrusted_err" >&2
     fail "the untrusted client failed for an unexpected reason (wanted a verification error)" ;;
esac
echo "PASS: an untrusted client is rejected at verification (the negative half)"

# ---------------------------------------------------------------------------
# 3. MFBASIC client, where the backend takes a local trust anchor.
# ---------------------------------------------------------------------------
host_os=$(uname -s)
if [ "$host_os" != "Linux" ]; then
  echo "SKIP: MFBASIC-to-MFBASIC leg needs SSL_CERT_FILE (OpenSSL backend); $host_os takes its"
  echo "      anchors from the system trust store, which a test may not modify. The server half"
  echo "      above is proven here; the client half is covered by rt-behavior/tls/tls-connect-google-rt."
  exit 0
fi

mkdir -p "$work/client/src"
cat >"$work/client/project.json" <<'EOF'
{ "name": "tls_loopback_client", "version": "0.1.0", "mfb": "1.0",
  "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
EOF
cat >"$work/client/src/main.mfb" <<EOF
IMPORT encoding
IMPORT io
IMPORT tls

FUNC main AS Integer
  RES sock = tls::connect("127.0.0.1", $PORT, 10000, "127.0.0.1")
  tls::write(sock, "hello-tls")
  io::print(encoding::utf8Decode(tls::read(sock, 1024)))
  tls::close(sock)
  RETURN 0
END FUNC
EOF
build_output=$("$MFB_EXE" build "$work/client" 2>&1) || {
  echo "FAIL: client build error" >&2; printf '%s\n' "$build_output" >&2; exit 1; }
client_exe=$(printf '%s\n' "$build_output" | sed -n 's/^Wrote executable to //p' | tail -n 1)

start_server || fail "server did not start (mfb-client case)"
mfb_client_out=$(SSL_CERT_FILE="$work/id/ca.pem" "$client_exe" 2>&1)
mfb_client_status=$?
stop_server
if [ "$mfb_client_status" -ne 0 ] || [ "$mfb_client_out" != "echo:hello-tls" ]; then
  echo "--- client output" >&2; printf '%s\n' "$mfb_client_out" >&2
  fail "MFBASIC tls::connect did not complete the loopback exchange"
fi
echo "PASS: tls::connect completed an MFBASIC-to-MFBASIC loopback exchange"

# The mirror of case 2 on the client side: with no trust anchor the MFBASIC
# client must refuse the same server.
start_server || fail "server did not start (mfb-client-untrusted case)"
untrusted_mfb=$(SSL_CERT_FILE="$work/other/ca.pem" "$client_exe" 2>&1)
untrusted_mfb_status=$?
stop_server
if [ "$untrusted_mfb_status" -eq 0 ]; then
  echo "--- client output" >&2; printf '%s\n' "$untrusted_mfb" >&2
  fail "MFBASIC tls::connect ACCEPTED an unverifiable chain"
fi
echo "PASS: tls::connect refuses a chain it cannot verify"
