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

if [ -n "${MFB_TARGET:-}" ]; then
  target_name="$MFB_TARGET"
  target_arg="-target $MFB_TARGET"
  target_label="$target_arg "
else
  host_os="$(uname -s)"
  case "$host_os" in
    Darwin)
      target_os="macos"
      ;;
    Linux)
      target_os="linux"
      ;;
    MINGW* | MSYS* | CYGWIN*)
      target_os="windows"
      ;;
    *)
      target_os="$(printf '%s' "$host_os" | tr '[:upper:]' '[:lower:]')"
      ;;
  esac

  host_arch="$(uname -m)"
  case "$host_arch" in
    arm64)
      target_arch="aarch64"
      ;;
    x86_64 | amd64)
      target_arch="x86_64"
      ;;
    *)
      target_arch="$host_arch"
      ;;
  esac

  target_name="$target_os-$target_arch"
  target_arg=""
  target_label=""
fi

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
  mfp_path="$test_dir/$package_name.mfp"
  nir_path="$test_dir/$package_name.nir"
  nplan_path="$test_dir/$package_name.nplan"
  nobj_path="$test_dir/$package_name.nobj"
  ncode_path="$test_dir/$package_name.ncode"
  target_nir_path="$test_dir/$package_name.$target_name.nir"
  target_nplan_path="$test_dir/$package_name.$target_name.nplan"
  target_nobj_path="$test_dir/$package_name.$target_name.nobj"
  target_ncode_path="$test_dir/$package_name.$target_name.ncode"

  rm -f "$ast_path" "$ir_path" "$hex_path" "$mfp_path" "$nir_path" "$nplan_path" "$nobj_path" "$ncode_path" "$target_nir_path" "$target_nplan_path" "$target_nobj_path" "$target_ncode_path" "$test_dir/$package_name.out"

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
    if [ -f "$golden_dir/$package_name.mfp" ] || [ -f "$golden_dir/$package_name.info" ]; then
      echo "$ mfb build tests/$test_name"
      "$MFB_EXE" build "tests/$test_name"
      echo "[exit $?]"
    fi
    if [ -f "$golden_dir/$package_name.$target_name.nir" ]; then
      echo "$ mfb build ${target_label}-nir tests/$test_name"
      "$MFB_EXE" build $target_arg -nir "tests/$test_name"
      echo "[exit $?]"
    fi
    if [ -f "$golden_dir/$package_name.$target_name.nplan" ]; then
      echo "$ mfb build ${target_label}-nplan tests/$test_name"
      "$MFB_EXE" build $target_arg -nplan "tests/$test_name"
      echo "[exit $?]"
    fi
    if [ -f "$golden_dir/$package_name.$target_name.nobj" ]; then
      echo "$ mfb build ${target_label}-nobj tests/$test_name"
      "$MFB_EXE" build $target_arg -nobj "tests/$test_name"
      echo "[exit $?]"
    fi
    if [ -f "$golden_dir/$package_name.$target_name.ncode" ]; then
      echo "$ mfb build ${target_label}-ncode tests/$test_name"
      "$MFB_EXE" build $target_arg -ncode "tests/$test_name"
      echo "[exit $?]"
    fi
    if [ -f "$golden_dir/$package_name.run" ]; then
      echo "$ mfb build ${target_label}tests/$test_name"
      build_output=$("$MFB_EXE" build $target_arg "tests/$test_name" 2>&1)
      build_status=$?
      printf '%s\n' "$build_output"
      echo "[exit $build_status]"
      if [ "$build_status" -eq 0 ]; then
        run_path=$(printf '%s\n' "$build_output" | sed -n 's/^Wrote executable to //p' | tail -n 1)
        if [ -n "$run_path" ]; then
          echo "$ $run_path"
          "$run_path"
          echo "[exit $?]"
        else
          echo "error: build did not report an executable path"
          echo "[exit 1]"
        fi
      fi
    fi
  } >"$log_path" 2>&1
  rm -f "$test_dir/$package_name.out"

  if [ -f "$ast_path" ]; then
    mv "$ast_path" "$actual_dir/$package_name.ast"
  fi
  if [ -f "$ir_path" ]; then
    mv "$ir_path" "$actual_dir/$package_name.ir"
  fi
  if [ -f "$hex_path" ]; then
    mv "$hex_path" "$actual_dir/$package_name.hex"
  fi
  if [ -f "$golden_dir/$package_name.info" ] && [ -f "$mfp_path" ]; then
    "$MFB_EXE" pkg info "tests/$test_name/$package_name.mfp" >"$actual_dir/$package_name.info" 2>&1
  fi
  if [ -f "$mfp_path" ]; then
    mv "$mfp_path" "$actual_dir/$package_name.mfp"
  fi
  if [ -f "$nir_path" ]; then
    mv "$nir_path" "$actual_dir/$package_name.$target_name.nir"
  fi
  if [ -f "$nplan_path" ]; then
    mv "$nplan_path" "$actual_dir/$package_name.$target_name.nplan"
  fi
  if [ -f "$nobj_path" ]; then
    mv "$nobj_path" "$actual_dir/$package_name.$target_name.nobj"
  fi
  if [ -f "$ncode_path" ]; then
    mv "$ncode_path" "$actual_dir/$package_name.$target_name.ncode"
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
  compare_optional_output "$test_name/$package_name.mfp" \
    "$golden_dir/$package_name.mfp" \
    "$actual_dir/$package_name.mfp"
  compare_optional_output "$test_name/$package_name.info" \
    "$golden_dir/$package_name.info" \
    "$actual_dir/$package_name.info"
  compare_optional_output "$test_name/$package_name.$target_name.nir" \
    "$golden_dir/$package_name.$target_name.nir" \
    "$actual_dir/$package_name.$target_name.nir"
  compare_optional_output "$test_name/$package_name.$target_name.nplan" \
    "$golden_dir/$package_name.$target_name.nplan" \
    "$actual_dir/$package_name.$target_name.nplan"
  compare_optional_output "$test_name/$package_name.$target_name.nobj" \
    "$golden_dir/$package_name.$target_name.nobj" \
    "$actual_dir/$package_name.$target_name.nobj"
  compare_optional_output "$test_name/$package_name.$target_name.ncode" \
    "$golden_dir/$package_name.$target_name.ncode" \
    "$actual_dir/$package_name.$target_name.ncode"
done

if [ "$failures" -ne 0 ]; then
  echo "acceptance tests failed: $failures mismatch(es)" >&2
  exit 1
fi

echo "acceptance tests passed"
