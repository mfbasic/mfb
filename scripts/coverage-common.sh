#!/usr/bin/env sh
# Shared coverage settings. Sourced by scripts/coverage.sh,
# scripts/coverage-check.sh, and the global floor step in
# .github/workflows/coverage.yml. Source it with the repo root as $PWD.
#
# Defines two things: IGNORE (denominator exclusions) and PKG_FLAGS (which
# packages a `cargo llvm-cov report` pass covers).
#
# --- IGNORE ---------------------------------------------------------------
# Excluded from the coverage denominator:
#   - target/ and tests/          : build artifacts + the integration harness
#                                   (also matches repository/target/ and
#                                   repository/tests/)
#   - *_runtime_tables.rs         : generated Unicode data tables (accessors are
#                                   covered by unicode_backend.rs tests)
#   - code/private/unicode.rs     : generated Unicode lookup arrays
#   - src/testutil.rs             : test-only helpers
#
# NOT excluded: repository/src/**. Before bug-347 that code sat outside the
# denominator only because `repository/` was a separate Cargo workspace and
# `--workspace` never selected it — an accident, not a policy. It is now a
# workspace member, measured and gated like every other crate in the tree.
#
# This regex used to be hand-duplicated in three files; bug-347 collapsed them,
# because an edit to one copy would silently diverge the local gate from CI's.
IGNORE='(^|/)(target|tests)/|_runtime_tables\.rs$|/code/private/unicode\.rs$|/src/testutil\.rs$'

# --- PKG_FLAGS ------------------------------------------------------------
# `cargo llvm-cov report` accepts no --workspace flag (it is rejected as
# "specific to [test, ...]"), and with no package selection it silently reports
# only the root package's objects. That is exactly how bug-347 stayed invisible:
# the run step instrumented `mfb_repository` and its profile data was collected
# correctly, but every report pass dropped its object file, so repository/src/**
# never appeared in the numerator OR the denominator. Enumerating the members
# explicitly is what makes the report match the run.
#
# Derived from cargo metadata rather than hardcoded, so a future workspace
# member is measured automatically instead of silently reintroducing this bug.
PKG_FLAGS="$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; print(" ".join("-p " + p["name"] for p in sorted(json.load(sys.stdin)["packages"], key=lambda p: p["name"])))')"
