# bug-613: a program's top-level initializer that calls into a package sees the package's globals still ZERO

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (silent wrong value)

Status: Open
Regression Test: none yet — see Phase 1

A program's top-level `LET`/`MUT` initializer that calls a package function reading
that package's own global gets the global's **zero value**, not its declared
initializer. It builds with no diagnostic and runs with exit 0 — the value is just
wrong. `LET top = test::getVar()` where the package declares
`EXPORT MUT SomeVar AS Integer = 200` and `getVar` returns it prints `0`.

It is silent: nothing marks the value as uninitialized, and the same call made from
`main` returns `200`, so it passes any test that doesn't read at top level.

**The single correct behavior a fix produces:** an imported package's top-level
bindings are initialized before any binding of a project that imports it, so `top`
prints `200`.

References:

- `mfb spec language` §13 (modules and packages); bug-551 measured that package globals
  are initialized "before `main`", which is true — but not before the consumer's own
  globals.
- bug-612: found in the same session. It blocks the direct read form
  (`LET top = test::SomeVar`); once bug-612 is fixed that form hits THIS bug too.

## Failing Reproduction

Measured 2026-09-13, release binary at `bfb0cfbfc`, macOS aarch64. Package `test`
and `app/project.json` exactly as in bug-612; `app/src/main.mfb`:

    IMPORT test
    IMPORT io

    LET top = test::getVar()

    FUNC main() AS Integer
      io::print(toString(top))
      RETURN 0
    END FUNC

```
mfb build pkg && cp pkg/test.mfp app/packages/ && mfb build app && app/build/capp.out
```

- Observed: build exit 0, prints `0`, exit 0.
- Expected: prints `200`.

Contrast (works): a single project with `MUT own AS Integer = 7`, `FUNC getOwn()`
returning it, and `LET top = getOwn()` declared after `own` prints `7`.

## Root Cause

`mfb build --nir app` shows the merged initializer:

    "name": "__mfb_init_globals_capp",
    { "op": "storeGlobal", "name": "top", ... "target": "5ee6e04073f3898b.test.getVar" },
    { "op": "storeGlobal", "name": "5ee6e04073f3898b.test.SomeConst", ... "100" },
    { "op": "storeGlobal", "name": "5ee6e04073f3898b.test.SomeVar",   ... "200" }

`src/target/shared/nir/lower.rs:lower_global_initializer` emits one `storeGlobal` per
`IrProject::bindings` entry **in vector order**. `src/ir/package.rs:merge_package`
appends the package's bindings after the consumer's (`push_unique` into
`project.bindings`), so the consumer's `top` runs first and `getVar()` reads the
still-zero slot.

Hypothesis still to confirm in Phase 1: whether one package's initializer that calls
a *second* package (a package-to-package import) has the same ordering, which depends
on the order `merge_package` is called in.

## Goal

- The reproduction prints `200`.
- For any import graph, every package's bindings initialize before the bindings of
  every project/package that imports it; bindings within one project keep their
  declaration order.

### Non-goals (must NOT change)

- Declaration order of bindings within one project.
- The `.mfp` format and symbol naming (`_mfb_global_<project>_…`).
- bug-551's behavior (shared `EXPORT MUT` slot, writes visible to the package).
- Thread-worker re-initialization semantics (spec on `thread::start`): workers
  re-run the same initializer; the fix must keep it one function or update both.
- **Tempting wrong fix (forbidden):** lazily initializing on first read, or
  constant-folding package initializers into the consumer. The first changes
  side-effect timing the spec pins; the second only masks the order for literals.

## Blast Radius

- `src/ir/package.rs:merge_package`: binding order — fixed by this bug (or ordering
  in `lower_global_initializer`).
- `src/target/shared/nir/lower.rs:lower_global_initializer`: order of `storeGlobal`.
- Package → package imports (diamond, chain) — to audit in Phase 1.
- Thread workers' re-initialization — uses the same initializer; verify after the fix.
- bug-612's direct-read form — hits this bug once bug-612 lands; its test must
  cover it.

## Fix Design

Order the merged bindings so dependencies come first: merge packages in dependency
order, and prepend (not append) each package's bindings ahead of its importers', or
have `lower_global_initializer` emit package bindings (identity-prefixed names)
before the root project's. Recursive imports need a topological order. The risk is
in diamond imports and in `.ir`/`.nir`/`.ncodesum` golden drift: every fixture that
imports a package with globals changes init order. Each drifted golden must be
inspected as an ORDER-only change.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Runtime test: the reproduction asserts `200`. Confirm RED (prints `0`).
- [ ] Add a chain test (app → pkgA → pkgB, pkgA's top-level initializer calls pkgB)
      and record its current result here.
- [ ] Audit the Blast Radius rows.

Acceptance: tests RED for the documented reason; audit verdicts recorded.
Commit: —

### Phase 2 — the fix

- [ ] Dependency-first binding order (see Fix Design).

Acceptance: Phase 1 tests pass; same-project declaration order unchanged.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/test-accept.sh`, `artifact-gate.sh all`; every drifted golden checked to be
      an order-only change.
- [ ] `cargo test --release --no-fail-fast`.
- [ ] Re-run the reproduction on macOS and the Linux boxes.

Acceptance: full suite green; golden delta is init order only.
Commit: —

## Validation Plan

- Regression tests: Phase 1 runtime tests.
- Runtime proof: reproduction prints `200`.
- Doc sync: consider stating the initialization order (packages before importers) in
  `mfb spec language` §13; it is not written down today.
- Full suite: `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- Order in `merge_package` vs. in `lower_global_initializer` — recommend
  `merge_package`, so the IR itself carries the order every backend sees.

## Summary

A silent wrong value with a small, well-localized cause. The risk is in import-graph
ordering (diamonds, chains) and in the wide order-only golden drift the fix produces.
