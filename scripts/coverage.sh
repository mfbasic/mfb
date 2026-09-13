#!/usr/bin/env sh
# Local coverage via cargo-llvm-cov (LLVM source-based). Works on macOS aarch64
# and Linux with the same engine, so local and CI numbers agree per platform.
#
# Runs the instrumented workspace test suite once and leaves the merged profile
# data in place so scripts/coverage-check.sh can generate the per-file JSON gate
# report without re-running the suite (report reuses the cached profdata).
#
# IGNORE (denominator exclusions) and PKG_FLAGS (packages each report pass
# covers) live in scripts/coverage-common.sh — see that file for both. Per
# bug-347, `repository/` is now a workspace member, so repository/src/** is
# exercised by the run below AND measured by the report passes; $PKG_FLAGS is
# what makes the latter true.
set -eu

cd "$(dirname "$0")/.."

. ./scripts/coverage-common.sh

# --bins: fast per-file coverage for `src/**` only. Instrument and run the `mfb`
# unit tests, then regenerate the gate report (target/coverage/coverage.json)
# from that profile. Remaining arguments go to `cargo llvm-cov`.
#
# This is NOT an approximation of the full run for `src/**`; it is the same
# measurement. `mfb` is a binary-only package (`cargo test -p mfb --lib` answers
# "no library targets found"), and a Cargo integration test can only link a
# package's LIBRARY target, so nothing under `tests/` links a line of `src/**`.
# Those binaries reach the compiler by spawning `target/release/mfb`
# (`tests/common/mod.rs`'s `mfb_exe()`), a separate process whose profile this
# one never merges. `src/**` coverage is therefore a pure function of the
# `--bins` unit tests, and this mode takes minutes where the full run takes hours.
#
# What it does NOT measure: `repository/src/**`. `mfb_repository` IS a library,
# so its integration tests do contribute, and every one of them is skipped here.
# Its numbers in the report are meaningless; run without `--bins` before claiming
# the whole gate is green.
if [ "${1:-}" = "--bins" ]; then
  shift
  cargo llvm-cov --bins --no-fail-fast --no-report "$@"

  # Before reporting: the plain `mfb` binary is instrumented and never executed,
  # and llvm-cov would merge its whole mapping at count 0.
  drop_never_executed_binaries

  # `--output-path` does not create its directory, and in a fresh worktree nothing
  # else has (the full run's `--html --output-dir` pass is what used to): without
  # this the whole unit-test run finishes and then fails writing the report.
  mkdir -p target/coverage
  cargo llvm-cov report $PKG_FLAGS \
    --ignore-filename-regex "$IGNORE" \
    --json --output-path target/coverage/coverage.json >/dev/null

  echo "src/** profile refreshed; report with scripts/coverage-check.sh"
  exit 0
fi

# Instrument + run the suite, holding the profile for later report passes.
# --no-fail-fast: keep running (and collecting coverage from) every test binary
# even if one fails, so a single failing target still contributes its coverage
# to the merged profile.
#
# We DO NOT swallow the exit code. A failing test must fail this script (and the
# CI job) loudly — masking it with `|| true` previously hid real defects (a test
# that SIGABRTs mid-run silently drops its whole binary's profile and zeroes
# coverage for everything it covered). We still generate the reports from the
# profile that was collected, then exit with the suite's status so the failure
# is not buried. `set -e` must not abort before the reports, so capture the code.
status=0
cargo llvm-cov --workspace --all-targets --no-fail-fast \
  --no-report || status=$?

# The plain (non-test) binaries are instrumented and never executed; llvm-cov
# would merge their whole mapping at count 0. Drop them before the first report
# pass — they stay gone for coverage-check.sh and the global-floor pass that
# follow in the same job.
drop_never_executed_binaries

# Human-readable + tooling reports from the held profile. If the run produced no
# profile at all (e.g. a compile failure), these error out and `set -e` fails the
# script here — which is still a loud, non-zero exit, so nothing is masked.
cargo llvm-cov report $PKG_FLAGS \
  --ignore-filename-regex "$IGNORE" \
  --html --output-dir target/coverage
cargo llvm-cov report $PKG_FLAGS \
  --ignore-filename-regex "$IGNORE" \
  --lcov --output-path target/coverage/lcov.info
cargo llvm-cov report $PKG_FLAGS \
  --ignore-filename-regex "$IGNORE" \
  --cobertura --output-path target/coverage/cobertura.xml

echo "HTML:      target/coverage/html/index.html"
echo "lcov:      target/coverage/lcov.info"
echo "cobertura: target/coverage/cobertura.xml"

# Surface the suite's exit status: a failing/aborting test now fails the job.
exit "$status"
