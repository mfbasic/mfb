# bug-825: a raising comparison inside AND/OR escapes the inline TRAP

Last updated: 2026-10-05
Effort: small (guess)
Severity: MEDIUM
Class: Correctness (error routing)

Status: Fixed (58a4e76f7, dbcb17396, a30f192e0)
Regression Test: `tests/syntax/trap/inline-trap-fixed-comparison-short-circuit-invalid`,
`tests/rt-behavior/trap/inline-trap-fixed-comparison-short-circuit-rt`, and
`ir::tests::inline_trap_short_circuit::{a_raising_comparison_in_a_short_circuited_operand_names_the_operator,
an_unconditional_or_non_raising_comparison_is_not_refused}`.

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

## Resolution

- **`58a4e76f7`** — `Analyzer::note_short_circuited_binary_operator`
  (`src/ir/shape.rs`) now also asks `fallible::comparison_can_raise` on the
  operands' types (lowering's own `expression_type` oracle), the predicate
  `lower::trap_hoist_kind` lifts on. The repro `/tmp/p165/c1` built and printed
  `function handler 77050010` before; it is now rejected at line 11 with
  `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL` naming `` `<` ``. Audit of the walk
  against `trap_hoist_kind`: calls, arithmetic binary and unary operators were
  already covered; the comparison was the only raise site missing. Covered
  positions (in-process test): right of `AND`, right of `OR`, under `NOT`,
  nested `AND` inside an `OR` operand, and the left side of an `AND` that is
  itself short-circuited. Not refused: a raising comparison as the left operand
  of the outermost `AND`/`OR` (always evaluated, lifted and caught), and
  comparisons that convert nothing into `Fixed` (`Integer`/`Integer`,
  `Byte`/`Fixed`, `Fixed`/`Fixed`). The new in-process comparison test fails
  against HEAD's `shape.rs` and passes with the fix (measured by swapping the
  file and running `cargo test --bin mfb inline_trap_short_circuit`).
  - Fixtures: the invalid fixture rejects three scrutinees (`AND`, `OR`,
    `NOT`-nested). The valid control shows every unconditional position caught
    by its inline handler: whole argument, left of `AND`, left of `OR`
    (`77050010`), a non-finite product left of `OR` (`77050003`), and the
    bind-first workaround the diagnostic suggests.
- **`dbcb17396`** — spec `language/08_error-model.md` rule 11 names the raising
  comparison case. It also corrects the `&` sentence: `LET s = a & b TRAP(e)` is
  rejected with `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` (measured: `/tmp/p165/d1`,
  `error[2-203-0069]` at line 9; `name() & b` with an infallible `name` is the
  same). A `[[trap_init.rs:total_functions]]` citation wrapped in backticks was
  rendering literally; it is fixed (`spec-census.sh --render language`: 0 leaks).
- **`a30f192e0`** — the stale "toString is total except on `List OF Byte`"
  comments are corrected at `arg_type_makes_inline_builtin_fallible`,
  `ArgFallibility`, `inline_builtin_is_infallible`,
  `Fallibility::call_is_fallible`, `fallible::analyze`,
  `LowerContext::call_is_fallible`, `lower_inline_trap`'s root check and
  `verify/resources.rs`'s dead-handler site. The shape.rs copy went in with
  `58a4e76f7`.
- **`f54a44bbc`** (found on the way) — `language/06_functions.md` cited the
  nonexistent `compat.rs:Scope::assignable`. It now cites `assignable`
  (`spec-census.sh --citations language`: 0 misses).

Verification (CARGO_TARGET_DIR=/tmp/bs15-target):
- `cargo test --bin mfb inline_trap_short_circuit`: 11 passed.
- `cargo test --bin mfb -- spec inline_trap tostring`: 94 passed.
- `cargo test --test rt_inline_trap_raising_operator --test rt_inline_trap_nested_call`:
  20 + 21 passed.
- `scripts/test-accept.sh` on the two new fixtures plus
  `inline-trap-fixed-comparison-promotion-rt` and
  `trap-local-after-fixed-comparison-invalid`: 4 passed.
- `scripts/artifact-gate.sh <mfb> all`: 1606 tests, 2264 goldens, 0 diffs. No
  existing golden changed.
