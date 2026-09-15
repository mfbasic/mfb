# bug-630: `thread::waitFor(thread::start(…))` does not compile — the result type is the generic `Out`

Last updated: 2026-09-14
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: src/codegen/builtins/thread/tests.rs + a tests/rt-behavior/threads fixture (to add, Phase 1)

A `thread::waitFor` (and, by the same mechanism, `thread::receive` / `thread::accept`) whose
handle argument is a nested call instead of a binding fails to build with an internal error:

```
error: native inlined field size not available for type 'Out' while lowering assign total while lowering while
```

The same program with the handle bound first (`LET t AS Thread OF … = thread::start(…)`,
then `thread::waitFor(t)`) builds and runs. The frontend accepts the nested spelling, so this
is a code-generation failure on a valid program.

**The single correct behavior a fix produces:** a thread read whose handle argument is a nested
`thread::start(…)` call gets the same concrete result type as the bound spelling, and the
program builds and runs with the same output.

References:

- `src/docs/spec/language/16_threads.md` (`thread::waitFor` returns `Out`).
- Found while fixing bug-622 (probe `tw_temp_handle`). Root cause traced read-only on the
  `worktree-B-622` tree.
- bug-479 (the `thread.start` arm of `thread_runtime_return_type`, same area).

## Failing Reproduction

`mfb build --debug` of:

```
IMPORT io
IMPORT thread

ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  RETURN len(seed)
END FUNC

SUB main()
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 100
    total = total + thread::waitFor(thread::start(work, "abc"))
    i = i + 1
  END WHILE
  io::print("total=" & toString(total))
END SUB
```

- Observed: `error: native inlined field size not available for type 'Out' while lowering
  assign total while lowering while` (macOS aarch64; the main checkout's release `mfb` built
  2026-09-14 08:09 and the `worktree-B-622` build both fail identically).
- Expected: builds; prints `total=300`.

Contrast: `LET t AS Thread OF String TO Integer = thread::start(work, "abc")` then
`total = total + thread::waitFor(t)` builds and prints `total=300`.

## Root Cause

Two gaps line up (traced, not yet test-confirmed — Phase 1 confirms each):

1. `CodeBuilder::thread_runtime_return_type`
   (`src/codegen/memory/value/builder_value_semantics.rs`, the `"thread.waitFor"` arm) types
   the result from `self.static_type_name(args.first()?)`. For a `Local` handle that is the
   binding's `ThreadHandle { out, .. }`. For a nested `NirValue::Call` / `CallResult` /
   `RuntimeCall` to `thread.start`, `static_type_name`'s call arm is a hand-written table with
   no `thread.start` entry, so it answers `None` and the arm never reaches the `ThreadHandle`
   match that would yield `Integer`.
2. The resolver in `src/codegen/engine/value/builder_values.rs` then falls through its
   `.or_else` chain to `runtime::spec_for_call(target).map(|spec| ParameterType::declared(spec.abi.returns))`.
   `src/target/shared/runtime/catalog.rs` (`abi_return_name`, in `supported_helper_specs`)
   derived that `abi.returns` from the registry descriptor's raw return type with `.name()`
   and no `contains_var` guard, so `thread.waitFor`'s `Var("Out")` is frozen as the string
   `"Out"`. `registry::call_return_type_typed` has that guard and declines; the catalog
   fallback does not. The bogus `declared("Out")` reaches the assignment's marshalling, whose
   inlined-size lookup (`src/codegen/collection/layout/builder_collection_layout.rs`) raises
   the error.

## Goal

- The reproduction builds and prints `total=300`; `thread::receive(thread::start(…))` and
  `thread::accept(thread::start(…))` shapes type correctly too.
- No generic descriptor return type can reach a value's type through the catalog fallback.

### Non-goals (must NOT change)

- Bound-handle thread calls and their generated code.
- bug-479's parent-kind correction for `thread.start`.
- Not a fix: special-casing the error site to skip the size lookup for an unknown type (it
  would mis-lower the value instead of failing).

## Blast Radius

To be verified by search in Phase 1:

- `thread.receive`, `thread.acceptResource`, `thread.readResource` arms of
  `thread_runtime_return_type` — same `static_type_name(args.first())` gap for a nested
  handle argument.
- Every registry member whose descriptor return type contains a type variable — all reach the
  same unguarded catalog fallback when their own resolver declines; audit the list.
- The sibling type walks `static_nir_value_type` / `static_type_name_with_types` — check
  whether they share the missing `thread.start` call case (`.ai/compiler.md` on sibling walks).

## Fix Design

Close the leak at its origin: the catalog fallback must not produce a spec return type from a
descriptor that contains a type variable (the same `contains_var` rule
`call_return_type_typed` applies), so an unresolved generic fails loudly at the resolver
instead of flowing on as a declared type. Then fix the imprecision: let
`thread_runtime_return_type` resolve a nested handle argument — give `static_type_name`'s call
arm a `thread.start` case that reuses the existing `thread.start` arm, rather than a second
copy of it.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [ ] `src/codegen/builtins/thread/tests.rs` (or the builder's type tests): a nested
      `thread.start` handle argument types `thread.waitFor` to the worker's `Out`; confirm RED.
- [ ] `tests/rt-behavior/threads/`: a fixture running the reproduction (plus `receive` /
      `accept` nested shapes); confirm the build fails today.
- [ ] Audit the Blast Radius list with a verdict per site.

Acceptance: the tests fail for the documented reason; every site has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] Guard the catalog fallback against type-variable returns (`catalog.rs`).
- [ ] Nested `thread.start` handle typing (`builder_value_semantics.rs`), shared with any
      sibling walk the audit finds.

Acceptance: Phase 1 tests pass; bound-handle programs byte-identical.
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Regenerate the new fixture's goldens; full suite; `scripts/test-accept.sh`.

Acceptance: full suite green; no golden outside the new fixture moves.
Commit: —

## Validation Plan

- Regression tests: Phase 1 unit test + fixture.
- Runtime proof: the reproduction prints `total=300`.
- Doc sync: none expected (the spec already states `waitFor` returns `Out`).
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- Catalog guard — decline (no spec return, the resolver errors) vs. a sentinel the resolver
  refuses explicitly. Recommended: decline, if the audit shows no caller depends on the
  fallback for a generic member.

## Summary

A two-line typing gap exposed by a catalog fallback that should never have produced a generic
name. The risk is in the audit of other generic members that may be silently leaning on that
fallback today.
