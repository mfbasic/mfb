#!/usr/bin/env sh
# Fast per-file coverage for `src/**` only: instrument and run the `mfb` unit
# tests (`--bins`), then regenerate the gate report from that profile.
#
# This is NOT an approximation of scripts/coverage.sh for `src/**` — it is the
# same measurement. `mfb` is a binary-only package (`cargo test -p mfb --lib`
# answers "no library targets found"), and a Cargo integration test can only
# link a package's LIBRARY target, so nothing under `tests/` links a line of
# `src/**`. Those binaries reach the compiler by spawning
# `target/release/mfb` (`tests/common/mod.rs`'s `mfb_exe()`), a separate process
# whose profile this one never merges. `src/**` coverage is therefore a pure
# function of the `--bins` unit tests, and this script takes minutes where the
# full run takes hours.
#
# What it does NOT measure: `repository/src/**`. `mfb_repository` IS a library,
# so its integration tests do contribute, and every one of them is skipped here.
# Its numbers in the report below are meaningless — use scripts/coverage.sh
# before claiming the whole gate is green.
#
# Same IGNORE / PKG_FLAGS as every other coverage entry point
# (scripts/coverage-common.sh); bug-347 exists because three copies diverged.
set -eu

cd "$(dirname "$0")/.."

. ./scripts/coverage-common.sh

cargo llvm-cov --bins --no-fail-fast --no-report "$@"

cargo llvm-cov report $PKG_FLAGS \
  --ignore-filename-regex "$IGNORE" \
  --json --output-path target/coverage/coverage.json >/dev/null

echo "src/** profile refreshed; report with scripts/coverage-check.sh"
