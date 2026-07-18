# bug-287: `mfb test` fails when the first file imports `io` (or `collections`/`fs`) only under an alias

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed 2026-07-18
Regression Test: tests/ (new) — a project whose file does `IMPORT io AS console` + a TESTING block runs its tests

The synthesized test driver calls `io.print` (and the coverage helpers call
`collections.*` / `fs.*`). `ensure_import` decides an import is already present by
checking only `import.module == "io"`, but per spec `IMPORT io AS console` does not
introduce the name `io`. So when the user imports `io` only under an alias,
`ensure_import` wrongly concludes `io` is available, adds no plain import, and every
driver `io.print` fails resolution — `mfb test` errors on a project that `mfb build`
compiles fine.

The single correct behavior a fix produces: `mfb test` synthesizes a working driver
regardless of whether the user imports `io`/`collections`/`fs` plainly, under an
alias, or not at all.

References:

- `src/docs/spec/language/13_modules-and-packages.md:55` (aliased import does not
  bind the plain name).
- Found during goal-06 review of `src/testing.rs`.

## Failing Reproduction

```
IMPORT io AS console
TESTING "t"
  TCASE "c"
    assert 1 = 1
  END TCASE
END TESTING
```

- Observed: `mfb build` succeeds; `mfb test` emits repeated `SYMBOL_UNKNOWN_IMPORT:
  Package io is used but not imported in this file`.
- Expected: `Tests: 1 Pass: 1`.

Contrast (correct today): plain `IMPORT io` passes.

## Root Cause

`src/testing.rs:156-164` (`ensure_import`) checks only `import.module == "io"`,
counting an aliased import as satisfying the plain name; the coverage helpers share
the flaw for `collections`/`fs` at `src/testing/desugar.rs:430-431`.

## Goal

- `ensure_import` requires `import.alias.is_none()` for the existence check (a
  second plain import of the same module is legal), so an aliased-only import does
  not suppress the injected plain import.

### Non-goals (must NOT change)

- The driver's use of `io.print` / coverage helpers.
- Behavior when the user already has a plain import (must not double-inject in a way
  that errors).

## Blast Radius

- `testing.rs:ensure_import` — fixed here.
- `desugar.rs:430-431` (collections/fs coverage helpers) — same fix.

## Fix Design

Add `&& import.alias.is_none()` to the existence check (or route the driver's calls
through the existing alias). Prefer adding a plain import — a module may be imported
both plainly and aliased. Rejected alternative: emitting `console.print` — the
driver can't know the user's alias name generally.

## Phases

### Phase 1 — failing test
- [ ] Test project with `IMPORT io AS console` + TESTING; confirm `mfb test` fails
      today.
### Phase 2 — the fix
- [ ] Require `alias.is_none()` in `ensure_import` and the two coverage-helper sites.
### Phase 3 — validation
- [ ] Full suite green; `mfb test` passes the repro.

## Validation Plan

- Regression: the aliased-import test project.
- Runtime proof: `Tests: 1 Pass: 1`.
- Doc sync: none.

## Summary

A one-condition scoping bug in the test-driver synthesis; requiring the import be
unaliased fixes it. Same fix at the two coverage-helper sites.

## Resolution

`ensure_import` decided a module was already available by matching
`import.module == module` alone. Per spec 13, `IMPORT io AS console` binds *only*
`console`, so an aliased-only import suppressed the injected plain import and the
synthesized driver's `io::print` — and the coverage helpers' `collections::` /
`fs::` — failed to resolve. The check now also requires `import.alias.is_none()`.

Safe because `SYMBOL_DUPLICATE_IMPORT` keys off the *binding* name
(`src/resolver/resolution.rs`), so `io` and `console` coexist.

Tests: `testing::tests::ensure_import_ignores_an_aliased_import` and the fixture
`tests/syntax/testing/testing-aliased-import-valid`, which aliases all three
packages and ships `testrun` plus coverage goldens so the driver and the
coverage-helper paths are both guarded.

### Three corrections to this report

1. **Its reproduction does not compile.** `TESTING "t"` with a bare `TCASE` is a
   parse error — `TESTING` takes no description and requires a `TGROUP`. The bug
   is real, but as written the repro never reached the resolver.
2. **"Two fix sites" is wrong.** `src/testing/desugar.rs:430-431` are *calls to*
   `ensure_import`, not a duplicated predicate. One edit covers all three
   modules; there is no second edit to make.
3. The `SYMBOL_UNKNOWN_IMPORT` diagnostic anchors to the user's `IMPORT` line,
   not to the synthesized driver.

A grep for other AST-level import injectors confirms `ensure_import` is the only
one, so the single edit is complete end to end.
