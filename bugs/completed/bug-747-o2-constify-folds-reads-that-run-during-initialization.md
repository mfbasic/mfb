# bug-747: `-O2` constification folds a global read that runs before the global is initialized

Last updated: 2026-09-26
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: FIXED
Regression Test: tests/runtime/rt_global_init_order_fold.rs; `optimizer::opt1::globals::tests::reads_that_can_run_during_initialization_are_kept`; `optimizer::opt1::plans::globals::tests::initializer_reachability_follows_calls_and_function_values`

## STATUS: FIXED (see the commit that archived this file)

Found while designing bug-740's literal-`LET` folding (plan-161-A), which
needs the same guard. The fix is the guard: a new analysis,
`initializer_reachable` (`src/optimizer/opt1/plans/globals.rs`), and the
constification row now skips every function it returns
(`substitute_outside_initialization`, `src/optimizer/opt1/globals.rs`). No
deviation from the design below.

The single correct behavior: the `-O` level never changes the value a program
reads from a module binding. A read that runs before the binding's own
initializer store observes whatever it observes at `-O0`, at every level.

## Failing Reproduction

```
IMPORT io
LET A AS Integer = f()
LET C AS Integer = B + 1
PRIVATE LET B AS Integer = 5

FUNC f() AS Integer
  RETURN B * 2
END FUNC

SUB main()
  io::print(toString(A))
  io::print(toString(C))
  io::print(toString(B))
END SUB
```

`mfb build -O<n>` then run, at main `5c4ea72e3`:

| Level | Output |
| --- | --- |
| `-O0` | `0 1 5` |
| `-O1` | `0 1 5` |
| `-O2` | `10 6 5` |
| `-O3` | `10 6 5` |

The same happens with `PRIVATE MUT B` (never written). A `PUBLIC` `LET` is
unaffected at main, because the row only handles private globals.

## Root Cause

Bindings initialize one at a time in declaration order ("Within one project,
bindings initialize in declaration order", `mfb spec language
modules-and-packages`), from the `__mfb_init_globals_<project>` SUB, and a slot
whose store has not run yet reads zero. `A`'s initializer calls `f`, which reads
`B` before `B`'s store; `C`'s initializer reads `B` directly, in the initializer
SUB, before the store.

`constify` (`src/optimizer/opt1/globals.rs`, the Level-2 "Global localization /
constification" row) replaced every read of a never-written private global
with its literal initializer, in every function including the initializer SUB.
For a read that runs before the store, the literal is not the value the
program reads, so the rewrite changed the result. The dial's contract is
"levels change emitted code, never results" (`mfb man optimizations`).

## Goal

- `tests/runtime/rt_global_init_order_fold.rs`: the program above prints
  `0 1 5` at `-O0` through `-O3`, for `PRIVATE LET`, `PRIVATE MUT` and `LET`.

### Non-goals (must NOT change)

- The zero read itself. Whether reading an uninitialized binding should be a
  diagnostic is a language question; this bug only makes every level agree
  with `-O0`.
- Folding in code that only runs after initialization: `main`'s read of `B`
  still becomes the literal at `-O2`.

## Blast Radius

- `constify` (Level 2): fixed here.
- Dead-global elimination (Level 2): unaffected. It removes a global only when
  nothing reads it after constification, and a kept read keeps the global.
- Read-only memory inference (Level 2): unaffected. It only clears `mutable`,
  which changes no value.
- plan-161-A's new Level-1 literal-`LET` folding row uses the same guard from
  the start.

## Fix Design

`initializer_reachable(module)` returns the initializer SUB plus every function
it can reach: direct `Call`/`CallResult` targets that name module functions, and
every function mentioned as a value (`FunctionRef`, `Closure`), transitively. A
function passed to a builtin such as `collections::forEach` or `thread::start`
may be invoked, so it counts as reached. `constify` substitutes only in
functions outside that set.

## Phases

### Phase 1 — failing test

- [x] `tests/runtime/rt_global_init_order_fold.rs`: three cases (`PRIVATE LET`,
      `PRIVATE MUT`, `LET`) built at `-O0..-O3`. At base, `MFB_TEST_EXE=<main
      debug mfb> cargo test --test rt_global_init_order_fold` fails the two
      private cases with `left: "10\n6\n5\n"` (the `-O2` output).

### Phase 2 — the fix

- [x] `initializer_reachable` in `src/optimizer/opt1/plans/globals.rs`, with a
      unit test for calls, transitive calls, function values and closures.
- [x] `constify` skips initializer-reachable functions; unit test
      `reads_that_can_run_during_initialization_are_kept`.

Acceptance: the Phase 1 test passes (3/3).
Commit: (this commit)

### Phase 3 — full validation

- [x] No golden moves: every golden is built at the default `-O1`, where the
      row is off. Covered by plan-161-A's full-suite run, which includes this
      commit.

## Summary

This was one wrong assumption: "never written" does not mean "always holds its
initializer", because the initializer runs partway through a sequence the
program can observe. The fix limits the rewrite to code that can only run after
that sequence has finished.
