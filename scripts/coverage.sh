#!/usr/bin/env sh
# Local coverage via cargo-llvm-cov (LLVM source-based). Works on macOS aarch64
# and Linux with the same engine, so local and CI numbers agree per platform.
#
# Runs the instrumented workspace test suite once and leaves the merged profile
# data in place so scripts/coverage-check.sh can generate the per-file JSON gate
# report without re-running the suite (report reuses the cached profdata).
#
# The --ignore-filename-regex excludes, from the coverage denominator:
#   - target/ and tests/          : build artifacts + the integration harness
#   - repository/target/          : generated bindgen/serde in the sub-crate
#   - *_runtime_tables.rs         : generated Unicode data tables (accessors are
#                                   covered by unicode_backend.rs tests)
#   - code/private/unicode.rs     : generated Unicode lookup arrays
set -eu

cd "$(dirname "$0")/.."

IGNORE='(^|/)(target|tests)/|repository/target/|_runtime_tables\.rs$|/code/private/unicode\.rs$|/src/testutil\.rs$'

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
  --ignore-filename-regex "$IGNORE" \
  --no-report || status=$?

# Human-readable + tooling reports from the held profile. If the run produced no
# profile at all (e.g. a compile failure), these error out and `set -e` fails the
# script here — which is still a loud, non-zero exit, so nothing is masked.
cargo llvm-cov report \
  --ignore-filename-regex "$IGNORE" \
  --html --output-dir target/coverage
cargo llvm-cov report \
  --ignore-filename-regex "$IGNORE" \
  --lcov --output-path target/coverage/lcov.info
cargo llvm-cov report \
  --ignore-filename-regex "$IGNORE" \
  --cobertura --output-path target/coverage/cobertura.xml

echo "HTML:      target/coverage/html/index.html"
echo "lcov:      target/coverage/lcov.info"
echo "cobertura: target/coverage/cobertura.xml"

# Surface the suite's exit status: a failing/aborting test now fails the job.
exit "$status"
