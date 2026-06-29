#!/usr/bin/env bash
# Byte-identical self-diff gate for the MIR layer (plan-00-A §4 Phase 3).
#
# For every buildable acceptance fixture, build the native code plan (`-ncode`)
# and — for executable projects — the final linked binary under both
# `-codegen direct` (today's no-MIR path) and `-codegen mir` (routed through the
# neutral MIR layer), and assert the two are byte-for-byte identical. This is
# the safety net that de-risks every later neutralization plan (B–G): the MIR
# rewrite must never change the AArch64 output on the target we already trust.
#
# Usage: codegen-selfdiff.sh <mfb-exe> [name-glob ...]
#   name-glob: optional shell glob(s) matched against each test dir name; when
#              given, only matching tests run (e.g. 'collection-*' 'float-*').
set -u

if [ "$#" -lt 1 ]; then
  echo "usage: codegen-selfdiff.sh <mfb-exe> [name-glob ...]" >&2
  exit 2
fi

MFB_EXE=$1
shift
FILTERS=("$@")
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TEST_ROOT="$ROOT/tests"

if [ -n "${MFB_TARGET:-}" ]; then
  target_arg=(-target "$MFB_TARGET")
else
  target_arg=()
fi

matches_filter() {
  [ "${#FILTERS[@]}" -eq 0 ] && return 0
  local name=$1 pat
  for pat in "${FILTERS[@]}"; do
    # shellcheck disable=SC2254
    case "$name" in
      $pat) return 0 ;;
    esac
  done
  return 1
}

project_name() {
  sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$1/project.json" | head -n 1
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cd "$ROOT" || exit 2

ran=0
checked=0
skipped=0
failures=0

for test_dir in "$TEST_ROOT"/*; do
  [ -d "$test_dir" ] || continue
  [ -f "$test_dir/project.json" ] || continue

  test_name=$(basename "$test_dir")
  matches_filter "$test_name" || continue
  ran=$((ran + 1))

  pkg=$(project_name "$test_dir")
  [ -n "$pkg" ] || { echo "skip $test_name: no project name" >&2; skipped=$((skipped + 1)); continue; }

  ncode_path="$test_dir/$pkg.ncode"
  out_glob=("$test_dir/$pkg".out)

  # Build the native code plan with the direct path. If it does not build (an
  # `*_invalid` fixture, a package project, or an unsupported target), skip this
  # fixture entirely — there is nothing to diff.
  rm -f "$ncode_path"
  if ! "$MFB_EXE" build ${target_arg[@]+"${target_arg[@]}"} -ncode -codegen direct "$test_dir" >/dev/null 2>&1 \
      || [ ! -f "$ncode_path" ]; then
    rm -f "$ncode_path"
    skipped=$((skipped + 1))
    continue
  fi
  cp "$ncode_path" "$work/$test_name.direct.ncode"

  rm -f "$ncode_path"
  if ! "$MFB_EXE" build ${target_arg[@]+"${target_arg[@]}"} -ncode -codegen mir "$test_dir" >/dev/null 2>&1 \
      || [ ! -f "$ncode_path" ]; then
    echo "FAIL $test_name: -codegen mir failed to produce a code plan" >&2
    rm -f "$ncode_path"
    failures=$((failures + 1))
    continue
  fi
  cp "$ncode_path" "$work/$test_name.mir.ncode"
  rm -f "$ncode_path"

  checked=$((checked + 1))
  if ! diff -q "$work/$test_name.direct.ncode" "$work/$test_name.mir.ncode" >/dev/null; then
    echo "FAIL $test_name: .ncode differs between -codegen direct and -codegen mir" >&2
    diff -u "$work/$test_name.direct.ncode" "$work/$test_name.mir.ncode" | head -40 >&2
    failures=$((failures + 1))
    continue
  fi

  # For executable fixtures, also diff the final linked binary. Only fixtures
  # with a `.run` golden are expected to link an executable; build both and
  # compare. A build that links no executable (library/package fixtures) is
  # silently skipped here.
  if [ -f "$test_dir/golden/$pkg.run" ]; then
    rm -f "${out_glob[@]}"
    direct_out=$("$MFB_EXE" build ${target_arg[@]+"${target_arg[@]}"} -codegen direct "$test_dir" 2>/dev/null \
      | sed -n 's/^Wrote executable to //p' | tail -n 1)
    if [ -n "$direct_out" ] && [ -f "$direct_out" ]; then
      cp "$direct_out" "$work/$test_name.direct.out"
      rm -f "${out_glob[@]}" "$direct_out"
      mir_out=$("$MFB_EXE" build ${target_arg[@]+"${target_arg[@]}"} -codegen mir "$test_dir" 2>/dev/null \
        | sed -n 's/^Wrote executable to //p' | tail -n 1)
      if [ -n "$mir_out" ] && [ -f "$mir_out" ]; then
        cp "$mir_out" "$work/$test_name.mir.out"
        rm -f "${out_glob[@]}" "$mir_out"
        if ! diff -q "$work/$test_name.direct.out" "$work/$test_name.mir.out" >/dev/null; then
          echo "FAIL $test_name: linked binary differs between -codegen direct and -codegen mir" >&2
          failures=$((failures + 1))
        fi
      else
        echo "FAIL $test_name: -codegen mir failed to link an executable" >&2
        failures=$((failures + 1))
      fi
    fi
    rm -f "${out_glob[@]}"
  fi
done

echo "self-diff: ran=$ran checked=$checked skipped=$skipped failures=$failures"
[ "$failures" -eq 0 ]
