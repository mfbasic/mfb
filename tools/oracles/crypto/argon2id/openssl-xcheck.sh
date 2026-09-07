#!/usr/bin/env bash
# A SECOND, independent Argon2id oracle: OpenSSL's own ARGON2ID KDF.
#
# `cargo run --release` already self-checks this directory's reference against
# RFC 9106 / RFC 7693 and cross-checks it against RustCrypto `argon2 =0.5.3`.
# This script adds a third opinion from a different codebase entirely, which is
# the point: two implementations agreeing is weak evidence when one was written
# by reading the other. Run it when changing the MFBASIC core.
#
# Usage: ./openssl-xcheck.sh [path-to-openssl]
#   Needs OpenSSL >= 3.2 (the release that added the ARGON2ID KDF). macOS ships
#   LibreSSL as `openssl`, which does NOT have it -- pass a Homebrew path, e.g.
#   ./openssl-xcheck.sh /opt/homebrew/opt/openssl@3/bin/openssl
set -uo pipefail

OPENSSL=${1:-openssl}
command -v "$OPENSSL" >/dev/null || { echo "no such openssl: $OPENSSL" >&2; exit 2; }
ver=$("$OPENSSL" version 2>/dev/null)
case "$ver" in
  LibreSSL*) echo "SKIP: $OPENSSL is $ver -- LibreSSL has no ARGON2ID KDF" >&2; exit 2 ;;
esac
"$OPENSSL" kdf -help 2>&1 | grep -q kdfopt || { echo "SKIP: $OPENSSL has no 'kdf' command ($ver)" >&2; exit 2; }

REF_BIN=target/release/argon2ref
[ -x "$REF_BIN" ] || cargo build --release --quiet || exit 1

hexof() { printf '%s' "$1" | od -An -tx1 -v | tr -d ' \n'; }

# pass  salt              m      t  p  len
CASES="
password somesalt12345678 8      1  1  32
password somesalt12345678 32     3  4  32
password somesalt12345678 19456  2  1  32
x        saltsalt         4096   1  1  16
"

fail=0; ran=0
while read -r pw sa m t p l; do
  [ -z "${pw:-}" ] && continue
  mine=$("$REF_BIN" run "$(hexof "$pw")" "$(hexof "$sa")" "$m" "$t" "$p" "$l")
  theirs=$("$OPENSSL" kdf -keylen "$l" -kdfopt "pass:$pw" -kdfopt "salt:$sa" \
             -kdfopt "iter:$t" -kdfopt "memcost:$m" -kdfopt "lanes:$p" \
             -kdfopt "threads:1" -binary ARGON2ID 2>/dev/null \
           | od -An -tx1 -v | tr -d ' \n')
  ran=$((ran+1))
  if [ "$mine" = "$theirs" ] && [ -n "$theirs" ]; then
    echo "OK   m=$m t=$t p=$p l=$l  $mine"
  else
    fail=$((fail+1))
    echo "FAIL m=$m t=$t p=$p l=$l"
    echo "  reference: $mine"
    echo "  openssl:   ${theirs:-<empty -- check the kdfopt names for your version>}"
  fi
done <<EOF
$CASES
EOF

echo "openssl-xcheck ($ver): $ran case(s), $fail failure(s)"
[ "$ran" -gt 0 ] || { echo "ran NOTHING -- that is a harness bug, not a pass" >&2; exit 2; }
exit $([ "$fail" -eq 0 ] && echo 0 || echo 1)
