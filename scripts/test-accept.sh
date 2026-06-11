#!/usr/bin/env bash
set -u

if [ "$#" -lt 2 ]; then
  echo "usage: test-accept.sh <mfb-exe> <actual-output-dir>" >&2
  exit 2
fi

MFB_EXE=$1
ACTUAL_ROOT=$2
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TEST_ROOT="$ROOT/tests"

host_arch="$(uname -m)"
case "$host_arch" in
  arm64)
    bin_arch="aarch64"
    ;;
  *)
    bin_arch="$host_arch"
    ;;
esac

failures=0

project_name() {
  sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$1/project.json" | head -n 1
}

compare_file() {
  local label=$1
  local expected=$2
  local actual=$3

  if [ ! -f "$expected" ]; then
    echo "missing golden: $expected" >&2
    failures=$((failures + 1))
    return
  fi

  if [ ! -f "$actual" ]; then
    echo "missing actual $label: $actual" >&2
    failures=$((failures + 1))
    return
  fi

  if ! diff -u "$expected" "$actual"; then
    echo "mismatch: $label" >&2
    failures=$((failures + 1))
  fi
}

compare_optional_output() {
  local label=$1
  local expected=$2
  local actual=$3

  if [ -f "$expected" ]; then
    compare_file "$label" "$expected" "$actual"
    return
  fi

  if [ -f "$actual" ]; then
    echo "unexpected actual $label: $actual" >&2
    failures=$((failures + 1))
  fi
}

rm -rf "$ACTUAL_ROOT"
mkdir -p "$ACTUAL_ROOT"

cd "$ROOT" || exit 2

for test_dir in "$TEST_ROOT"/*; do
  [ -d "$test_dir" ] || continue
  [ -f "$test_dir/project.json" ] || continue

  test_name=$(basename "$test_dir")
  package_name=$(project_name "$test_dir")
  if [ -z "$package_name" ]; then
    echo "could not read project name for $test_name" >&2
    failures=$((failures + 1))
    continue
  fi

  golden_dir="$test_dir/golden"
  actual_dir="$ACTUAL_ROOT/$test_name"
  mkdir -p "$actual_dir"

  log_path="$actual_dir/build.log"
  ast_path="$test_dir/$package_name.ast"
  ir_path="$test_dir/$package_name.ir"
  hex_path="$test_dir/$package_name.hex"
  bin_path="$test_dir/$package_name.$bin_arch.bin"

  rm -f "$ast_path" "$ir_path" "$hex_path" "$bin_path" "$test_dir/$package_name.out"

  {
    echo "$ mfb build -ast tests/$test_name"
    "$MFB_EXE" build -ast "tests/$test_name"
    echo "[exit $?]"
    echo "$ mfb build -ir tests/$test_name"
    "$MFB_EXE" build -ir "tests/$test_name"
    echo "[exit $?]"
    if [ -f "$golden_dir/$package_name.hex" ]; then
      echo "$ mfb build -bc tests/$test_name"
      "$MFB_EXE" build -bc "tests/$test_name"
      echo "[exit $?]"
    fi
    if [ -f "$golden_dir/$package_name.$bin_arch.bin" ]; then
      echo "$ mfb build -bin tests/$test_name"
      "$MFB_EXE" build -bin "tests/$test_name"
      echo "[exit $?]"
    fi
  } >"$log_path" 2>&1

  if [ -f "$ast_path" ]; then
    mv "$ast_path" "$actual_dir/$package_name.ast"
  fi
  if [ -f "$ir_path" ]; then
    mv "$ir_path" "$actual_dir/$package_name.ir"
  fi
  if [ -f "$hex_path" ]; then
    mv "$hex_path" "$actual_dir/$package_name.hex"
  fi
  if [ -f "$bin_path" ]; then
    mv "$bin_path" "$actual_dir/$package_name.$bin_arch.bin"
  fi

  compare_file "$test_name/build.log" "$golden_dir/build.log" "$log_path"
  compare_optional_output "$test_name/$package_name.ast" \
    "$golden_dir/$package_name.ast" \
    "$actual_dir/$package_name.ast"
  compare_optional_output "$test_name/$package_name.ir" \
    "$golden_dir/$package_name.ir" \
    "$actual_dir/$package_name.ir"
  compare_optional_output "$test_name/$package_name.hex" \
    "$golden_dir/$package_name.hex" \
    "$actual_dir/$package_name.hex"
  compare_optional_output "$test_name/$package_name.$bin_arch.bin" \
    "$golden_dir/$package_name.$bin_arch.bin" \
    "$actual_dir/$package_name.$bin_arch.bin"
done

if [ "$failures" -ne 0 ]; then
  echo "acceptance tests failed: $failures mismatch(es)" >&2
  exit 1
fi

echo "acceptance tests passed"
