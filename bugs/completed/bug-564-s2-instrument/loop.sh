#!/usr/bin/env bash
# bug-564 s2 driver: run BIN N times with P concurrent, tally stdout lines.
# usage: [B564_DYLIB=... B564_*=...] loop.sh BIN N P OUTDIR
# SIP strips DYLD_* from /bin/bash and /usr/bin/xargs, so the dylib is passed as
# B564_DYLIB and injected right at the fixture's exec.
set -u
BIN=$1; N=$2; P=$3; OUT=$4
rm -rf "$OUT"; mkdir -p "$OUT"
export BIN OUT
seq 1 "$N" | xargs -P "$P" -I{} /bin/bash -c 'if [ -n "${B564_DYLIB:-}" ]; then DYLD_INSERT_LIBRARIES=$B564_DYLIB "$BIN" >"$OUT/run-{}.out" 2>"$OUT/run-{}.err"; else "$BIN" >"$OUT/run-{}.out" 2>"$OUT/run-{}.err"; fi; echo "exit=$?" >>"$OUT/run-{}.out"'
echo "== tally $OUT"
cat "$OUT"/run-*.out | sort | uniq -c
echo "== done"
