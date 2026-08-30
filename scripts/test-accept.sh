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

# Refuse to run concurrently with another test-accept — concurrent runs thrash
# disk/CPU and clobber each other's actual output, yielding phantom "missing
# actual" failures on unrelated fixtures.
#
# `pgrep -f 'test-accept\.sh'` matches every process whose command line contains
# this script's path, which includes our OWN transient children: bash keeps the
# parent `bash scripts/test-accept.sh …` command line on the subshells and
# pipeline members it fork()s to evaluate a `$(...)`, in the window before they
# exec(). Excluding only `$$` (the main shell) missed those, so the guard would
# report a phantom "pid N is running" with no real concurrent run — a false CI
# abort (deterministic under bash 5.2). Instead, skip every candidate that shares
# our process group: an invocation's children/subshells inherit its PGID at
# fork() (so they are excluded even mid-race), while a genuinely separate run is
# launched into its own session/group. A candidate that has already exited (empty
# PGID) is not a live run either. The `.sh` anchor still keeps this from matching
# test-accept-selftest.sh (a distinct, lightweight harness).
mypgid=$(ps -o pgid= -p "$$" | tr -d ' ')
other=""
for pid in $(pgrep -f 'test-accept\.sh'); do
  [ "$pid" = "$$" ] && continue
  cpgid=$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ')
  [ -z "$cpgid" ] && continue
  [ "$cpgid" = "$mypgid" ] && continue
  # bug-455: a candidate only counts if it is EXECUTING the script, not merely
  # mentioning it. Another session's wrapper shell
  # (`zsh -c "... scripts/test-accept.sh ..."`) carries the path inside its `-c`
  # string and matches `pgrep -f` while holding no lock at all -- observed
  # blocking a run whose rival was still in its `cargo build` stage, and causing
  # mutual-wait deadlocks between sessions politely queueing on each other's
  # text. A real invocation has the script as argv[0] (`./scripts/test-accept.sh`) or
  # argv[1] (`bash scripts/test-accept.sh`); a wrapper has `-c` there instead.
  cargs=$(ps -o args= -p "$pid" 2>/dev/null)
  ca0=${cargs%% *}
  carest=${cargs#* }
  ca1=${carest%% *}
  case "$ca0" in
    */test-accept.sh|test-accept.sh) ;;
    *)
      case "$ca1" in
        */test-accept.sh|test-accept.sh) ;;
        *) continue ;;
      esac
      ;;
  esac
  other=$pid
  break
done
if [ -n "$other" ]; then
  echo "Another test-accept (pid $other) is running." >&2
# Exit 98, not 1: a refusal is NOT a gate result. Sharing 1 with "found diffs"
# means a lock collision reads as a golden regression, and the reader spends
# their time on the wrong question (observed: `cargo test` and a manual
# `test-accept.sh` refusing each other, and `tests/golden.rs` reporting it
# as a failed gate in 0.16s).
  exit 98
fi

# Shared codegen-dump artifact table (also sourced by scripts/artifact-gate.sh),
# so the two drivers cannot drift about which build dumps exist. It defines the
# host and native dump kinds this harness's native-artifact regions iterate; the
# packaging/execution artifacts (mfp/info/audit/testrun/coverage) are this
# harness's alone and stay in their own bespoke blocks below.
# shellcheck source=artifact-kinds.sh
. "$ROOT/scripts/artifact-kinds.sh"

# Hermetic package key store. `mfb build` verifies an imported package's
# attestation against the registry key pinned under
# `$MFB_HOME/<sha256(repo-url)>/server.pub` (`local_paths_for_repo`), defaulting
# `MFB_HOME` to `$HOME/.mfb`. Without this, a fixture's output depends on whether
# the *developer* has ever run `mfb repo auth` against whatever registry URL is
# the current default — so the suite passes or fails based on the machine it runs
# on, not the code.
#
# That is not hypothetical: `f2f583807` changed `DEFAULT_REPO_URL` from
# `http://127.0.0.1:7777` to `https://mfb-repo.fly.dev`, which changed which
# `$HOME/.mfb/<hash>/` directory is consulted. On a machine that had authed
# against the public registry, `pkg-01-tampered-signature` then reached the
# signature check and reported "invalid attestation signature" instead of "no
# pinned registry key" — a red suite whose obvious "fix" (sync the golden) would
# have baked one developer's key store into the tree and broken every clean
# checkout in the opposite direction.
#
# An empty per-run directory means "no key pinned for any registry", which is the
# only state a checkout can reproduce anywhere. Exported before the first fixture
# so every `mfb` invocation below inherits it.
MFB_HOME=$(mktemp -d)
export MFB_HOME
trap 'rm -rf "$MFB_HOME"' EXIT

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

# Host libc, used only to pick which executable a `.run` fixture executes.
#
# On Linux `mfb build` emits ONE executable per libc world -- `<pkg>-glibc.out`
# AND `<pkg>-musl.out` -- while macOS and Windows emit a single `<pkg>.out`. A
# musl binary is dynamically linked against /lib/ld-musl-<arch>.so.1, which a
# glibc host does not have, so running the wrong flavor dies in the loader with
# exit 127 before `main`. Selecting by host libc is therefore a correctness
# requirement, not a preference: the previous `tail -n 1` picked whichever line
# the compiler happened to print last (musl), so every runnable fixture failed
# on a glibc Linux host.
#
# `ldd --version` prints "musl libc" on musl and "ldd (GNU libc) …" on glibc.
# musl's ldd exits non-zero for `--version`, hence the 2>&1 and no status check.
host_libc=""
case "$target_name" in
  linux-*)
    if ldd --version 2>&1 | head -n 1 | grep -qi musl; then
      host_libc="musl"
    else
      host_libc="glibc"
    fi
    ;;
esac

# Canonicalize the libc-flavor suffix out of a build's stdout so build.log reads
# the same on every host. Without this the golden corpus is host-specific: a
# Linux run logs two `Wrote executable to …-{glibc,musl}.out` lines where macOS
# logs one `…out`, so every fixture that links an executable mismatches.
#
# This normalizes ONLY the flavor suffix and the ADJACENT duplicate
# `Wrote executable to` line it produces -- nothing else is rewritten, so a real
# change in build output still drifts the golden. The dedupe is deliberately
# limited to immediately-repeated lines: a fixture can legitimately log the same
# executable path from two separate build steps, and a whole-file `seen[$0]`
# dedupe would collapse those and break the macOS goldens instead. The dual-emit
# itself is not what these fixtures test; it is covered by the cli_* build tests.
#
# Applied to the WHOLE build.log rather than per step, because `Wrote executable
# to` is emitted by several steps (the `.run` build, the mfp/info build, `-app`),
# and a per-step pass silently missed the ones that are not captured in a var.
normalize_exe_flavor() {
  sed -E 's/-(glibc|musl)\.out$/.out/' \
    | awk '/^Wrote executable to / && $0 == prev { next } { print; prev = $0 }'
}

# Pick the executable to run from a build's stdout. One line -> use it. Several
# (Linux dual-emit) -> the flavor this host can actually execute.
select_run_path() {
  _paths=$(printf '%s\n' "$1" | sed -n 's/^Wrote executable to //p')
  if [ -n "$host_libc" ]; then
    _match=$(printf '%s\n' "$_paths" | grep -e "-$host_libc\.out\$" | tail -n 1)
    if [ -n "$_match" ]; then
      printf '%s\n' "$_match"
      return
    fi
  fi
  printf '%s\n' "$_paths" | tail -n 1
}

# True if a CONSOLE golden of kind $2 exists for ANY target, not just this host's.
#
# The requested native-dump flags must not depend on which host the harness runs
# on, because the `$ mfb build …` command line is echoed into build.log and
# compared exactly. Keying off `$package_name.$target_name.$ext` meant a fixture
# whose only native goldens are macos-aarch64 silently dropped `-nir -nplan -nobj`
# on a Linux host, drifting its build.log. Comparison stays per-host-target (a
# dump with no golden for this target is simply not compared); only the REQUEST
# is made uniform. Every fixture carrying native goldens has a macos-aarch64 set,
# so this is a no-op on macOS -- no golden regeneration needed.
any_target_console_golden() {
  for _g in "$golden_dir/$1."*".$2"; do
    [ -f "$_g" ] || continue
    case "${_g##*/}" in
      "$1".*.app."$2") continue ;;
    esac
    return 0
  done
  return 1
}

# The `-app` counterpart of `any_target_console_golden`: true if an app-mode
# golden of kind $2 exists for ANY target.
any_target_app_golden() {
  for _g in "$golden_dir/$1."*".app.$2"; do
    [ -f "$_g" ] && return 0
  done
  return 1
}

# plan-100: global opt-level switch, mirroring MFB_TARGET above. Unset (the
# default) appends nothing, so every `mfb build` below is byte-for-byte the
# command this harness has always run and the binary applies its own default of
# -O1 -- today's exact codegen.
#
# Deliberately NOT folded into the echoed `$ mfb build …` label the way
# `target_label` is: build.log is itself an exact-compared golden, so echoing
# `-O1` would drift every fixture's build.log under MFB_OPT=1, breaking the very
# gate (explicit -O1 == default) that MFB_OPT=1 exists to prove. The level is a
# property of the run, not of the fixture.
if [ -n "${MFB_OPT:-}" ]; then
  opt_arg="-O$MFB_OPT"
else
  opt_arg=""
fi

failures=0
ran=0
skipped=0

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

# Run a subprocess (a `mfb build` front-end pass, or a built fixture program) under
# a watchdog with a deterministic stdin (bug-320). Wrapping the builds too means an
# infinite loop in the compiler becomes one named `timeout` failure instead of
# wedging the whole suite with no output and no exit code.
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
#
# 300s is NOT "anything above this is wedged" once the compiler is unoptimized.
# Measured on an M-series Mac, `tests/acceptance` (692 tests, all passing) takes
# 63.7s driven by `target/release/mfb` but 338.6s driven by `target/debug/mfb` —
# so the default trips a perfectly healthy fixture on any debug-compiler run, and
# CI's acceptance job is exactly that (it consumes the `build` job's debug
# artifact). That is why the bound is an env knob: CI raises
# `MFB_ACCEPT_RUN_TIMEOUT` rather than this default moving, since a release-driven
# local run should still catch a hang quickly.
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
# organized under five top-level trees: tests/acceptance (the single TESTING
# app), tests/syntax/<feature>/* (compile-time diagnostics), tests/rt-error/
# <feature>/* (runtime errors), tests/rt-behavior/<feature>/* (runtime
# behavior), and tests/byte-identity/* (compile-only .ncodesum gate coverage —
# never executed). A <feature> directory is just a grouping dir (no project.json
# of its own) and is skipped. Process substitution keeps the loop in this shell
# so `ran`/`failures` persist.
while IFS= read -r project_json; do
  test_dir=$(dirname "$project_json")

  test_name=${test_dir#"$TEST_ROOT/"}
  matches_filter "$test_name" || continue

  # Per-fixture environment gate. A fixture may ship an executable `test-gate.sh`
  # in its own directory that decides, at runtime, whether this machine can run
  # it. The gate is run from the fixture directory; exit 0 means "run me", any
  # non-zero exit means "skip me" and whatever the gate printed is the reason.
  #
  # This exists for fixtures whose success depends on the host environment rather
  # than the compiler — e.g. `rt-behavior/tls/tls-connect-google-rt`, a live TLS
  # handshake against a public host, which cannot pass on a machine with no
  # network. Deleting such a fixture loses the coverage everywhere; leaving it
  # ungated turns the whole suite red on any offline/firewalled box. The gate
  # keeps the fixture running where it can and skipping (loudly, with a reason)
  # where it cannot, so a skip is never silently mistaken for a pass.
  if [ -f "$test_dir/test-gate.sh" ]; then
    gate_reason=$(cd "$test_dir" && bash test-gate.sh 2>&1)
    if [ "$?" -ne 0 ]; then
      skipped=$((skipped + 1))
      printf '[skip] %s: %s\n' "$test_name" "${gate_reason:-no reason given}" >&2
      continue
    fi
  fi

  ran=$((ran + 1))
  # Stream per-fixture progress to stderr so a long run is observable live
  # (stdout stays reserved for the final pass/fail summary; goldens diff only
  # captured command output, never this line).
  printf '[%d] %s\n' "$ran" "$test_name" >&2
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
    # Through `run_with_watchdog` like every other subprocess this harness
    # spawns, for its /dev/null stdin (bug-320) as much as its hang bound. Run
    # bare, this `mfb test` inherited the driving loop's stdin -- which is the
    # `find` pipe feeding `while read project_json` at the bottom of this file --
    # so a fixture whose TESTING blocks read stdin ATE THE FIXTURE LIST. That
    # produced two failures on every run, in both directions:
    #   * the io EOF cases ("expected a trap with code 77020003, but none
    #     occurred") saw pipe bytes instead of the EOF they assert; and
    #   * the next fixture's path arrived truncated at a random prefix
    #     ("could not read project name for fb/.claude/worktrees/..."), with a
    #     nondeterministic number of fixtures silently swallowed (1193 of 1208
    #     ran in one measured pair).
    # Found while proving plan-100's gate; the bare call predates that plan.
    test_out=$(run_with_watchdog "$MFB_EXE" test "tests/$test_name" 2>&1)
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

  # Clean any stale dump artifacts a previous run left in the fixture dir. Host
  # dumps and the package binary are cleaned by name; each native dump
  # (ARTIFACT_NATIVE_KINDS) is cleaned in every spelling the harness produces —
  # the non-infixed path `mfb build` writes, the target-infixed golden name, and,
  # for the app kinds, the `-app` variant. App-mode `-nir/-nplan/-ncode` write to
  # the same non-infixed `$package_name.{nir,nplan,ncode}` path as console mode,
  # so a fixture carries either console or app goldens for a kind, never both.
  rm -f "$ast_path" "$ir_path" "$hex_path" "$mfp_path"
  for ext in $ARTIFACT_NATIVE_KINDS; do
    rm -f "$test_dir/$package_name.$ext" "$test_dir/$package_name.$target_name.$ext"
    case " $ARTIFACT_NATIVE_APP_KINDS " in
      *" $ext "*) rm -f "$test_dir/$package_name.$target_name.app.$ext" ;;
    esac
  done
  remove_output_dir "$test_dir"

  {
    # Batch the artifact dumps: `mfb build` output flags combine, so one
    # invocation per flag family shares a single front-end pass instead of
    # re-parsing/resolving/typechecking the project once per artifact.
    console_flags="-ast -ir"
    if [ -f "$golden_dir/$package_name.hex" ]; then
      console_flags="$console_flags -br"
    fi
    # Native dumps: request each kind (ARTIFACT_NATIVE_KINDS) that has a
    # target-infixed console golden for ANY target -- see
    # `any_target_console_golden`. Same table + flag mapping the fast gate uses.
    for ext in $ARTIFACT_NATIVE_KINDS; do
      if any_target_console_golden "$package_name" "$ext"; then
        console_flags="$console_flags $(artifact_build_flag "$ext")"
      fi
    done
    # plan-36: capture build.log with `-q` so the deterministic `Building …`
    # summary line (and any `-v` timings) never enter the exact-compared golden.
    # `-q` restores today's minimal output; the `Wrote … to` artifact line still
    # prints on stdout, so the run-path extraction below is unaffected.
    echo "$ mfb build ${target_label}${console_flags} tests/$test_name"
    # shellcheck disable=SC2086
    run_with_watchdog "$MFB_EXE" build -q $target_arg $opt_arg $console_flags "tests/$test_name"
    echo "[exit $?]"
    if [ -f "$golden_dir/$package_name.mfp" ] || [ -f "$golden_dir/$package_name.info" ]; then
      echo "$ mfb build tests/$test_name"
      # shellcheck disable=SC2086
      run_with_watchdog "$MFB_EXE" build -q $opt_arg "tests/$test_name"
      echo "[exit $?]"
    fi
    # App-mode dumps: same any-target rule as the console loop above, so the
    # echoed `-app` command line does not vary by host either.
    app_flags=""
    for ext in $ARTIFACT_NATIVE_APP_KINDS; do
      if any_target_app_golden "$package_name" "$ext"; then
        app_flags="$app_flags $(artifact_build_flag "$ext")"
      fi
    done
    if [ -n "$app_flags" ]; then
      echo "$ mfb build ${target_label}-app${app_flags} tests/$test_name"
      # shellcheck disable=SC2086
      run_with_watchdog "$MFB_EXE" build -q $target_arg $opt_arg -app $app_flags "tests/$test_name"
      echo "[exit $?]"
    fi
    # A `<pkg>.run` golden forces the full `mfb build` (link + merge) path and
    # only executes the produced binary when the build SUCCEEDS. That makes it two
    # different things depending on the fixture:
    #   - a rt-behavior fixture builds cleanly, so `.run` is an execution proof;
    #   - a `-invalid` fixture (notably several under tests/syntax/**, e.g. the
    #     security pkg-0N confusion cases) is EXPECTED to fail the build, so its
    #     `.run` never reaches the executable — it is a MERGE TRIGGER that drives
    #     the fixture past `-ast -ir` into the full merge where the finding trips,
    #     with the diagnostic captured in build.log. `.run` contents are never
    #     compared. This is the source of truth for the convention; the security
    #     README (tests/rt-behavior/security/README.md) points here.
    if [ -f "$golden_dir/$package_name.run" ]; then
      echo "$ mfb build ${target_label}tests/$test_name"
      build_output=$(run_with_watchdog "$MFB_EXE" build -q $target_arg $opt_arg "tests/$test_name" 2>&1)
      build_status=$?
      printf '%s\n' "$build_output"
      echo "[exit $build_status]"
      if [ "$build_status" -eq 0 ]; then
        # Execute the flavor this host can actually load; the path is logged
        # verbatim and canonicalized by the whole-log pass below.
        run_path=$(select_run_path "$build_output")
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
  # Canonicalize the host-varying libc flavor out of the finished log (see
  # `normalize_exe_flavor`) so this golden is comparable on every host.
  normalize_exe_flavor <"$log_path" >"$log_path.norm" && mv "$log_path.norm" "$log_path"
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
  # Move each native dump the build wrote (non-infixed path) to its target-infixed
  # actual name. App kinds (ARTIFACT_NATIVE_APP_KINDS) go to the `-app` variant
  # name when the fixture carries an app golden for that kind, else the console
  # name — matching how the flags above were chosen.
  for ext in $ARTIFACT_NATIVE_KINDS; do
    src="$test_dir/$package_name.$ext"
    [ -f "$src" ] || continue
    dest="$actual_dir/$package_name.$target_name.$ext"
    case " $ARTIFACT_NATIVE_APP_KINDS " in
      *" $ext "*)
        if [ -f "$golden_dir/$package_name.$target_name.app.$ext" ]; then
          dest="$actual_dir/$package_name.$target_name.app.$ext"
        fi ;;
    esac
    # Keep the actual only when this host's target has a golden to compare it
    # against. The flags are now requested host-independently
    # (`any_target_console_golden`) so the echoed command line matches on every
    # host, which means a Linux run legitimately produces dumps that only have
    # macos-aarch64 goldens. Moving those in would trip `compare_optional_output`'s
    # "unexpected actual" guard — a guard worth keeping, so discard here instead of
    # weakening it.
    if [ -f "$golden_dir/$(basename "$dest")" ]; then
      mv "$src" "$dest"
    else
      rm -f "$src"
    fi
  done

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
      # `</dev/null` for the same reason as the behavioral-test call above: this
      # loop's stdin is the `find` pipe, and a fixture whose TESTING blocks read
      # stdin would eat the fixture list (and see pipe bytes instead of EOF).
      "$MFB_EXE" test "tests/$test_name" 2>&1 </dev/null
      echo "[exit $?]"
    } >"$testrun_path"
    # `mfb test` links an executable into the project dir; do not leave it behind.
    remove_output_dir "$test_dir"
  fi

  # `mfb test --coverage` proof (plan-18-C): run with coverage and capture the
  # machine-independent sidecars (relative-path slot map + per-slot counts +
  # failed source lines). Only when the fixture ships a covmap golden.
  if [ -f "$golden_dir/$package_name.covmap.json" ]; then
    # `</dev/null`: see the behavioral-test call above -- this loop's stdin is
    # the `find` pipe driving it.
    "$MFB_EXE" test --coverage "tests/$test_name" >/dev/null 2>&1 </dev/null
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
  # Native dumps (console) and their app-mode variants, driven by the shared
  # table so the compared set matches what the flags requested and the mv placed.
  for ext in $ARTIFACT_NATIVE_KINDS; do
    compare_optional_output "$test_name/$package_name.$target_name.$ext" \
      "$golden_dir/$package_name.$target_name.$ext" \
      "$actual_dir/$package_name.$target_name.$ext"
  done
  for ext in $ARTIFACT_NATIVE_APP_KINDS; do
    compare_optional_output "$test_name/$package_name.$target_name.app.$ext" \
      "$golden_dir/$package_name.$target_name.app.$ext" \
      "$actual_dir/$package_name.$target_name.app.$ext"
  done
done < <(find "$TEST_ROOT" -name project.json | sort)

# A filter that matched only gated-out fixtures ($ran == 0 but $skipped > 0) is
# not "no match" — the tests exist, this machine just cannot run them. Only a
# genuine zero-match (nothing ran, nothing skipped) is the usage error.
if [ "${#FILTERS[@]}" -ne 0 ] && [ "$ran" -eq 0 ] && [ "$skipped" -eq 0 ]; then
  echo "no tests matched filter: ${FILTERS[*]}" >&2
  exit 2
fi

skip_note=""
[ "$skipped" -ne 0 ] && skip_note=", $skipped skipped"

if [ "$failures" -ne 0 ]; then
  echo "acceptance tests failed: $failures mismatch(es) ($ran test(s) ran$skip_note)" >&2
  exit 1
fi

echo "acceptance tests passed ($ran test(s) ran$skip_note)"
