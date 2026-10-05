# bug-825: a raising comparison inside AND/OR escapes the inline TRAP

Last updated: 2026-10-04
Effort: small (guess)
Severity: MEDIUM
Class: Correctness (error routing)

Status: Open
Regression Test: proposed — an invalid fixture expecting
`TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`, plus a valid control without AND/OR whose
inline handler catches the error.

Found by the plan-165 review (2026-10-04), on main `85a5f0a29`.

## Failing Reproduction

`/tmp/p165/c1`. With `fx` a `Fixed` and `big` either `3000000000.0` or
`3000000000`:

```
LET b = finish(selected AND fx < big) TRAP(e)
```

It compiles. At runtime the comparison's promotion raises 77050010
(`ErrOverflow`), and the error reaches the **function-level** handler rather than
this inline `TRAP`. Without the `AND`, the inline handler catches it.

Spec `language/08_error-model.md` rule 11 requires a fallible call or raising
operator inside a short-circuited `AND`/`OR` operand to be rejected with
`TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`.

## Root Cause

The short-circuit check in `src/ir/shape.rs` asks only `operator_can_raise`.
bug-816 added `ir::fallible::comparison_can_raise` to four fallibility checks but
not to this one. The Integer/Fixed form predates bug-816; bug-816 widened it to
Float/Fixed.

## Also fix (found in the same review)

- **Spec error, rule 11 (commit 107e78bdb):** it says `LET s = a & b TRAP(e)`
  gets `TYPE_INLINE_TRAP_DEAD_HANDLER`. It is actually rejected with
  `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` (`/tmp/p165/d1`). Correct the sentence to the
  current behavior.
- **Stale comments:** three comments still say `toString` is infallible except on
  `List OF Byte`, contradicting the owner's 2026-10-04 decision (every overload is
  fallible):
  - `builtins::arg_type_makes_inline_builtin_fallible` and
    `builtins::inline_builtin_is_infallible` (`src/codegen/builtins/mod.rs`);
  - `Fallibility::call_is_fallible` (`src/ir/fallible.rs`).

## Goal

The short-circuit check treats a raising comparison (`comparison_can_raise`)
exactly like a raising operator. The example above is rejected with
`TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`, and the spec and comments match the code.
