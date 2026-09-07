#!/usr/bin/env bash
# A THIRD, independent opinion on every hash: OpenSSL's own `dgst`.
#
# `../run.sh` checks MFBASIC against the RustCrypto reference in this directory.
# That is two implementations, and two implementations agreeing is weaker than it
# looks -- they could share a misreading of the spec, and a wrong oracle is
# ratified rather than caught. This script adds a third codebase. Run it when
# changing the MFBASIC core, or when adding an algorithm here.
#
# Note this compares OpenSSL against the REFERENCE, not against MFBASIC: it is
# checking the judge, which is the thing `run.sh` cannot check for itself.
#
# Usage: ./openssl-xcheck.sh [path-to-openssl]
#   macOS ships LibreSSL as `openssl`, which has no SHA-3 or SHAKE. Algorithms
#   this openssl cannot compute are SKIPPED and counted, never silently passed:
#   ./openssl-xcheck.sh /opt/homebrew/opt/openssl@3/bin/openssl
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
OPENSSL=${1:-openssl}
command -v "$OPENSSL" >/dev/null || { echo "no such openssl: $OPENSSL" >&2; exit 2; }
ver=$("$OPENSSL" version 2>/dev/null)
command -v xxd >/dev/null || { echo "SKIP: xxd is needed to build the binary inputs" >&2; exit 2; }

REF_BIN=$HERE/target/release/hashref
[ -x "$REF_BIN" ] || cargo build --release --quiet || exit 1

WORK=$(mktemp -d "${TMPDIR:-/tmp}/hash-openssl.XXXXXX") || exit 2
trap 'rm -rf "$WORK"' EXIT

# label -> the `openssl dgst` flag that computes it.
flag_for() {
  case "$1" in
    sha1)      echo "-sha1" ;;
    sha2-224)  echo "-sha224" ;;
    sha2-256)  echo "-sha256" ;;
    sha2-384)  echo "-sha384" ;;
    sha2-512)  echo "-sha512" ;;
    sha3-224)  echo "-sha3-224" ;;
    sha3-256)  echo "-sha3-256" ;;
    sha3-384)  echo "-sha3-384" ;;
    sha3-512)  echo "-sha3-512" ;;
    shake256)  echo "-shake256" ;;
    *)         echo "" ;;
  esac
}

# The same input shape `../mfb` uses, so a length that trips one trips both.
hexpattern() { awk -v n="$1" 'BEGIN { for (i = 0; i < n; i++) printf "%02x", i % 251 }'; }

# "<label> <outlen>", the widths the algorithms actually produce.
CASES="sha1 20
sha2-224 28
sha2-256 32
sha2-384 48
sha2-512 64
sha3-224 28
sha3-256 32
sha3-384 48
sha3-512 64
shake256 32
shake256 64"

LENGTHS="0 1 55 56 64 111 112 128 135 136 143 144 1000"
# shellcheck disable=SC2086
LENGTH_COUNT=$(printf '%s\n' $LENGTHS | wc -l | tr -d ' ')

ran=0; fail=0; skipped=0
while read -r algo outlen; do
  [ -n "${algo:-}" ] || continue
  flag=$(flag_for "$algo")
  [ -n "$flag" ] || { echo "no openssl flag for $algo" >&2; fail=$((fail + 1)); continue; }

  # SHAKE is an XOF: without -xoflen, OpenSSL prints nothing at all. An openssl
  # that lacks the digest, or lacks -xoflen, is a SKIP -- not a pass.
  xof=""
  [ "$algo" = "shake256" ] && xof="-xoflen $outlen"
  # shellcheck disable=SC2086
  if ! probe=$(printf '' | "$OPENSSL" dgst $flag $xof -r 2>/dev/null) || [ -z "$probe" ]; then
    echo "SKIP $algo/$outlen -- $OPENSSL ($ver) cannot compute it"
    skipped=$((skipped + 1))
    continue
  fi

  bad=0
  for n in $LENGTHS; do
    hexstr=$(hexpattern "$n")
    printf '%s' "$hexstr" | xxd -r -p >"$WORK/in.bin"
    # shellcheck disable=SC2086
    theirs=$("$OPENSSL" dgst $flag $xof -r <"$WORK/in.bin" 2>/dev/null | cut -d' ' -f1)
    mine=$("$REF_BIN" run "$algo" "$outlen" "${hexstr:--}")
    ran=$((ran + 1))
    if [ -n "$theirs" ] && [ "$mine" = "$theirs" ]; then
      continue
    fi
    fail=$((fail + 1)); bad=$((bad + 1))
    echo "FAIL $algo/$outlen input=$n bytes"
    echo "  reference: $mine"
    echo "  openssl:   ${theirs:-<empty>}"
  done
  [ "$bad" -eq 0 ] && echo "OK   $algo/$outlen  $LENGTH_COUNT lengths"
done <<EOF
$CASES
EOF

echo "openssl-xcheck ($ver): $ran case(s), $fail failure(s), $skipped algorithm(s) skipped"
# A harness that ran nothing must not read as green.
[ "$ran" -gt 0 ] || { echo "ran NOTHING -- that is a harness bug, not a pass" >&2; exit 2; }
[ "$fail" -eq 0 ] || exit 1
exit 0
