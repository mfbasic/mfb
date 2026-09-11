# bug-577: the S3 integration-test target no longer compiles after publish metadata was added

Last updated: 2026-09-11
Effort: small (<1h)
Severity: HIGH
Class: Correctness / test infrastructure

Status: Open
Regression Test: `repository/tests/s3_backend.rs`

`cargo test -p mfb_repository --all-features` cannot compile the S3 integration-test target.  The
target's GC fixture calls `Store::publish_package_version` without the required publish-metadata
argument, so Cargo exits before either ignored live-S3 test can be selected or run.  This is a
feature-gated regression: the default repository test command does not compile the test's body
because its crate is guarded by `#![cfg(feature = "s3")]`.

The single correct behavior is that the test target builds with the `s3` feature enabled, retaining
its existing test fixture semantics: its synthetic published package has no author, URL, or
description metadata.

References:

- Found by the repository review on 2026-09-11.
- `repository/src/store.rs:Store::publish_package_version`
- `repository/tests/s3_backend.rs:s3_gc_reclaims_only_the_orphan`
- Commit `4e210db4ca` added the `PublishMetadata` parameter; the stale S3 call predates it.

## Failing Reproduction

From the workspace root:

```sh
cargo test -p mfb_repository --all-features
```

- Observed: compilation fails with `E0061`: `publish_package_version` takes eight arguments but
  the call in `repository/tests/s3_backend.rs:169` supplies seven; Cargo identifies the missing
  `&PublishMetadata` argument.
- Expected: the all-features test target compiles. The two live-S3 tests remain ignored unless the
  operator deliberately supplies their S3 environment and selects `--ignored`.

Contrast: `cargo test -p mfb_repository` completes the default 330-test suite, because the S3
test crate has `#![cfg(feature = "s3")]`.

| Environment | Command | Result |
| --- | --- | --- |
| Default features | `cargo test -p mfb_repository` | works ✓ |
| `s3` feature enabled | `cargo test -p mfb_repository --all-features` | fails to compile ✗ |

## Root Cause

`Store::publish_package_version` at `repository/src/store.rs:1772` accepts
`metadata: &PublishMetadata` as its eighth argument and persists those values. The test fixture at
`repository/tests/s3_backend.rs:s3_gc_reclaims_only_the_orphan` still uses the seven-argument form
that existed before commit `4e210db4ca`. Rust therefore rejects the integration-test crate at
compile time. The other callers are immune: `rg -n 'publish_package_version\\(' repository --glob
'*.rs'` finds this one stale call; the production path and unit-test fixtures provide metadata.

## Goal

- Make every repository call to `Store::publish_package_version` match its current eight-argument
  contract, and restore a compiling all-features test target.

### Non-goals (must NOT change)

- Do not alter `Store::publish_package_version`'s public/internal contract or make metadata
  optional through another overload merely to accommodate an outdated fixture.
- Do not enable or weaken the ignored live-S3 tests, change S3 credentials/configuration, or alter
  garbage-collection behavior.
- Do not modify package metadata semantics; this fixture must explicitly represent absent metadata.

## Blast Radius

- `repository/tests/s3_backend.rs:s3_gc_reclaims_only_the_orphan` — fixed by this bug: the only
  stale seven-argument caller.
- `repository/src/server.rs:publish_package` — unaffected: passes parsed `PublishMetadata`.
- `repository/src/{store,server,backfill,gc}.rs` test and production callers — unaffected: the
  repository-wide call-site search found each already supplies the metadata argument.

## Fix Design

Add `&PublishMetadata::default()` to the stale test-fixture invocation and import the type from
`mfb_repository::store`. This directly expresses the fixture's intended absence of optional
metadata and keeps the production method contract uniform. Do not change the test from ignored to
non-ignored: its execution requires a real S3-compatible endpoint by design.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Preserve the documented all-features compilation failure as the RED reproduction.
- [x] Complete the repository-wide `publish_package_version` call-site audit and record each
      classification above.

Acceptance: the documented command fails for the missing eighth argument; the audit identifies no
other stale caller.
Commit: —

### Phase 2 — the fix

- [ ] Update `repository/tests/s3_backend.rs` to pass an explicit default `PublishMetadata` value.
- [ ] Keep the S3 GC fixture's no-metadata semantics explicit in its code or adjacent comment.

Acceptance: the all-features target compiles; the fixture still describes an artifact with absent
author, URL, and description.
Commit: —

### Phase 3 — validation

- [ ] Run `cargo fmt --check -p mfb_repository`.
- [ ] Run `cargo test -p mfb_repository --all-features`.
- [ ] When a disposable S3-compatible endpoint is available, run the documented ignored
      `repository/tests/s3_backend.rs` tests against it.

Acceptance: all-features tests are green; live-S3 tests are run separately when their required
environment is supplied.
Commit: —

## Validation Plan

- Regression test: the existing `repository/tests/s3_backend.rs` integration-test target must
  compile under `--all-features`.
- Runtime proof: execute its ignored tests against MinIO or another disposable S3-compatible
  endpoint using the commands in that file's header.
- Doc sync: none expected; `repository/tests/s3_backend.rs` already documents its environment.
- Full suite: `cargo test -p mfb_repository --all-features`.

## Summary

This is a contained feature-gated call-site drift. The only risk is restoring compilation without
making the test fixture accidentally claim package metadata that it intentionally omits; an explicit
default metadata value is the narrow fix.
