#!/usr/bin/env bash
# The MFBASIC-to-MFBASIC TLS loopback proof, run on a REMOTE Linux box
# (plan-110-F Phase 2).
#
# `check-tls-loopback.sh` proves the server half everywhere, but its
# MFBASIC-client leg needs a backend that takes a local trust anchor -- the
# OpenSSL one, via `SSL_CERT_FILE` -- so it SKIPs on the macOS dev host. This
# ships a cross-built client and server to a Linux box and runs the leg there,
# which is the only place the whole exchange can be MFBASIC on both ends without
# a test modifying a system trust store.
#
# The identity is generated HERE (the dev host has openssl and the generator) and
# copied over, so the remote box needs nothing but a shell.
#
# Usage: check-tls-loopback-remote.sh <mfb-exe> <ssh-port> [linux-target]
#   e.g. check-tls-loopback-remote.sh target/release/mfb 2227 linux-x86_64
set -u

if [ "$#" -lt 2 ]; then
  echo "usage: check-tls-loopback-remote.sh <mfb-exe> <ssh-port> [linux-target]" >&2
  exit 2
fi
MFB_EXE=$1
PORT_SSH=$2
TARGET=${3:-linux-x86_64}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TLS_PORT=${TLS_LOOPBACK_PORT:-18443}
REMOTE=test@127.0.0.1
RDIR=mfb-tls-loopback

work=$(mktemp -d)
cleanup() {
  ssh -p "$PORT_SSH" -o BatchMode=yes "$REMOTE" "rm -rf $RDIR" 2>/dev/null
  rm -rf "$work"
}
trap cleanup EXIT

bash "$ROOT/scripts/gen-test-tls-identity.sh" "$work/id" >/dev/null || exit 1
bash "$ROOT/scripts/gen-test-tls-identity.sh" "$work/other" >/dev/null || exit 1

make_project() { # <dir> <name> <source>
  mkdir -p "$1/src"
  cat >"$1/project.json" <<EOF
{ "name": "$2", "version": "0.1.0", "mfb": "1.0",
  "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
EOF
  printf '%s\n' "$3" >"$1/src/main.mfb"
}

# The paths the server reads are the REMOTE ones; the identity is copied to
# ~/$RDIR/id there.
remote_home=$(ssh -p "$PORT_SSH" -o BatchMode=yes "$REMOTE" 'echo $HOME' 2>/dev/null)
if [ -z "$remote_home" ]; then
  echo "SKIP: box on ssh port $PORT_SSH is not reachable"; exit 0
fi

make_project "$work/server" tls_remote_server "IMPORT encoding
IMPORT io
IMPORT tls

FUNC main AS Integer
  RES listener = tls::listen(\"127.0.0.1\", $TLS_PORT, \"$remote_home/$RDIR/id/chain.pem\", \"$remote_home/$RDIR/id/server-key.pem\")
  io::print(\"listening\")
  RES conn = tls::accept(listener, 20000)
  LET got = encoding::utf8Decode(tls::read(conn, 1024))
  tls::write(conn, \"echo:\" & got)
  tls::close(conn)
  tls::close(listener)
  RETURN 0
END FUNC"

make_project "$work/client" tls_remote_client "IMPORT encoding
IMPORT io
IMPORT tls

FUNC main AS Integer
  RES sock = tls::connect(\"127.0.0.1\", $TLS_PORT, 10000, \"127.0.0.1\")
  tls::write(sock, \"hello-tls\")
  io::print(encoding::utf8Decode(tls::read(sock, 1024)))
  tls::close(sock)
  RETURN 0
END FUNC"

for which in server client; do
  out=$("$MFB_EXE" build -target "$TARGET" "$work/$which" 2>&1) || {
    echo "FAIL: $which build for $TARGET failed" >&2; printf '%s\n' "$out" >&2; exit 1; }
done

# musl and glibc variants are both emitted; ship whichever the box runs.
libc=$(ssh -p "$PORT_SSH" -o BatchMode=yes "$REMOTE" \
  'ldd --version 2>&1 | head -1 | grep -qi musl && echo musl || echo glibc')
ssh -p "$PORT_SSH" -o BatchMode=yes "$REMOTE" "rm -rf $RDIR && mkdir -p $RDIR/id" || exit 1
# NB: the unrelated CA is copied SEPARATELY, under its own name. Listing it in
# the same scp as `id/ca.pem` lands both as `ca.pem` and the second silently
# clobbers the first, so the "trusted" leg then runs against the wrong anchor and
# fails the handshake -- a self-inflicted failure that looks exactly like a real
# one.
scp -q -P "$PORT_SSH" -o BatchMode=yes \
  "$work/id/chain.pem" "$work/id/server-key.pem" "$work/id/ca.pem" \
  "$REMOTE:$RDIR/id/" || exit 1
scp -q -P "$PORT_SSH" -o BatchMode=yes "$work/other/ca.pem" "$REMOTE:$RDIR/id/other-ca.pem" || exit 1
scp -q -P "$PORT_SSH" -o BatchMode=yes "$work/server/build/tls_remote_server-$libc.out" \
  "$REMOTE:$RDIR/server.out" || exit 1
scp -q -P "$PORT_SSH" -o BatchMode=yes "$work/client/build/tls_remote_client-$libc.out" \
  "$REMOTE:$RDIR/client.out" || exit 1

run_leg() { # <ca-file> -> prints the client's output
  ssh -p "$PORT_SSH" -o BatchMode=yes "$REMOTE" "
    cd $RDIR && chmod +x server.out client.out
    ./server.out > server.log 2>&1 &
    pid=\$!
    for i in \$(seq 1 100); do grep -q listening server.log 2>/dev/null && break; sleep 0.1; done
    SSL_CERT_FILE=\$PWD/id/$1 ./client.out 2>&1
    rc=\$?
    kill \$pid 2>/dev/null; wait \$pid 2>/dev/null
    echo \"[rc \$rc]\"
  " 2>&1
}

trusted=$(run_leg ca.pem)
case "$trusted" in
  *"echo:hello-tls"*"[rc 0]"*) ;;
  *) echo "FAIL: MFBASIC-to-MFBASIC TLS exchange did not complete on the remote box" >&2
     printf '%s\n' "$trusted" >&2
     exit 1 ;;
esac
echo "PASS: tls::connect <-> tls::listen completed on $TARGET/$libc (port $PORT_SSH)"

untrusted=$(run_leg other-ca.pem)
case "$untrusted" in
  *"[rc 0]"*) echo "FAIL: tls::connect ACCEPTED a chain signed by an unrelated CA" >&2
              printf '%s\n' "$untrusted" >&2
              exit 1 ;;
esac
echo "PASS: tls::connect refuses a chain it cannot verify on $TARGET/$libc"
