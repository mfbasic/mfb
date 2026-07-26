#!/usr/bin/env bash
# bug-388 Phase 1/3 cross-target determinism harness.
#
# For each in-scope fixture and EACH of the four goldened targets, compile it N
# times in a FRESH `mfb` process (std HashMap seeds per process) and count the
# distinct sha256s of the emitted `.ncode`. Compares the stable hash against the
# committed `<target>.ncodesum` golden. `-ncode` is execution-free, so all four
# targets (incl. the three linux-* cross targets) regenerate on the macOS host.
#
# Usage: scripts/ncode-determinism-alltargets.sh <mfb-binary> [N]
set -u
MFB="$1"; N="${2:-50}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
host_arch="$(uname -m)"; case "$host_arch" in arm64) A=aarch64;; x86_64) A=x86_64;; *) A=$host_arch;; esac
case "$(uname -s)" in Darwin) HOST_TGT="macos-$A";; Linux) HOST_TGT="linux-$A";; *) HOST_TGT="unknown-$A";; esac

FIXTURES="
tests/byte-identity/audio audio_codegen_cover_rt
tests/byte-identity/crypto crypto_codegen_cover_rt
tests/byte-identity/fs fs_codegen_cover_rt
tests/byte-identity/net net_codegen_cover_rt
tests/byte-identity/os os_codegen_cover_rt
tests/byte-identity/tls tls_codegen_cover_rt
tests/rt-behavior/crypto/crypto-ec-valid crypto-ec-valid
"
TARGETS="macos-aarch64 linux-aarch64 linux-x86_64 linux-riscv64"

printf "%-40s %-16s %5s %-11s %s\n" FIXTURE TARGET UNIQ CLASS "hash(count)"
echo "$FIXTURES" | while read -r td pkg; do
  [ -n "$td" ] || continue
  for tgt in $TARGETS; do
    gsum="$REPO/$td/golden/$pkg.$tgt.ncodesum"
    [ -f "$gsum" ] || { printf "%-40s %-16s %5s %-11s\n" "$pkg" "$tgt" - NO-GOLDFILE; continue; }
    gold=$(cut -d" " -f1 <"$gsum")
    targ=""; [ "$tgt" = "$HOST_TGT" ] || targ="-target $tgt"
    tmp=$(mktemp); i=0
    while [ "$i" -lt "$N" ]; do
      rm -f "$REPO/$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
      # shellcheck disable=SC2086
      "$MFB" build -q -ncode $targ "$REPO/$td" >/dev/null 2>&1
      [ -f "$REPO/$td/$pkg.ncode" ] && shasum -a 256 "$REPO/$td/$pkg.ncode" | cut -d" " -f1 >>"$tmp"
      i=$((i+1))
    done
    rm -f "$REPO/$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
    uniq_n=$(sort -u "$tmp" | grep -c .)
    first=$(head -1 "$tmp")
    if [ "$uniq_n" -gt 1 ]; then class=FLAKY
    elif [ "$first" = "$gold" ]; then class=CLEAN
    else class=STALE-GOLD; fi
    listing=$(sort "$tmp" | uniq -c | awk '{printf "%s:%d ", substr($2,1,8), $1}')
    printf "%-40s %-16s %5s %-11s %s\n" "$pkg" "$tgt" "$uniq_n" "$class" "$listing"
    rm -f "$tmp"
  done
done
