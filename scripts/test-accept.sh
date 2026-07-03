#!/usr/bin/env bash
set -u

if [ "$#" -lt 2 ]; then
  echo "usage: test-accept.sh <mfb-exe> <actual-output-dir> [name-glob ...]" >&2
  echo "  name-glob: optional shell glob(s) matched against each test dir name;" >&2
  echo "             when given, only matching tests run (e.g. 'collection-*' 'func_math_*')." >&2
  exit 2
fi

MFB_EXE=$1
ACTUAL_ROOT=$2
shift 2
FILTERS=("$@")
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TEST_ROOT="$ROOT/tests"

# Returns 0 if $1 (relative test path) or its basename matches any filter glob,
# or if no filters were given.
matches_filter() {
  [ "${#FILTERS[@]}" -eq 0 ] && return 0
  local name=$1 pat base
  base=$(basename "$name")
  for pat in "${FILTERS[@]}"; do
    # shellcheck disable=SC2254
    case "$name" in
      $pat) return 0 ;;
    esac
    # shellcheck disable=SC2254
    case "$base" in
      $pat) return 0 ;;
    esac
  done
  return 1
}

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
ran=0

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

# Top-level tests plus grouped suites (e.g. tests/security/*) one level down.
for test_dir in "$TEST_ROOT"/* "$TEST_ROOT"/security/*; do
  [ -d "$test_dir" ] || continue
  [ -f "$test_dir/project.json" ] || continue

  test_name=${test_dir#"$TEST_ROOT/"}
  matches_filter "$test_name" || continue
  ran=$((ran + 1))
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
  mir_path="$test_dir/$package_name.mir"
  target_mir_path="$test_dir/$package_name.$target_name.mir"
  # macOS app-mode (`mfb build -app`) native goldens. App-mode `-nir/-nplan/-ncode`
  # write to the same `$package_name.{nir,nplan,ncode}` paths as console mode, so a
  # fixture carries either console or app goldens for a given extension, never both.
  target_app_nir_path="$test_dir/$package_name.$target_name.app.nir"
  target_app_nplan_path="$test_dir/$package_name.$target_name.app.nplan"
  target_app_ncode_path="$test_dir/$package_name.$target_name.app.ncode"

  rm -f "$ast_path" "$ir_path" "$hex_path" "$mfp_path" "$nir_path" "$nplan_path" "$nobj_path" "$ncode_path" "$mir_path" "$target_nir_path" "$target_nplan_path" "$target_nobj_path" "$target_ncode_path" "$target_mir_path" "$target_app_nir_path" "$target_app_nplan_path" "$target_app_ncode_path" "$test_dir/$package_name.out"

  {
    # Batch the artifact dumps: `mfb build` output flags combine, so one
    # invocation per flag family shares a single front-end pass instead of
    # re-parsing/resolving/typechecking the project once per artifact.
    console_flags="-ast -ir"
    if [ -f "$golden_dir/$package_name.hex" ]; then
      console_flags="$console_flags -br"
    fi
    if [ -f "$golden_dir/$package_name.$target_name.nir" ]; then
      console_flags="$console_flags -nir"
    fi
    if [ -f "$golden_dir/$package_name.$target_name.nplan" ]; then
      console_flags="$console_flags -nplan"
    fi
    if [ -f "$golden_dir/$package_name.$target_name.nobj" ]; then
      console_flags="$console_flags -nobj"
    fi
    if [ -f "$golden_dir/$package_name.$target_name.ncode" ]; then
      console_flags="$console_flags -ncode"
    fi
    if [ -f "$golden_dir/$package_name.$target_name.mir" ]; then
      console_flags="$console_flags -mir"
    fi
    echo "$ mfb build ${target_label}${console_flags} tests/$test_name"
    # shellcheck disable=SC2086
    "$MFB_EXE" build $target_arg $console_flags "tests/$test_name"
    echo "[exit $?]"
    if [ -f "$golden_dir/$package_name.mfp" ] || [ -f "$golden_dir/$package_name.info" ]; then
      echo "$ mfb build tests/$test_name"
      "$MFB_EXE" build "tests/$test_name"
      echo "[exit $?]"
    fi
    app_flags=""
    if [ -f "$golden_dir/$package_name.$target_name.app.nir" ]; then
      app_flags="$app_flags -nir"
    fi
    if [ -f "$golden_dir/$package_name.$target_name.app.nplan" ]; then
      app_flags="$app_flags -nplan"
    fi
    if [ -f "$golden_dir/$package_name.$target_name.app.ncode" ]; then
      app_flags="$app_flags -ncode"
    fi
    if [ -n "$app_flags" ]; then
      echo "$ mfb build ${target_label}-app${app_flags} tests/$test_name"
      # shellcheck disable=SC2086
      "$MFB_EXE" build $target_arg -app $app_flags "tests/$test_name"
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
    if [ -f "$golden_dir/$package_name.$target_name.app.nir" ]; then
      mv "$nir_path" "$actual_dir/$package_name.$target_name.app.nir"
    else
      mv "$nir_path" "$actual_dir/$package_name.$target_name.nir"
    fi
  fi
  if [ -f "$nplan_path" ]; then
    if [ -f "$golden_dir/$package_name.$target_name.app.nplan" ]; then
      mv "$nplan_path" "$actual_dir/$package_name.$target_name.app.nplan"
    else
      mv "$nplan_path" "$actual_dir/$package_name.$target_name.nplan"
    fi
  fi
  if [ -f "$nobj_path" ]; then
    mv "$nobj_path" "$actual_dir/$package_name.$target_name.nobj"
  fi
  if [ -f "$ncode_path" ]; then
    if [ -f "$golden_dir/$package_name.$target_name.app.ncode" ]; then
      mv "$ncode_path" "$actual_dir/$package_name.$target_name.app.ncode"
    else
      mv "$ncode_path" "$actual_dir/$package_name.$target_name.ncode"
    fi
  fi
  if [ -f "$mir_path" ]; then
    mv "$mir_path" "$actual_dir/$package_name.$target_name.mir"
  fi

  audit_path="$actual_dir/$package_name.audit"
  if [ -f "$golden_dir/$package_name.audit" ]; then
    : >"$audit_path"
    if [ -f "$test_dir/audit.args" ]; then
      while IFS= read -r argline || [ -n "$argline" ]; do
        [ -z "$argline" ] && continue
        {
          echo "\$ mfb audit $argline tests/$test_name"
          # shellcheck disable=SC2086
          "$MFB_EXE" audit $argline "tests/$test_name" 2>&1
          echo "[exit $?]"
        } >>"$audit_path"
      done <"$test_dir/audit.args"
    else
      {
        echo "\$ mfb audit --format text tests/$test_name"
        "$MFB_EXE" audit --format text "tests/$test_name" 2>&1
        echo "[exit $?]"
        echo "\$ mfb audit --format json tests/$test_name"
        "$MFB_EXE" audit --format json "tests/$test_name" 2>&1
        echo "[exit $?]"
      } >>"$audit_path"
    fi
  fi

  compare_file "$test_name/build.log" "$golden_dir/build.log" "$log_path"
  compare_optional_output "$test_name/$package_name.audit" \
    "$golden_dir/$package_name.audit" \
    "$audit_path"
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
  compare_optional_output "$test_name/$package_name.$target_name.mir" \
    "$golden_dir/$package_name.$target_name.mir" \
    "$actual_dir/$package_name.$target_name.mir"
  compare_optional_output "$test_name/$package_name.$target_name.app.nir" \
    "$golden_dir/$package_name.$target_name.app.nir" \
    "$actual_dir/$package_name.$target_name.app.nir"
  compare_optional_output "$test_name/$package_name.$target_name.app.nplan" \
    "$golden_dir/$package_name.$target_name.app.nplan" \
    "$actual_dir/$package_name.$target_name.app.nplan"
  compare_optional_output "$test_name/$package_name.$target_name.app.ncode" \
    "$golden_dir/$package_name.$target_name.app.ncode" \
    "$actual_dir/$package_name.$target_name.app.ncode"
done

if [ "${#FILTERS[@]}" -ne 0 ] && [ "$ran" -eq 0 ]; then
  echo "no tests matched filter: ${FILTERS[*]}" >&2
  exit 2
fi

if [ "$failures" -ne 0 ]; then
  echo "acceptance tests failed: $failures mismatch(es) ($ran test(s) ran)" >&2
  exit 1
fi

echo "acceptance tests passed ($ran test(s) ran)"
