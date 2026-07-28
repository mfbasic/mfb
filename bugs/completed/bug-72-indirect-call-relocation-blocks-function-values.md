# bug-72: Calling a function value by name (`f(2, 3)`) never links — the object plan records the indirect call as a relocation to a local variable's name

Last updated: 2026-07-09
Effort: small (<1h)

Calling a `FUNC`-typed binding — a lambda, a `FUNC` parameter, or a local holding a
function reference — **always fails the build**, on every platform, with an
internal linker error naming the *variable*:

```
error: native object plan relocation target 'addTwo' is neither defined nor imported
```

Code generation is not at fault: it already emits a correct indirect call, and
skipping the bogus relocation makes every case run correctly (verified below). The
defect is in the object-plan bookkeeping, which turns *every* call into a symbol
relocation — including a `CallKind::Indirect` call, whose "symbol" is the local
binding's name (`addTwo`, `f`, `g`) and can therefore never be defined or imported.
`NativeObjectPlan::validate` then rejects the plan.

Because calling a function value is the whole point of having one, this makes the
language's first-class-function surface **unusable by direct call**. Everything
else about function values works: they can be bound, returned from a function,
stored in a record field, and passed to `collections::transform`/`filter`/`reduce`
(whose inline lowering performs the indirect call itself, bypassing the plan's call
list).

The single correct behavior a fix produces: a call through a `FUNC`-typed local,
parameter, or lambda binding links and executes, calling the target function value;
no relocation is emitted for a call that has no symbol.

References:

- `src/docs/spec/linker/02_object-plan.md` — "each call's `CallKind` maps to an
  object-plan relocation kind: … `Indirect` becomes `indirectCall`". The spec
  states the current, wrong behavior and must change with the fix.
- `src/docs/spec/language/06_functions.md`, `.../04_types.md` §"function/lambda
  types" — function values are a first-class type with no stated restriction on
  calling them.
- Found while fixing bug-26 and bug-35 (nested `FUNC` type parsing); the
  reproduction there could not be completed end-to-end because of this bug.
- Memory note: `bugs-20-39-fixed`.

## Failing Reproduction

```basic
' src/main.mfb
IMPORT io

FUNC makeAdder(base AS Integer) AS FUNC(Integer) AS Integer
  LET captured AS Integer = base
  RETURN LAMBDA(value AS Integer) -> value + captured
END FUNC

FUNC main AS Integer
  LET addTwo AS FUNC(Integer) AS Integer = makeAdder(2)
  io::print(toString(addTwo(5)))
  RETURN 0
END FUNC
```

```
$ mfb build .
error: native object plan relocation target 'addTwo' is neither defined nor imported
```

- Observed: the build fails at object-plan validation. Exit 1, no executable.
- Expected: the program links and prints `7`.

The failure is independent of arity, of lambda-vs-`FUNC`-reference, and of whether
the callee is a local or a parameter. All of the following fail identically:

| Form | Result |
| --- | --- |
| `LET g AS FUNC(Integer, Integer) AS Integer = add` then `g(10, 20)` | fails ✗ |
| `FUNC apply(f AS FUNC(Integer, Integer) AS Integer)` calling `f(2, 3)` | fails ✗ |
| `LET addTwo = makeAdder(2)` then `addTwo(5)` (a closure) | fails ✗ |

Contrast cases that work correctly today, and bound the bug:

- Binding, returning, and capturing function values
  (`tests/rt-error/functions/lambda-capture-valid`).
- Passing a function value to a builtin: `collections::transform(xs, addTwo)`
  works, because the builtin's inline lowering emits the `blr` itself and never
  goes through `FunctionBuilder::add_call`.
- A record field of function type: `TYPE Handler { fn AS FUNC(Integer) AS Integer }`
  constructs and runs.
- Calls to a named top-level `FUNC` (`CallKind::Local`) — those have a real symbol.

The reproduction fails on macOS-aarch64 (host) and both Linux object writers share
the identical code, so it is platform-independent.

## Root Cause

`FunctionBuilder::add_call` (`src/target/shared/plan/function_builder.rs:378`)
classifies a call whose target is neither a known function, import, nor runtime
helper as `CallKind::Indirect`, recording `symbol = target` — i.e. the *source
binding's name*, because an indirect call has no symbol at all. The plan is
otherwise correct; `mfb build -nplan` shows:

```json
{ "target": "g", "symbol": "g", "kind": "indirect", "stringLiterals": [] }
```

Both object writers then walk `function.calls` unconditionally and emit one
`ObjectRelocation` per call — `relocations()` in `src/os/macos/object.rs:544` (loop at `:562`) and
`src/os/linux/object.rs:429` (loop at `:447`) — mapping `CallKind::Indirect` to the relocation kind
`"indirectCall"` with `to = "g"`. `NativeObjectPlan::validate`
(`src/os/macos/object.rs:679`, `src/os/linux/object.rs:505`) requires every
relocation's `to` to be in `defined_symbols`, `imported_symbols`, or
`external_symbols`. A local variable name is in none of them, so the build dies.

The machine-code path is unaffected and already correct: `builder_values.rs:592`
(and `:754` for a fallible `CallResult`) sees the target in `self.locals` with a
`FUNC(` type and lowers a genuine indirect call through the callable value. That is why the contrast cases work and why the
one-line experiment below produces correct programs.

**Confirmation.** Adding `if matches!(call.kind, CallKind::Indirect) { continue; }`
to both `relocations()` functions makes all three failing forms above build and
print the correct results (`7`, `5`, `30`). Reverted; no fix is committed.

## Goal

- A call through a `FUNC`-typed local, parameter, or lambda binding builds, links,
  and calls the function value at runtime.
- The object plan emits no relocation for a call that has no symbol target.
- `mfb build -nobj` output for such a program validates.

### Non-goals (must NOT change)

- The machine-code lowering of indirect calls (`builder_values.rs:592`, `:754`) —
  correct.
- Relocations for `Local`, `Runtime`, `Import`, and data references.
- The inline lowering of `collections::transform`/`filter`/`reduce`, which already
  performs its own indirect call.
- **Tempting wrong fix, forbidden:** making `validate()` skip the `to`-is-defined
  check for `indirectCall` relocations while still emitting them. That leaves a
  relocation record whose `to` field is a source-level variable name — a lie in the
  object model that the next consumer will trip over. If the plan is to keep an
  `indirectCall` record at all, it must not carry a symbol in `to`.
- Do not "fix" this by rejecting direct calls on function values in the front end.

## Blast Radius

Searched for every consumer of `CallKind::Indirect` and of `function.calls`:

- `src/os/macos/object.rs:544` (`relocations`, loop at `:562`) — the bug; fixed here.
- `src/os/linux/object.rs:429` (`relocations`, loop at `:447`) — identical code;
  fixed here.
- `src/os/macos/object.rs:168` (`external_symbols(&relocations)`) — derives external
  symbols from the relocation list, so dropping the record also stops `g` from
  being proposed as an external symbol. In scope, and the point.
- `src/target/shared/plan/mod.rs:307` (`NativePlanFunction::validate`) — accepts all
  four `CallKind`s but requires a non-empty `symbol`. If the fix stops storing the
  binding name in `symbol`, this check needs the `Indirect` case exempted. In scope.
- `src/target/shared/plan/json.rs:147` — renders `"indirect"` in the `.nplan` dump.
  Unaffected: the plan's *call list* is a faithful record of what the function
  calls, and keeping `kind: "indirect"` there is correct. Only the *relocation*
  derived from it is wrong.
- `src/os/macos/object.rs:1136` and `src/os/linux/object.rs:886`
  (`lowers_full_plan_covering_every_branch`) — both assert `kinds.contains(
  "indirectCall")`. They will fail with the fix and must be updated to assert the
  opposite (no relocation for an indirect call). This is a deliberate expectation
  change, not a masked test.
- No fixture golden (`.nobj`/`.nplan`) contains `indirectCall` — verified by grep —
  because no fixture can currently compile such a program. No goldens shift.

## Fix Design

Stop deriving a relocation from a call that has no symbol. In both `relocations()`
functions, skip `CallKind::Indirect`; drop `"indirectCall"` from the object-plan
relocation-kind vocabulary. Correspondingly, either stop populating
`PlanCall::symbol` for indirect calls (and exempt `Indirect` from the non-empty
`symbol` check in `NativePlanFunction::validate`), or keep `symbol` as a *display*
copy of the binding name and document that it is never a linker symbol. Prefer the
former: an empty `symbol` makes the "no symbol exists" invariant unforgeable.

Rejected alternatives:

- *Exempt `indirectCall` from validation.* Rejected above (non-goal): it preserves a
  relocation whose `to` names a stack local.
- *Synthesize a symbol for the callee.* There is none to synthesize; the callee is a
  runtime value, possibly a closure allocated this second.

The correctness risk is small and concentrated in the two `relocations()` functions.
The end-to-end proof is that the three reproduction forms run and print the right
numbers, which the confirmation experiment already showed.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add a runtime fixture `tests/rt-behavior/functions/call-function-value-rt`
      exercising all three forms (local, parameter, closure) plus a
      returned-then-called value and asserting the printed results. Confirmed it
      failed pre-fix with the relocation error.
- [x] Add an object-plan unit test (both platforms) asserting an indirect call
      produces no relocation naming the binding (the updated
      `lowers_full_plan_covering_every_branch`).
- [x] Blast-radius audit complete (above).

Acceptance: the fixture fails with `relocation target '…' is neither defined nor
imported`; the audit list has a verdict per site.
Commit: —

### Phase 2 — the fix

- [x] Skip `CallKind::Indirect` in `relocations()` in `src/os/macos/object.rs` and
      `src/os/linux/object.rs`.
- [x] Stop storing the binding name in `PlanCall::symbol` for indirect calls; exempt
      `Indirect` from the non-empty-`symbol` check in `NativePlanFunction::validate`
      (now positively required to be empty).
- [x] Update `lowers_full_plan_covering_every_branch` in both object writers to
      assert an indirect call yields no relocation.

Acceptance: the Phase 1 fixture prints `7` / `5` / `30`; contrast cases (transform,
record field, named calls) still behave identically; nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [x] New fixture ships its own goldens (`build.log`, `.ast`, `.ir`, `.run`); no
      pre-existing golden contained `indirectCall` (grep-confirmed), so nothing
      else shifts.
- [x] Update `src/docs/spec/linker/02_object-plan.md` — removed `indirectCall` from
      the relocation-kind mapping and stated that an indirect call produces no
      relocation because it has no symbol. (`04_symbols-and-relocations.md` already
      documented the correct machine-code behavior — left as-is.)
- [x] `cargo test --bin mfb` (2478 passed). `scripts/test-accept.sh` left for the
      orchestrator per task constraints.
- [x] Re-ran the reproduction on Linux aarch64 (`ssh -p 2223`, glibc) and riscv64
      (`ssh -p 2229`, musl); both print `30 5 7 12`, exit 0.

Acceptance: full suite green; golden deltas confined to the new fixture; the
reproduction runs on macOS and both Linux arches.
Commit: —

## Validation Plan

- Regression test(s): `tests/rt-behavior/functions/call-function-value-rt` plus the
  per-platform object-plan unit tests.
- Runtime proof: the fixture's `.run` golden shows `7`, `5`, `30`, exit 0.
- Doc sync: `src/docs/spec/linker/02_object-plan.md` (relocation-kind mapping).
- Full suite: `scripts/test-accept.sh target/debug/mfb target/accept-actual` and
  `cargo test`.

## Resolution

Fixed. An indirect call now carries no linker symbol and produces no relocation.

Changes:

- `src/target/shared/plan/function_builder.rs` — `add_call` records
  `(CallKind::Indirect, String::new())` instead of the source binding's name, so
  the plan can never mistake a stack local for a linker symbol.
- `src/target/shared/plan/mod.rs` — `NativePlanFunction::validate` now requires
  `target` non-empty for every call, requires a non-empty `symbol` only for
  `Local`/`Import`/`Runtime`, and *positively requires* `Indirect` to carry an
  empty `symbol` (making the "no symbol exists" invariant unforgeable).
- `src/os/macos/object.rs` and `src/os/linux/object.rs` — `relocations()` now
  `continue`s on `CallKind::Indirect`, dropping `indirectCall` from the
  object-plan relocation-kind vocabulary; `external_symbols` naturally stops
  proposing the binding name. Both `lowers_full_plan_covering_every_branch` tests
  updated to assert no `indirectCall` kind and no relocation targeting the binding.
- `src/docs/spec/linker/02_object-plan.md` — relocation-kind mapping corrected.

Verification:

- Bug-doc reproduction builds and runs, printing `7` (exit 0).
- New fixture `tests/rt-behavior/functions/call-function-value-rt` covers a local
  function reference, a `FUNC` parameter, a captured closure, and a
  returned-then-called value; `.run` golden is `30 / 5 / 7 / 12`.
- `mfb build -nobj`/`-nplan` for the fixture validate; nplan shows
  `"symbol": ""` for the four indirect calls; nobj has zero relocations targeting
  a binding and no `indirectCall` kind.
- `cargo test --bin mfb`: 2478 passed. Runtime-proved on macOS aarch64 (host),
  Linux aarch64 glibc (`ssh -p 2223`), and Linux riscv64 musl (`ssh -p 2229`).

## Summary

The engineering risk is almost entirely in the expectation change: two existing unit
tests and one spec paragraph assert the buggy behavior (that an indirect call
becomes an `indirectCall` relocation). The code fix is two `continue`s plus the
`symbol` invariant. Machine-code lowering, the plan's call list, and every other
relocation kind are untouched.
