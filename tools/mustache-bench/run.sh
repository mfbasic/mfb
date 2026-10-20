#!/usr/bin/env bash
# Scaling of mustache::render with the size of its context. See README.md.
#
# Usage: tools/mustache-bench/run.sh [path-to-mfb]
# Env:   SIZES (default "4000 16000 64000")
#
# For each render shape and each size, builds a probe against the package in
# packages/mustache, renders five times in process (timed with
# datetime::monotonicNanos around the render alone), and reports the median. A
# shape passes when each step up in size costs at most 4.4x the step before it:
# 4x the data should cost about 4x the time, and a quadratic render costs 16x.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MFB="${1:-$ROOT/target/release/mfb}"
SIZES="${SIZES:-4000 16000 64000}"
[ -x "$MFB" ] || { echo "no compiler at $MFB; pass one, or run: cargo build --release --bin mfb" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# name | template | partials statement | context builder (MFB String expression over `items`)
SHAPES=(
  "flat|{{#items}}<li>{{name}}</li>\\n{{/items}}|NOTHING|\"{\\\"items\\\":[\" & items & \"]}\""
  "wrapped|{{#data}}{{#items}}<li>{{name}}</li>{{/items}}{{/data}}|NOTHING|\"{\\\"data\\\":{\\\"items\\\":[\" & items & \"]}}\""
  "partial|{{#items}}{{>row}}{{/items}}|partials = collections::set(partials, \"row\", \"<li>{{name}}</li>\\\\n\")|\"{\\\"items\\\":[\" & items & \"]}\""
  "outer|{{#items}}{{site}} {{name}}\\n{{/items}}|NOTHING|\"{\\\"site\\\":\\\"s\\\",\\\"items\\\":[\" & items & \"]}\""
)

# A probe that renders the shape over N items five times and prints the median
# nanoseconds and the output's byte length.
probe() {
  local dir="$1" template="$2" partials="$3" context="$4" n="$5"
  mkdir -p "$dir/src"
  cat > "$dir/project.json" <<JSON
{"name":"mustachebench","version":"0.1.0","mfb":"1.0","kind":"executable","description":"mustache scaling probe",
"sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],
"packages":[{"name":"mustache","version":"=1.0.0","source":"local://$ROOT/packages/mustache","direct":true,"requiredBy":[]}],
"entry":"main","targets":["native"]}
JSON
  cat > "$dir/src/main.mfb" <<MFB
IMPORT mustache
IMPORT json
IMPORT io
IMPORT datetime
IMPORT collections
IMPORT strings

FUNC main() AS Integer
  MUT items AS String = ""
  FOR i = 1 TO $n
    IF i > 1 THEN items = items & ","
    items = items & "{\\"name\\":\\"n" & toString(i) & "\\"}"
  NEXT
  LET ctx AS json::Json = json::parse($context)
  MUT partials AS Map OF String TO String = Map OF String TO String {}
  $partials
  MUT times AS List OF Integer = []
  MUT size AS Integer = 0
  FOR run = 1 TO 5
    LET start AS Integer = datetime::monotonicNanos()
    LET out AS String = mustache::renderWith("$template", ctx, partials)
    times = collections::append(times, datetime::monotonicNanos() - start)
    size = strings::byteLen(out)
  NEXT
  LET sorted AS List OF Integer = collections::sort(times)
  io::print(toString(collections::get(sorted, 2)) & " " & toString(size))
  RETURN 0
END FUNC
MFB
}

failures=0
for shape in "${SHAPES[@]}"; do
  IFS='|' read -r name template partials context <<< "$shape"
  prev=""
  line="$name:"
  for n in $SIZES; do
    dir="$WORK/$name-$n"
    probe "$dir" "$template" "$partials" "$context" "$n"
    "$MFB" build -q "$dir" > "$dir/build.log" 2>&1 || { echo "BUILD FAILED: $name $n"; cat "$dir/build.log"; exit 1; }
    result="$("$dir/build/mustachebench.out")"
    read -r nanos bytes <<< "$result"
    line="$line  n=$n ${nanos}ns (${bytes}B)"
    if [ -n "$prev" ]; then
      ratio="$(awk -v a="$nanos" -v b="$prev" 'BEGIN { printf "%.2f", a / b }')"
      line="$line  step x$ratio"
      if awk -v r="$ratio" 'BEGIN { exit !(r > 4.4) }'; then
        failures=$((failures + 1))
        line="$line  NOT LINEAR"
      fi
    fi
    prev="$nanos"
  done
  echo "$line"
done

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures step(s) grew faster than linear"
  exit 1
fi
echo "every shape scales linearly"
