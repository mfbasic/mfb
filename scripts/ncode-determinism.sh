#!/usr/bin/env bash
# bug-388 Phase 1/3 determinism harness.
#
# For each in-scope byte-identity / rt-behavior fixture, compile it N times, each
# in a FRESH `mfb` process (std HashMap seeds per process, so a stable order must
# survive independent seeds), and count how many distinct sha256s the emitted
# `.ncode` takes for the HOST target. Also compares the single/first hash against
# the committed `macos-aarch64` golden sum.
#
# Usage: scripts/ncode-determinism.sh <mfb-binary> [N]
#   |unique| > 1            -> FLAKY (residual nondeterminism)
#   |unique| == 1, != gold  -> STABLE-STALE-GOLDEN
#   |unique| == 1, == gold  -> CLEAN
set -u
MFB="$1"; N="${2:-50}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
host_arch="$(uname -m)"; case "$host_arch" in arm64) A=aarch64;; x86_64) A=x86_64;; *) A=$host_arch;; esac
case "$(uname -s)" in Darwin) HOST_TGT="macos-$A";; Linux) HOST_TGT="linux-$A";; *) HOST_TGT="unknown-$A";; esac

# fixture-dir  package-name
FIXTURES="
tests/byte-identity/audio audio_codegen_cover_rt
tests/byte-identity/crypto crypto_codegen_cover_rt
tests/byte-identity/fs fs_codegen_cover_rt
tests/byte-identity/net net_codegen_cover_rt
tests/byte-identity/os os_codegen_cover_rt
tests/byte-identity/tls tls_codegen_cover_rt
tests/rt-behavior/crypto/crypto-ec-valid crypto-ec-valid
"

overall_flaky=0
printf "%-42s %6s %-10s %s\n" FIXTURE UNIQ CLASS "hashes(count)"
echo "$FIXTURES" | while read -r td pkg; do
  [ -n "$td" ] || continue
  gsum="$REPO/$td/golden/$pkg.$HOST_TGT.ncodesum"
  gold=""; [ -f "$gsum" ] && gold=$(cut -d" " -f1 <"$gsum")
  tmp=$(mktemp)
  i=0
  while [ "$i" -lt "$N" ]; do
    rm -f "$REPO/$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
    "$MFB" build -q -ncode "$REPO/$td" >/dev/null 2>&1
    if [ -f "$REPO/$td/$pkg.ncode" ]; then
      shasum -a 256 "$REPO/$td/$pkg.ncode" | cut -d" " -f1 >>"$tmp"
    else
      echo "MISSING .ncode after build for $pkg" >&2
    fi
    i=$((i+1))
  done
  rm -f "$REPO/$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
  uniq_lines=$(sort -u "$tmp")
  uniq_n=$(printf "%s\n" "$uniq_lines" | grep -c .)
  first=$(head -1 "$tmp")
  if [ "$uniq_n" -gt 1 ]; then
    class=FLAKY
  elif [ -n "$gold" ] && [ "$first" = "$gold" ]; then
    class=CLEAN
  elif [ -n "$gold" ]; then
    class=STALE-GOLD
  else
    class=NO-GOLD
  fi
  # compact hash listing with counts
  listing=$(sort "$tmp" | uniq -c | awk '{printf "%s:%d ", substr($2,1,8), $1}')
  printf "%-42s %6s %-10s %s\n" "$pkg" "$uniq_n" "$class" "$listing"
  rm -f "$tmp"
done
