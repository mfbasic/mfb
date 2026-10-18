#!/usr/bin/env bash
# The landing tier: the checks every change runs before it lands, in minutes.
#
# The full suite (`scripts/test-full.sh`) is hours of cross-product sweeps, and
# running it to land anything is what used to block the shared checkout. This
# tier runs everything EXCEPT those sweeps, plus whatever targeted gates the change
# names on the command line. `.ai/testing-gates.md` "Test tiers" is the policy.
#
#   1. fmt       `cargo fmt --all -- --check`
#   2. check     `cargo check --workspace --all-targets`: every integration-test
#                binary still compiles, without linking ~350 of them
#   3. build     compile the test binaries steps 4 and 5 run
#   4. skips     every `scripts/test-fast-skips.txt` entry still names a test
#   5. tests     the workspace's unit tests (bins + libs) minus the skip list, the
#                guard binaries, and each `--test` binary
#   6. artifact  `scripts/artifact-gate.sh` per `--artifact <builtin>`
#   7. accept    `scripts/test-accept.sh` over the `--accept` globs
#
# Every step runs even after one fails, so one report shows every red step. Exit 0
# only when all of them pass. Logs go to a fresh directory under $TMPDIR, printed
# at the start and the end.
set -u

usage() {
  cat >&2 <<'EOF'
Usage: bash scripts/test-fast.sh [--test <binary>]... [--artifact <builtin>]...
                                 [--accept <name-glob>]...

  --test <binary>       also run this integration test binary (a [[test]] name)
  --artifact <builtin>  also run the scoped artifact gate for tests/byte-identity/<builtin>
  --accept <name-glob>  also run test-accept.sh over fixtures matching the glob
EOF
}

REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO" || exit 2

# The guard and gate binaries cost seconds and protect tree-wide rules, so they run
# on every landing. Left out on purpose, each to be named with `--test` when its
# area changes: `golden` (the full artifact gate, the full tier's job),
# `inplace_self_update_census` (97 s; the self-update arms) and `script_selftests`
# (62 s; `test-accept.sh` and `remote-common.sh` machinery).
GUARDS=(
  architecture_guards
  example_project_names
  example_readmes
  no_operator_strings
  no_type_strings
  test_targets_registered
  gate_lock_covers_every_writer
  gate_mutual_exclusion
  gate_release_on_normal_exit
  scripts_index_is_complete
)

EXTRA_TESTS=()
ARTIFACTS=()
ACCEPT_GLOBS=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --test|--artifact|--accept)
      if [ "$#" -lt 2 ] || [ -z "$2" ]; then
        echo "error: $1 needs a value" >&2
        usage
        exit 2
      fi
      case "$1" in
        --test) EXTRA_TESTS+=("$2") ;;
        --artifact) ARTIFACTS+=("$2") ;;
        --accept) ACCEPT_GLOBS+=("$2") ;;
      esac
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) echo "error: unknown argument '$1'" >&2; usage; exit 2 ;;
  esac
done

TMP_ROOT="${TMPDIR:-/tmp}"
LOG_DIR="$(mktemp -d "${TMP_ROOT%/}/mfb-test-fast.XXXXXX")"
echo "test-fast: logs in $LOG_DIR"

RESULTS=()
FAILED=0

# run_step <name> <command...>: run one step into $LOG_DIR/<name>.log, record it.
run_step() {
  local name="$1"
  shift
  local start end rc verdict
  start=$(date +%s)
  echo "test-fast: $name ..."
  "$@" > "$LOG_DIR/$name.log" 2>&1
  rc=$?
  end=$(date +%s)
  case "$rc" in
    0) verdict="ok" ;;
    # artifact-gate.sh / test-accept.sh refuse with 98 when another gate holds
    # this tree's lock. That is "nothing was checked", not a pass.
    98) verdict="REFUSED (gate lock held; nothing checked)"; FAILED=1 ;;
    *) verdict="FAILED (exit $rc)"; FAILED=1 ;;
  esac
  RESULTS+=("$(printf '%-24s %5ss  %s' "$name" "$((end - start))" "$verdict")")
}

# The skip list, as substrings. A line's substring ends at its first double space.
SKIPS=()
while IFS= read -r line; do
  case "$line" in ''|'#'*) continue ;; esac
  SKIPS+=("${line%%  *}")
done < "$REPO/scripts/test-fast-skips.txt"

SKIP_ARGS=()
for skip in "${SKIPS[@]}"; do
  SKIP_ARGS+=(--skip "$skip")
done

TEST_ARGS=(--workspace --no-fail-fast --bins --lib)
for name in "${GUARDS[@]}" "${EXTRA_TESTS[@]+"${EXTRA_TESTS[@]}"}"; do
  TEST_ARGS+=(--test "$name")
done

# Every skip entry must match at least one test of the binaries step 5 runs.
check_skips() {
  local listing="$LOG_DIR/test-list.txt" skip count bad=0
  if ! cargo test "${TEST_ARGS[@]}" -- --list --format terse > "$listing"; then
    echo "error: could not list the landing tier's tests" >&2
    return 1
  fi
  for skip in "${SKIPS[@]}"; do
    count=$(grep -F -c -- "$skip" "$listing")
    if [ "$count" -eq 0 ]; then
      echo "stale entry in scripts/test-fast-skips.txt (matches no test): $skip" >&2
      bad=1
    else
      echo "$count  $skip"
    fi
  done
  return "$bad"
}

run_step fmt cargo fmt --all -- --check
run_step check cargo check --workspace --all-targets
run_step build cargo test "${TEST_ARGS[@]}" --no-run
run_step skips check_skips
run_step tests cargo test "${TEST_ARGS[@]}" -- "${SKIP_ARGS[@]}"

if [ "${#ARTIFACTS[@]}" -gt 0 ] || [ "${#ACCEPT_GLOBS[@]}" -gt 0 ]; then
  run_step release-mfb cargo build --release --bin mfb
  for builtin in "${ARTIFACTS[@]+"${ARTIFACTS[@]}"}"; do
    run_step "artifact-$builtin" bash "$REPO/scripts/artifact-gate.sh" target/release/mfb "$builtin"
  done
  if [ "${#ACCEPT_GLOBS[@]}" -gt 0 ]; then
    run_step accept bash "$REPO/scripts/test-accept.sh" target/release/mfb "$LOG_DIR/accept" \
      "${ACCEPT_GLOBS[@]}"
  fi
fi

echo
echo "test-fast summary (logs in $LOG_DIR):"
printf '  %s\n' "${RESULTS[@]}"
if [ "$FAILED" -ne 0 ]; then
  echo "test-fast: FAILED"
  exit 1
fi
echo "test-fast: ok"
