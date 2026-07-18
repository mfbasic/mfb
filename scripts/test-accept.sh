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

# `run_with_watchdog` is built on perl, matching test-macapp.sh/test-appimage.sh.
# perl ships with macOS, where this suite runs, and `timeout(1)` does not — but a
# stripped Linux box can have neither (Alpine's BusyBox has `timeout` and no perl).
# Fail here rather than let 462 fixtures each silently lose their watchdog.
if ! command -v perl >/dev/null 2>&1; then
  echo "test-accept.sh: perl is required for the per-fixture watchdog (bug-320)" >&2
  exit 2
fi

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

# Remove a build's `<test_dir>/build/` output directory (plan-46-D §4.1).
#
# The directory name is the fixed literal `build`, never interpolated from
# project.json — so unlike a `$package_name`-derived path, a bad manifest parse
# can never redirect this `rm -rf` at a fixture's source. `$test_dir` is still
# checked for emptiness, since `rm -rf "/build"` would be its own kind of bad day.
remove_output_dir() {
  local test_dir=$1

  [ -n "$test_dir" ] || return 0
  [ -d "$test_dir/build" ] || return 0

  rm -rf "$test_dir/build"
}

# Run a built fixture program with a watchdog and a deterministic stdin (bug-320).
#
# Without this, a program that never exits wedges the entire suite: no output, no
# failing fixture, no exit code, and the per-fixture log stays buffered so tailing
# it shows nothing either. `test-macapp.sh:run_headless` and `test-appimage.sh`
# already establish this perl/alarm pattern; this is the same shape, except stdout
# and stderr pass straight through rather than being summarized, because the
# program's output lands in `build.log` and is diffed against that golden. (The
# `<pkg>.run` file is only the marker that says "execute this fixture"; its
# contents are never compared.)
#
# stdin is redirected from /dev/null by the child itself rather than inherited:
# plan-15's broadcast reader subscribes to fd 0, and on a live pipe (what you get
# from `nohup ... &` without a redirect) that thread blocks forever, so the program
# completes its work and then hangs at teardown. Owning the redirect here keeps a
# fixture's result independent of how the harness was launched.
#
# Exit status mirrors what the shell would have reported running the program
# directly — 128+N on a signal, otherwise the program's own code — so existing
# `[exit N]` goldens are unaffected. A timeout prints `timeout` into the fixture's
# log, which diffs loudly against its `build.log` golden, and yields 99.
#
# The bound is deliberately far above any fixture's real runtime: it exists to
# turn an *infinite* hang into one named failure, not to police performance. It
# has to be, because some fixtures are legitimately slow for reasons that have
# nothing to do with the code under test — the `tests/rt-behavior/native/*` LINK
# fixtures `dlopen` the system `libsqlite3.dylib`, and macOS stalls 40-60s on that
# (0s CPU, wall-clock only, duration varying with the network). A 60s bound made
# those fixtures flaky and `native-link-alias-collision-rt` fail outright at 61s.
# Anything that trips 300s is genuinely wedged.
run_with_watchdog() {
  perl -e '
    my $limit = shift @ARGV;
    my $pid = fork();
    die "fork failed: $!\n" unless defined $pid;
    if ($pid == 0) {
      open(STDIN, "<", "/dev/null") or exit 127;
      exec(@ARGV) or exit 127;
    }
    local $SIG{ALRM} = sub {
      kill "KILL", $pid;
      waitpid($pid, 0);
      $| = 1;
      print "timeout\n";
      exit 99;
    };
    alarm $limit;
    waitpid($pid, 0);
    alarm 0;
    my $st = $?;
    exit(($st & 127) ? 128 + ($st & 127) : ($st >> 8));
  ' "${MFB_ACCEPT_RUN_TIMEOUT:-300}" "$@"
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

# Every directory holding a project.json is a test, at any depth. Tests are
# organized under four top-level trees: tests/acceptance (the single TESTING
# app), tests/syntax/<feature>/* (compile-time diagnostics), tests/rt-error/
# <feature>/* (runtime errors), and tests/rt-behavior/<feature>/* (runtime
# behavior). A <feature> directory is just a grouping dir (no project.json of
# its own) and is skipped. Process substitution keeps the loop in this shell so
# `ran`/`failures` persist.
while IFS= read -r project_json; do
  test_dir=$(dirname "$project_json")

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

  # A test with no golden/ directory is a behavioral (acceptance) test: run
  # `mfb test` and require exit 0 (all TESTING cases passed). Nothing is compared.
  if [ ! -d "$golden_dir" ]; then
    test_out=$("$MFB_EXE" test "tests/$test_name" 2>&1)
    test_status=$?
    {
      echo "\$ mfb test tests/$test_name"
      printf '%s\n' "$test_out"
      echo "[exit $test_status]"
    } >"$actual_dir/test.log"
    remove_output_dir "$test_dir"
    if [ "$test_status" -ne 0 ]; then
      echo "behavioral test failed (exit $test_status): $test_name" >&2
      printf '%s\n' "$test_out" >&2
      failures=$((failures + 1))
    fi
    continue
  fi

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

  rm -f "$ast_path" "$ir_path" "$hex_path" "$mfp_path" "$nir_path" "$nplan_path" "$nobj_path" "$ncode_path" "$mir_path" "$target_nir_path" "$target_nplan_path" "$target_nobj_path" "$target_ncode_path" "$target_mir_path" "$target_app_nir_path" "$target_app_nplan_path" "$target_app_ncode_path"
  remove_output_dir "$test_dir"

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
    # plan-36: capture build.log with `-q` so the deterministic `Building …`
    # summary line (and any `-v` timings) never enter the exact-compared golden.
    # `-q` restores today's minimal output; the `Wrote … to` artifact line still
    # prints on stdout, so the run-path extraction below is unaffected.
    echo "$ mfb build ${target_label}${console_flags} tests/$test_name"
    # shellcheck disable=SC2086
    "$MFB_EXE" build -q $target_arg $console_flags "tests/$test_name"
    echo "[exit $?]"
    if [ -f "$golden_dir/$package_name.mfp" ] || [ -f "$golden_dir/$package_name.info" ]; then
      echo "$ mfb build tests/$test_name"
      "$MFB_EXE" build -q "tests/$test_name"
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
      "$MFB_EXE" build -q $target_arg -app $app_flags "tests/$test_name"
      echo "[exit $?]"
    fi
    if [ -f "$golden_dir/$package_name.run" ]; then
      echo "$ mfb build ${target_label}tests/$test_name"
      build_output=$("$MFB_EXE" build -q $target_arg "tests/$test_name" 2>&1)
      build_status=$?
      printf '%s\n' "$build_output"
      echo "[exit $build_status]"
      if [ "$build_status" -eq 0 ]; then
        run_path=$(printf '%s\n' "$build_output" | sed -n 's/^Wrote executable to //p' | tail -n 1)
        if [ -n "$run_path" ]; then
          echo "$ $run_path"
          run_with_watchdog "$run_path"
          echo "[exit $?]"
        else
          echo "error: build did not report an executable path"
          echo "[exit 1]"
        fi
      fi
    fi
  } >"$log_path" 2>&1
  remove_output_dir "$test_dir"

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

  # `mfb test` runtime proof (plan-18): run the test driver and capture its
  # streamed tree, summary, and exit code. Only when the fixture ships a golden.
  testrun_path="$actual_dir/$package_name.testrun"
  if [ -f "$golden_dir/$package_name.testrun" ]; then
    {
      echo "\$ mfb test tests/$test_name"
      "$MFB_EXE" test "tests/$test_name" 2>&1
      echo "[exit $?]"
    } >"$testrun_path"
    # `mfb test` links an executable into the project dir; do not leave it behind.
    remove_output_dir "$test_dir"
  fi

  # `mfb test --coverage` proof (plan-18-C): run with coverage and capture the
  # machine-independent sidecars (relative-path slot map + per-slot counts +
  # failed source lines). Only when the fixture ships a covmap golden.
  if [ -f "$golden_dir/$package_name.covmap.json" ]; then
    "$MFB_EXE" test --coverage "tests/$test_name" >/dev/null 2>&1
    for ext in covmap.json covdata covfail; do
      if [ -f "$test_dir/coverage.$ext" ]; then
        cp "$test_dir/coverage.$ext" "$actual_dir/$package_name.$ext"
      fi
    done
    # Do not leave the coverage sidecars, report, or executable behind.
    rm -f "$test_dir/coverage.covmap.json" "$test_dir/coverage.covdata" \
      "$test_dir/coverage.covfail" "$test_dir/coverage.html"
    remove_output_dir "$test_dir"
  fi

  compare_file "$test_name/build.log" "$golden_dir/build.log" "$log_path"
  compare_optional_output "$test_name/$package_name.testrun" \
    "$golden_dir/$package_name.testrun" \
    "$testrun_path"
  compare_optional_output "$test_name/$package_name.covmap.json" \
    "$golden_dir/$package_name.covmap.json" \
    "$actual_dir/$package_name.covmap.json"
  compare_optional_output "$test_name/$package_name.covdata" \
    "$golden_dir/$package_name.covdata" \
    "$actual_dir/$package_name.covdata"
  compare_optional_output "$test_name/$package_name.covfail" \
    "$golden_dir/$package_name.covfail" \
    "$actual_dir/$package_name.covfail"
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
done < <(find "$TEST_ROOT" -name project.json | sort)

if [ "${#FILTERS[@]}" -ne 0 ] && [ "$ran" -eq 0 ]; then
  echo "no tests matched filter: ${FILTERS[*]}" >&2
  exit 2
fi

if [ "$failures" -ne 0 ]; then
  echo "acceptance tests failed: $failures mismatch(es) ($ran test(s) ran)" >&2
  exit 1
fi

echo "acceptance tests passed ($ran test(s) ran)"
