#!/usr/bin/env bash
# tools/browser-fetch-probe/run.sh — bug-706 regression check.
#
# Reproduces and checks the fix for both bug-706 sub-issues against a live
# build of examples/browser/{dom,fetch}:
#
#   1. fetchStyles's drive loop must give up on a stalled stylesheet (a
#      connection that accepts and then never writes/closes) within a bounded
#      time and without busy-spinning the CPU, the same way it already gives
#      up on a stylesheet that fails to connect.
#   2. resolveLocation/resolveHref must resolve five href shapes (absolute,
#      root-relative, query-only, fragment-only, "../"-relative, and plain
#      relative) exactly per RFC 3986 §5.3 merge + remove-dot-segments.
#
# Usage:
#   tools/browser-fetch-probe/run.sh [path-to-mfb]
#
# Defaults to <repo-root>/target/release/mfb if no path is given. Exits 0 and
# prints "PASS" for both checks on success; prints the mismatch and exits
# non-zero on failure. Builds everything into a scratch directory under
# mktemp -d; nothing here or under examples/ is modified.

set -u
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
MFB="${1:-$REPO_ROOT/target/release/mfb}"

if [ ! -x "$MFB" ]; then
  echo "browser-fetch-probe: no mfb binary at $MFB (build it with: cargo build --release --bin mfb)" >&2
  exit 2
fi

WORK="$(mktemp -d /tmp/browser-fetch-probe.XXXXXX)"
PAGE_PORT=0
HOLE_PORT=0
PAGE_PID=""
HOLE_PID=""

cleanup() {
  [ -n "$PAGE_PID" ] && kill "$PAGE_PID" >/dev/null 2>&1
  [ -n "$HOLE_PID" ] && kill "$HOLE_PID" >/dev/null 2>&1
  [ -n "$PAGE_PID" ] && wait "$PAGE_PID" >/dev/null 2>&1
  [ -n "$HOLE_PID" ] && wait "$HOLE_PID" >/dev/null 2>&1
  rm -rf "$WORK"
}
trap cleanup EXIT

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

# --- Build dom -> fetch -> probe, each into this scratch dir. --------------
cp -r "$REPO_ROOT/examples/browser/dom" "$WORK/dom" || fail "copy dom"
"$MFB" build "$WORK/dom" >/dev/null || fail "build dom"

cp -r "$REPO_ROOT/examples/browser/fetch" "$WORK/fetch" || fail "copy fetch"
mkdir -p "$WORK/fetch/packages"
cp "$WORK/dom/dom.mfp" "$WORK/fetch/packages/dom.mfp"
"$MFB" build "$WORK/fetch" >/dev/null || fail "build fetch"

cp -r "$SCRIPT_DIR/src" "$WORK/probe-src"
mkdir -p "$WORK/probe/src" "$WORK/probe/packages"
cp -r "$WORK/probe-src/." "$WORK/probe/src/"
cp "$WORK/fetch/fetch.mfp" "$WORK/probe/packages/fetch.mfp"
cp "$WORK/dom/dom.mfp" "$WORK/probe/packages/dom.mfp"
cp "$SCRIPT_DIR/project.json" "$WORK/probe/project.json"
"$MFB" build "$WORK/probe" >/dev/null || fail "build probe"
PROBE="$WORK/probe/build/browser-fetch-probe.out"

# --- Sub-issue 2: resolveHref, five inputs vs. their RFC-3986-correct values.
# Expected values match `urllib.parse.urljoin` (a standard RFC 3986 §5.3
# implementation) for the same base + href pairs; the example's authority
# formatting always includes an explicit ":80"/":443" port (an unrelated,
# pre-existing quirk this bug does not touch), so expected values below carry
# it too, unlike a bare urljoin() call.
# Portable associative-array substitute: macOS ships bash 3.2, which has no
# `declare -A`. label:expected pairs, one per line.
expected_pairs='
a:http://example.com:80/redirect?next=http://evil.example/x
a2:http://example.com:80/a/b/search?next=http://evil.example/x
b1:http://example.com:80/a/b/page.html?new=2
b2:http://example.com:80/a/b/page.html?old=1#section
c:http://example.com:80/a/up.html
plain:http://example.com:80/a/b/other.html
'

resolve_out="$("$PROBE" resolve)" || fail "probe resolve exited non-zero"
resolve_fail=0
while IFS= read -r pair; do
  [ -z "$pair" ] && continue
  label="${pair%%:*}"
  want="${pair#*:}"
  actual="$(printf '%s\n' "$resolve_out" | grep "^resolve|$label|" | cut -d'|' -f3-)"
  if [ "$actual" != "$want" ]; then
    echo "resolveHref[$label]: got '$actual', want '$want'" >&2
    resolve_fail=1
  fi
done <<< "$expected_pairs"
if [ "$resolve_fail" -ne 0 ]; then
  fail "resolveHref mismatch (see above)"
fi
echo "PASS: resolveHref (5/5 RFC-3986-correct)"

# --- Sub-issue 1: fetchStyles must bound a stalled stylesheet fetch. -------
HOLE_PORT=$(( (RANDOM % 5000) + 20000 ))
PAGE_PORT=$(( HOLE_PORT + 1 ))

mkdir -p "$WORK/page"
cat > "$WORK/page/index.html" <<EOF
<html><head><title>Spin Test</title>
<link rel="stylesheet" href="http://127.0.0.1:$HOLE_PORT/blackhole.css">
</head><body><h1>hi</h1></body></html>
EOF

python3 "$SCRIPT_DIR/page/blackhole.py" "$HOLE_PORT" >"$WORK/blackhole.log" 2>&1 &
HOLE_PID=$!
( cd "$WORK/page" && python3 -m http.server "$PAGE_PORT" >"$WORK/pagesrv.log" 2>&1 ) &
PAGE_PID=$!
sleep 1

BEFORE=$(date +%s)
spin_out=$(/usr/bin/time -l "$PROBE" spin "http://127.0.0.1:$PAGE_PORT/index.html" 2>"$WORK/time.log")
rc=$?
AFTER=$(date +%s)
wall=$((AFTER - BEFORE))

[ "$rc" -eq 0 ] || fail "probe spin exited $rc"

# Bounded: fetchStyles's own 30s deadline plus generous slack for process
# startup/scheduling noise, well under what an unbounded busy-spin would take
# (it never returns at all).
if [ "$wall" -gt 45 ]; then
  fail "spin check took ${wall}s wall-clock (want <=45s) — looks like the busy-spin regressed"
fi

if ! printf '%s\n' "$spin_out" | grep -q "^spin|ok=TRUE|filesLoaded=1|filesTotal=2|"; then
  fail "spin check: unexpected LoadResult: $spin_out"
fi

# Confirm no busy-spin: /usr/bin/time -l reports "real" wall time and "user"
# CPU time separately; a spinning loop would burn CPU roughly equal to the
# wall-clock wait, a bounded-with-backoff loop burns well under a second.
user_secs=$(grep -Eo '^[ ]*[0-9]+\.[0-9]+ *real' "$WORK/time.log" | head -1 | awk '{print $1}')
cpu_secs=$(grep -Eo '[0-9]+\.[0-9]+ *user' "$WORK/time.log" | head -1 | awk '{print $1}')
if [ -n "$cpu_secs" ]; then
  over_budget=$(python3 -c "print(1 if float('$cpu_secs') > 3.0 else 0)" 2>/dev/null || echo 0)
  if [ "$over_budget" = "1" ]; then
    fail "spin check burned ${cpu_secs}s of CPU over ${wall}s wall-clock — looks like the busy-spin regressed (want << wall time)"
  fi
fi

echo "PASS: fetchStyles deadline (bounded at ${wall}s wall-clock, ok=TRUE, one sheet skipped)"
exit 0
