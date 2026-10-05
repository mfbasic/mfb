# bug-835: `typeName(x)` under an inline `TRAP` fails the build with an internal error (`native code plan does not emit runtime helper '_mfb_rt_general_typeName'`)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/trap/inline-trap-typename-dead-handler-rt (to add)

The spec explicitly allows trapping the infallible inline built-in `typeName`. Its
handler is dead code and is flagged with the advisory warning
`TYPE_INLINE_TRAP_DEAD_HANDLER`, and "the program still compiles and runs, returning the
call's value". In practice the front end emits the warning, and then native codegen
aborts the build:

```
warn[2-203-0104 TYPE_INLINE_TRAP_DEAD_HANDLER]: inline TRAP handler is unreachable — the guarded call cannot fail
error: native code plan does not emit runtime helper '_mfb_rt_general_typeName'
```

No executable is written. Every inline-TRAP position fails the same way: `LET` (named
or bare `TRAP`) and assignment to a `MUT`. `len(...) TRAP(e)`, the sibling infallible
general built-in, compiles, runs, and prints its value.

The single correct behavior: `LET q = typeName(5) TRAP(e) / RECOVER "" / END TRAP`
builds with the dead-handler warning and prints `Integer`.

References:

- `src/docs/spec/language/08_error-model.md` §8.6 rule 11: "Trapping a
  provably-**infallible** inline-lowered built-in — `len`, `typeName`, … — is
  **allowed** but the handler is dead code, flagged by the advisory warning
  `TYPE_INLINE_TRAP_DEAD_HANDLER` (the program still compiles and runs, returning the
  call's value…)".

## Failing Reproduction

```basic
IMPORT io
FUNC main() AS Nothing
  LET q = typeName(5) TRAP(e)
    RECOVER ""
  END TRAP
  io::print(q)
END FUNC
```
`mfb build /tmp/hardening/B01/repro/r_typename`

- Observed: the warning above, then `error: native code plan does not emit runtime helper '_mfb_rt_general_typeName'`. No binary is produced.
- Expected: the warning, a binary, and output `Integer`.

Contrasts: `LET q = typeName(5)` with no TRAP prints `Integer` ✓.
`LET q = len([1]) TRAP(e) … RECOVER 0` warns, then prints `1` ✓.

## Root Cause

Hypothesis, ranked:

1. The inline-TRAP lowering wraps the root call as a `CallResult`
   (`src/ir/lower.rs:lower_inline_trap`, `check_root` with `hoists.is_empty()` → true).
   The `CallResult` path for a general built-in references the runtime helper symbol
   `_mfb_rt_general_typeName`. The native plan never emits that helper, because every
   plan/collector site special-cases `target == "typeName"` as compile-time folded:
   `src/target/shared/plan/symbols.rs:collect_platform_imports_from_value` (both the
   `RuntimeCall` and the `Call | CallResult` arms),
   `src/target/shared/validate/capabilities.rs:collect_runtime_calls_from_value`, and
   `src/target/shared/plan/function_builder.rs`. So the CallResult lowering asks for a
   symbol that by design does not exist. The error is raised at
   `src/codegen/engine/builder/mod.rs` ("native code plan does not emit runtime helper").
2. `len` survives because it has an inline lowering on the CallResult path. `typeName`
   has only the fold.

## Goal

- An inline TRAP around `typeName` folds to the string constant with an always-Ok result
  (or drops the check, since the handler is dead), and the program runs.

### Non-goals

- Keep the `TYPE_INLINE_TRAP_DEAD_HANDLER` warning.

## Blast Radius

- Every name-keyed infallible inline built-in under an inline TRAP should be checked:
  `len` is OK (measured). The total `bits::*` ops and `collections::contains`/`hasKey`/
  `getOr` compiled and ran under a TRAP in this session (`bits::band(1, 2) TRAP`,
  `collections::contains([1], 1) TRAP`, `collections::getOr([1], 5, 0) TRAP`), so only
  `typeName` was observed failing.

## Fix Design

In `lower_inline_trap`, do not wrap a root call that `inline_builtin_is_infallible`
proves total in a `CallResult`. Instead, lower it as a plain value and keep the handler
as dead code. Alternatively, teach the `CallResult` codegen path the `typeName` fold.

## Validation Plan

- An rt fixture (LET / MUT-assign / bare TRAP forms) printing the value. Then the full
  suite.

## Summary

The bug is a missing fold on one lowering path. It is low risk.
