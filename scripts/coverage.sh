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

IGNORE='(^|/)(target|tests)/|repository/target/|_runtime_tables\.rs$|/code/private/unicode\.rs$'

# Instrument + run the suite, holding the profile for later report passes.
# --no-fail-fast: keep running (and collecting coverage from) every test binary
# even if one fails, so an environment-flaky integration test (e.g. a pty/TTY
# winsize probe that the sandbox doesn't honor) can't zero out the whole report.
# A test-failure exit code must not abort the script (profile data is still
# written); a genuine compile failure surfaces below when the report steps find
# no profile. `|| true` keeps `set -e` from stopping here.
cargo llvm-cov --workspace --all-targets --no-fail-fast \
  --ignore-filename-regex "$IGNORE" \
  --no-report || true

# Human-readable + tooling reports from the held profile.
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
