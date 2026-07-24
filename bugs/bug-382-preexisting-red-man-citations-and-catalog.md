# bug-382: two pre-existing red tests on main — stale man citations (bug-330 aftermath) and an `audio.devices` catalog round-trip failure

Last updated: 2026-07-23
Effort: medium (1h–2h)
Severity: LOW (test-suite red; no shipped-artifact impact — both are metadata/doc consistency checks)
Class: Correctness (documentation/catalog metadata out of sync with code)

Status: Open
Regression Test: `docs::man::tests::man_citations_resolve`, `target::shared::runtime::catalog::tests::catalog_is_consistent`

`cargo test --release` on main fails **exactly two** tests (3201 passed, 2 failed):
`man_citations_resolve` and `catalog_is_consistent`. Both are metadata-consistency
checks (man-page citations resolving to real symbols; the runtime spec catalog
round-tripping). Neither affects emitted binaries. The single correct behavior a
fix produces: **`cargo test` is green on main** — every man citation resolves to
a symbol that exists in the cited file, and every catalogued call round-trips
through `spec_for_call` to its own spec object.

These were discovered during plan-47-A and **proven not caused by it**: 47-A
(`2366793a9`) touches only `src/target/shared/code/` (function bodies + 5 helper
signatures), renames/moves no symbol, and edits no man page, spec, or catalog
file. The failing citations reference symbols that moved in **bug-330**, long
before plan-47. Recording the pre-existing red baseline so future plan-47 units'
failures are distinguishable from it.

References:

- `src/docs/man/mod.rs:318` (`man_citations_resolve`) — the strict symbol-level
  citation check.
- `src/target/shared/runtime/catalog.rs:229` (`catalog_is_consistent`) — the
  `spec_for_call` round-trip assertion.
- bug-330 (`4540c0e31` "dispatch TLS backends from the package parent, not
  openssl.rs") — moved the `lower_tls_*_helper` functions from `tls/openssl.rs`
  to `tls/mod.rs` without sweeping the man citations.
- Memory note "Splits must sweep man AND spec citations" — this is that failure
  mode; `fix_citations.py` is known broken, so the repoint is by hand.
- Found during plan-47-A (`planning/plan-47-A-platform-family-match.md`).

## Failing Reproduction

```
cargo test --release man_citations_resolve
cargo test --release catalog_is_consistent
```

- Observed (`man_citations_resolve`), panic at `src/docs/man/mod.rs:318`
  "unresolvable man citations:", 15 stale citations:
  - `audio/openInput`, `audio/openOutput` → `audio/alsa.rs:SR_MIN`,
    `audio/macos.rs:SR_MIN`, `audio/macos.rs:BUF_MIN` — those constants now live
    in `src/target/shared/code/audio/common.rs`.
  - `tls/package`, `tls/close`, `tls/readText`, `tls/write`, `tls/writeText` →
    `tls/openssl.rs:lower_tls_{close,read,write,close_listener}_helper` — those
    functions now live in `src/target/shared/code/tls/mod.rs`.
- Observed (`catalog_is_consistent`), panic at
  `src/target/shared/runtime/catalog.rs:229` "spec_for_call audio.devices": the
  round-trip `ptr::eq(spec_for_call("audio.devices").unwrap(), spec)` is false.
- Expected: both tests pass.

Contrast: all other 3201 tests pass; the artifact-gate is 0-diff on all four
targets. The failures are isolated to doc/catalog metadata.

## Root Cause

**`man_citations_resolve` — CONFIRMED.** bug-330 (`4540c0e31`) moved the TLS
dispatch helpers from `tls/openssl.rs` to `tls/mod.rs`, and an earlier audio
refactor moved `SR_MIN`/`BUF_MIN` into `audio/common.rs`, but the man pages that
cite them (`src/docs/man/builtins/tls/*.md`, `.../audio/*.md`) still point at the
old files. The man pages were last edited at bug-327/bug-337 — *before* bug-330 —
so the citations have been stale since bug-330. `man_citations_resolve` is
symbol-level-strict, so a moved symbol is a hard failure. 9 man files are affected
(`grep -rln 'openssl.rs:lower_tls_\|alsa.rs:SR_MIN\|macos.rs:SR_MIN\|macos.rs:BUF_MIN' src/docs/man/`).

**`catalog_is_consistent` — HYPOTHESES (not yet confirmed).** `audio.devices`
appears exactly once as a `call:` in the spec tables
(`audio_specs.rs:13`), yet `spec_for_call("audio.devices")` returns a spec object
that is not pointer-equal to the iterated one. Ordered by likelihood:
1. `audio.devices` is reachable through the catalog aggregation twice (e.g., an
   `AUDIO_SPECS` array included in two chained iterators), so `spec_for_call`
   resolves to the first while the iteration reaches the second. Confirm by
   dumping the aggregated `specs` length vs. the unique `call` count.
2. `spec_for_call`'s search order differs from the iterated list's identity (e.g.
   it searches a rebuilt/cloned array), which would make `ptr::eq` fail — but
   then it would fail for *every* spec, not only `audio.devices`, so this is
   less likely.

## Goal

- `cargo test` green on main: `man_citations_resolve` and `catalog_is_consistent`
  both pass.
- Every man citation points at the file that actually defines the symbol.
- `audio.devices` (and every call) round-trips through `spec_for_call` to its own
  spec object exactly once.

### Non-goals (must NOT change)

- **Do not weaken either test.** The tempting wrong fix — relaxing
  `man_citations_resolve` to file-level, or deleting the `ptr::eq` round-trip — is
  forbidden; the tests are correct and the metadata is wrong.
- No change to emitted bytes, the language, or runtime behavior. This is pure
  metadata repair.
- Do not "fix" by moving the symbols back to the cited files — the code layout
  (post-bug-330) is correct; the citations are stale.

## Blast Radius

- 9 man files under `src/docs/man/builtins/{tls,audio}/` — the stale citations,
  fixed by this bug (repoint `openssl.rs` → `mod.rs`, `alsa.rs`/`macos.rs` →
  `common.rs`).
- Any other man citation to a bug-330/-327/-331-moved symbol — the same sweep
  should re-run `man_citations_resolve` to catch the whole set, not only the 15
  listed (the test reports all at once, so one run is exhaustive).
- The catalog: only `audio.devices` is reported; confirm no other call has the
  same duplication once the aggregation cause is found.

## Fix Design

- **Man citations:** by hand (`fix_citations.py` is broken — wrong `SPEC_DIR`),
  repoint each stale `[[file:symbol]]` to the file that now defines `symbol`.
  Verify with `grep -n "fn <symbol>"` before editing each. Re-run
  `man_citations_resolve` until green — it lists every remaining unresolved
  citation, so iterate to zero.
- **Catalog:** first confirm hypothesis 1 by instrumenting the aggregation
  (length vs. unique-call count); then remove the duplicate registration path (or
  the second spec entry). Keep the round-trip assertion intact.

## Phases

### Phase 1 — confirm + audit (no fix)

- [ ] Re-run both tests; capture the full unresolved-citation list and the
      catalog aggregation length vs. unique-call count. Write the confirmed
      catalog root cause into §Root Cause.

### Phase 2 — repoint man citations

- [ ] Repoint the 9 (or more) stale man files to the current symbol locations;
      `man_citations_resolve` green.

### Phase 3 — fix the catalog duplication

- [ ] Remove the `audio.devices` double-registration (per confirmed cause);
      `catalog_is_consistent` green.

### Phase 4 — full validation

- [ ] `cargo test` fully green; `scripts/artifact-gate.sh` still 0-diff.

## Validation Plan

- Regression tests: the two named tests, plus a full `cargo test`.
- Doc sync: the man pages ARE the doc; no spec change expected.
- Full suite: `cargo test --release` green, `scripts/artifact-gate.sh` 0-diff.

## Summary

Two pre-existing red tests on main, isolated to doc/catalog metadata, proven not
caused by plan-47-A. The man-citation half is a confirmed bug-330 sweep miss (9
files, mechanical repoint by hand); the catalog half needs a short investigation
of the spec aggregation. No shipped artifact is affected. Recorded so the
plan-47 effort can tell its own regressions apart from this known-red baseline.
