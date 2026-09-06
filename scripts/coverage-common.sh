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

# --- drop_never_executed_binaries -----------------------------------------
# Remove the plain (non-test) binary artifacts from the coverage target
# directory. Call it after the instrumented run and BEFORE the first report
# pass; the artifacts stay gone for every later pass in the same job.
#
# `cargo llvm-cov --bins` / `--all-targets` builds each bin target twice: the
# test harness, which runs, and the plain binary, which is instrumented and then
# never executed by anything. Nothing executes it — the integration suite reaches
# the compiler by spawning `target/release/mfb`, a separate uninstrumented
# process whose profile this one never merges.
#
# `llvm-cov` reports the UNION of every object's regions and sums their line
# counts, so the never-run copy contributes its whole mapping at count 0. Where
# the two copies inline the same function differently, lines the running copy
# demonstrably executes come back uncovered — and the size of that swings with
# code changes that have nothing to do with any test. Measured on one profile,
# reporting with the plain binaries present against absent:
#
#     248 files below the floor / 22,883 uncovered lines   (present)
#     205 files below the floor /  6,863 uncovered lines   (absent)
#
# Nothing is lost by dropping them. `grep -rn "cfg(not(test))" src/ repository/src/`
# (discounting `cfg_attr`) is 0, so no line exists only in a plain build; every
# line this removes is one the test copy already carries.
#
# THE DISCRIMINATOR IS THE BINARY'S CONTENT, and it has to be. Neither the hash
# nor the size nor the presence of the `debug/<name>` uplift identifies the
# harness: `mfb_repo`'s plain copy is the LARGER of its two (17.9 MB against
# 5.0 MB) and had no uplift at all, so an earlier size- or uplift-based rule left
# it in the report. A libtest harness embeds libtest's own argument help, so
# `--test-threads` appears in it and in nothing else here.
#
# The count guard is the safety net: deleting a harness by mistake would zero the
# coverage of everything it covers and read as a catastrophic regression, so a
# name whose binaries are ALL classified plain is left completely alone.
drop_never_executed_binaries() {
  cov_deps="${1:-target/llvm-cov-target}/debug/deps"
  [ -d "$cov_deps" ] || return 0
  cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import json,sys
for package in json.load(sys.stdin)["packages"]:
    for target in package["targets"]:
        if "bin" in target["kind"]:
            print(target["name"])' \
    | while IFS= read -r bin_name; do
        # Cargo writes `-` in a target name as `_` in the artifact filename
        # (`mfb-repo` -> `mfb_repo`), so the metadata name is not the on-disk
        # one. Globbing the metadata spelling matched nothing and left
        # `mfb_repo`'s plain copy in every report.
        bin_name=$(printf '%s' "$bin_name" | tr '-' '_')
        harnesses=0
        plain=""
        for candidate in "$cov_deps/$bin_name"-*; do
          [ -f "$candidate" ] || continue
          [ -x "$candidate" ] || continue
          case "$candidate" in *.d | *.o | *.dSYM) continue ;; esac
          if LC_ALL=C grep -qa -- "--test-threads" "$candidate"; then
            harnesses=$((harnesses + 1))
          else
            plain="$plain $candidate"
          fi
        done
        [ "$harnesses" -gt 0 ] || continue
        for doomed in $plain; do
          rm -f "$doomed"
        done
        rm -f "${1:-target/llvm-cov-target}/debug/$bin_name"
      done
}
