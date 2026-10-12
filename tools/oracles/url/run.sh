#!/usr/bin/env bash
# Differential oracle: MFBASIC's URL handling (`net::toUrl`, `toString(Url)`,
# `net::percentDecode`, `net::parseQuery`, `encoding::percent*` / `formUrl*`) against
# Node's WHATWG `URL` / `URLSearchParams` / `decodeURIComponent` / `encodeURIComponent`,
# and the `http` client's and server's chunked de-framing against Node's llhttp.
#
# For each string mode node/gen.mjs writes a deterministic case list; the MFB subject
# (mfb/) and the Node judge (node/oracle.mjs) answer every case; node/compare.mjs sorts
# each disagreement into a named group whose class (a bug / b spec-allowed / c owner
# decision) is declared in compare.mjs. The chunk modes are driven by node/chunked.mjs
# over 127.0.0.1. The subject is built from a copy under $WORK, never in the repo.
#
# Usage: ./run.sh [path-to-mfb-binary] [mode ...]     (no modes = every default mode)
#   MFB_URL_WPT=<urltestdata.json>  also run the `wpt` mode over that file (must be the
#                                   pinned copy, see README)
#   MFB_URL_KEEP=<dir>              copy every case list, answer and report there
# Exit: 0 every disagreement is class b, 1 a class a / c / unclassified one, 2 harness failure.
set -uo pipefail
. "$(dirname "$0")/../crypto/_lib/harness.sh"

ALL_MODES="parse pdecode query encode chunk-client chunk-server"

# Case counts per mode, declared HERE rather than counted from any side's output.
# node/gen.mjs's own count is checked against these.
expected_cases() {
  case $1 in
    parse) echo 1955 ;; # 429 corpus/parse.txt + 10 positions x 142 code points + 32 ports + 60 IPv4 forms + 14 long inputs
    pdecode) echo 613 ;; # 101 corpus/pdecode.txt + 256 `%XX` + 15 x 15 `%` pairs + 31 encodeURIComponent outputs
    query) echo 202 ;; # 74 corpus/query.txt + 128 code points in key and value
    encode) echo 168 ;; # 31 corpus/encode.txt + 128 ASCII + 9 Unicode code points
    chunk-client) echo 49 ;; # corpus/chunked.txt as response bodies
    chunk-server) echo 49 ;; # corpus/chunked.txt as request bodies
    wpt) echo 746 ;; # distinct inputs of the pinned urltestdata.json, lone surrogates dropped
    *) echo 0 ;;
  esac
}
WPT_SHA256=58d67bea710d5f46324fe6841df5fd82090fe4ec2d882bc0fc7c1784d4771884

MFB_ARG=""
if [ $# -gt 0 ] && [ -f "$1" ]; then
  MFB_ARG=$1
  shift
fi
oracle_init "$0" ${MFB_ARG:+"$MFB_ARG"}
MODES=${*:-$ALL_MODES}
if [ -n "${MFB_URL_WPT:-}" ] && [ $# -eq 0 ]; then MODES="$MODES wpt"; fi

command -v node >/dev/null || die "node is required"
say "node $(node --version)"

# Build a copy of mfb/ under $WORK (oracle_build_mfb would build inside the repo).
say "building mfb/ in $WORK"
cp -R "$HERE/mfb" "$WORK/mfb" || die "cannot copy mfb/"
build_log=$("$MFB_EXE" build -q "$WORK/mfb" 2>&1) || {
  printf '%s\n' "$build_log" >&2
  die "mfb build failed"
}
SUBJECT=""
while IFS= read -r candidate; do
  [ -x "$candidate" ] || continue
  if "$candidate" >/dev/null 2>&1; then SUBJECT=$candidate; break; fi
done <<EOF
$(printf '%s\n' "$build_log" | sed -n 's/^Wrote executable to //p')
EOF
[ -n "$SUBJECT" ] || { printf '%s\n' "$build_log" >&2; die "no built flavor ran"; }
say "ran $SUBJECT"

ran_total=0
fail_total=0
expected_total=0
for mode in $MODES; do
  expected=$(expected_cases "$mode")
  [ "$expected" -gt 0 ] || die "unknown mode: $mode (modes: $ALL_MODES wpt)"
  expected_total=$((expected_total + expected))
  wpt=""
  if [ "$mode" = wpt ]; then
    wpt=${MFB_URL_WPT:-}
    [ -f "$wpt" ] || die "wpt mode needs MFB_URL_WPT=<urltestdata.json>"
    [ "$(shasum -a 256 "$wpt" | cut -d' ' -f1)" = "$WPT_SHA256" ] ||
      die "$wpt is not the pinned urltestdata.json (sha256 $WPT_SHA256); update WPT_SHA256 and the wpt count together"
  fi
  generated=$(node "$HERE/node/gen.mjs" "$mode" "$WORK" ${wpt:+"$wpt"}) || die "gen.mjs failed for $mode"
  [ "$generated" = "$expected" ] ||
    die "gen.mjs wrote $generated case(s) for $mode but run.sh declares $expected"

  cases="$WORK/$mode.cases.json"
  case $mode in
    chunk-client | chunk-server)
      node "$HERE/node/chunked.mjs" "${mode#chunk-}" "$cases" "$SUBJECT" "$WORK/$mode.mfb.txt" "$WORK/$mode.node.txt" ||
        die "node/chunked.mjs failed on $mode"
      ;;
    *)
      MFB_URL_MODE="$mode" MFB_URL_JOB="$WORK/$mode.job" "$SUBJECT" >"$WORK/$mode.mfb.txt" 2>"$WORK/$mode.mfb.err" || {
        cat "$WORK/$mode.mfb.err" >&2
        die "the MFB subject failed on $mode"
      }
      node "$HERE/node/oracle.mjs" "$mode" "$cases" >"$WORK/$mode.node.txt" || die "node/oracle.mjs failed on $mode"
      ;;
  esac
  ran=$(grep -c '^case ' "$WORK/$mode.mfb.txt")
  ran_total=$((ran_total + ran))
  node "$HERE/node/compare.mjs" "$mode" "$cases" "$WORK/$mode.mfb.txt" "$WORK/$mode.node.txt" "$WORK/$mode.report.json"
  status=$?
  [ "$status" -le 1 ] || die "compare.mjs could not judge $mode"
  [ "$status" -eq 0 ] || fail_total=$((fail_total + 1))
done

if [ -n "${MFB_URL_KEEP:-}" ]; then
  mkdir -p "$MFB_URL_KEEP" && cp "$WORK"/*.json "$WORK"/*.txt "$MFB_URL_KEEP"/ && say "kept results in $MFB_URL_KEEP"
fi
oracle_verdict "url mfb-vs-node ($MODES) -- failures are modes with a class a / c divergence" "$ran_total" "$fail_total" "$expected_total"
