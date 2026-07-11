# bug-66: LINK `SUCCESS_ON`/`RESULT` `AND`/`OR` are emitted as bitwise ops on non-normalized operands (wrong result), and a `CInt32` const-pin bypasses the signed-32-bit range validation that params get (silent truncation)

Last updated: 2026-07-09
Effort: small (<1h)

Two LOW-severity logic defects in the LINK thunk emitter, batched (same file; the HIGH
register-escalation defect in the same `emit_link_expr` is filed separately as bug-56).

**(1) `AND`/`OR` are bitwise, not logical, on unnormalized operands.** `IrLinkExpr::And`/`Or`
lower to `and_registers`/`or_registers` on the operand *values*; only `Compare` and `Not`
produce a canonical `0`/`1`. So `SUCCESS_ON r AND (status = 0)` with native return `r = 2`
computes `2 & 1 = 0` and the final `compare 0; branch_eq call_fail` treats it as failure —
even though `r` is truthy and `status == 0`. The logical intent ("`r` nonzero AND
`status == 0`") is violated whenever a bare `Var`/`Int` leaf is an `AND`/`OR` operand.

**(2) `CInt32` const-pin skips the range check that params get.** A LINK ABI slot with
ctype `CInt32` fed by a `CONST` pin is emitted with `move_immediate(value)` and stored
full-width, with no range validation — the C callee reads only the low 32 bits, silently
truncating an out-of-range pinned value. A *param* feeding the same slot is range-checked
(`sxtw` compare → `range_fail`, raising `ErrOverflow`). The const-pin path is taken before
the `CInt32` range-check path and performs no width validation.

The single correct behavior a fix produces: (1) `AND`/`OR` combine logical truth values,
so any nonzero operand is "true"; (2) an out-of-range `CInt32` const pin is rejected, not
silently truncated.

References (all under `src/target/shared/code/link_thunk.rs`):

- `emit_link_expr` `And`/`Or` arms (`:974-983`): `and_registers`/`or_registers` on
  unnormalized operands. Only `Compare` (`:951-975`) and `Not` (`:938-951`) yield `0`/`1`.
- Const-pin branch (`:420-424`) vs the param `CInt32` range-check branch (`:436-446`):
  the const path is taken first and does no validation.
- Same function, separate HIGH bug: bug-56 (physical-register escalation into `x19`).
- KNOWN (not re-filed): bug-34 (LINK CONST bit-63-set 64-bit value lowered to 0).
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

(Both need a LINK binding; construct per the LINK test harness.)

- (1) `SUCCESS_ON r AND (status = 0)` where the native call returns `r = 2`, `status = 0`.
  Observed: treated as failure (`2 & 1 = 0`). Expected: success (`r` truthy AND
  `status == 0`).
- (2) A `CInt32` ABI slot pinned by `CONST 5000000000` (out of i32 range). Observed:
  silently truncated to the low 32 bits and passed to the C callee. Expected: rejected at
  thunk-emit / typecheck, as a param feeding the same slot would raise `ErrOverflow`.

Contrast: (1) `AND`/`OR` of two `Compare`s (the common case) works because comparisons
produce `0`/`1`, so bitwise coincides with logical. (2) Param-sourced `CInt32` slots are
range-checked and fail loudly; only const-pinned ones skip it.

## Root Cause

(1) `And`/`Or` lower to bitwise ops on operand values without normalizing bare `Var`/`Int`
leaves to `0`/`1`. (2) The const-pin emit path precedes and bypasses the `CInt32`
range-check applied to param-sourced slots.

## Goal

- LINK `AND`/`OR` treat any nonzero operand as true (logical semantics).
- An out-of-range `CInt32` const pin is rejected rather than truncated.

### Non-goals (must NOT change)

- `AND`/`OR` of comparison operands (already correct — must stay byte-identical where
  possible).
- Param-sourced `CInt32` validation (already correct).
- The bug-56 register-scheme rewrite (separate; coordinate if both land together).

## Blast Radius

- `emit_link_expr` `And`/`Or` — item (1).
- The `CInt32` const-pin branch — item (2). Consider validating const-vs-ctype range in IR
  / typecheck (earlier, uniform for all ctypes) rather than only at thunk emit.

## Fix Design

(1) Normalize each `AND`/`OR` operand to `0`/`1` (compare-nonzero) before combining, or
evaluate `AND`/`OR` with short-circuit compare-and-branch like `Not`/`Compare`. (2) Validate
`CONST` pin values against the slot ctype range at thunk-emit time (or in IR/typecheck),
rejecting an out-of-range `CInt32` const rather than truncating.

## Phases

### Phase 1 — failing tests

- [x] LINK test: `SUCCESS_ON status AND (status <> 0)` with `status = 2` → success
      (fails today: `native binding call failed`, exit 255).
- [x] LINK test: out-of-range `CInt32` const pin → rejected (silently truncates today).

### Phase 2 — the fixes

- [x] Normalize `AND`/`OR` operands; validate `CInt32` const pins.

### Phase 3 — validation

- [x] Regenerate goldens (comparison-only `AND`/`OR` byte-identical); `scripts/artifact-gate.sh`,
      `scripts/test-accept.sh` (run by the orchestrator).

## Validation Plan

- Regression test(s): the two LINK tests above.
- Runtime proof: the `AND` success-expression evaluates true; the const pin is rejected.
- Doc sync: none expected.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

LINK `SUCCESS_ON`/`RESULT` `AND`/`OR` use bitwise ops on unnormalized operands (wrong for
bare-value leaves), and a `CInt32` const pin skips the range check params get (silent
truncation). Both LOW and local; comparison-operand logic and param validation are
unchanged. The register-escalation defect in the same emitter is bug-56.

## Resolution

Both items fixed in `src/target/shared/code/link_thunk.rs`, building on the bug-56 vreg
form of `emit_link_expr`.

1. **`AND`/`OR` normalization.** Added `emit_link_bool` + `link_expr_is_boolean`. Each
   `AND`/`OR` operand now flows through `emit_link_bool`, which emits the operand and — only
   when the operand is a bare `Var`/`Int` leaf (an arbitrary integer) — normalizes it to a
   canonical `0`/`1` with a compare-nonzero before the `and`/`or`. Comparison and logical
   sub-expressions already yield `0`/`1`, so `link_expr_is_boolean` returns `true` for them
   and they pass through unchanged — `AND`/`OR`-of-comparisons stays byte-identical (verified:
   nested-success / sqlite / free / const-64bit runtime output unchanged). With both operands
   `0`/`1`, bitwise coincides with logical, so any nonzero operand is true.

2. **`CInt32` const-pin range check.** The const-pin branch now rejects a `CONST` pin whose
   value falls outside signed 32-bit range when the slot ctype is `CInt32`, returning a build
   error (`error: LINK function '…' CONST pin '… = …' does not fit the signed 32-bit range of
   its CInt32 ABI slot`). Because the value is a compile-time constant this is caught at
   thunk-emit rather than deferred to the runtime `range_fail`/`ErrOverflow` a param takes.
   It is a plain codegen-time error string (same mechanism as the existing "no source" LINK
   error), not a numbered diagnostic rule, so no Constant Registry change was required.
   In-range pins (e.g. `CONST nByte = -1`) and `CInt64` pins (e.g. `0xFFFFFFFFFFFFFFFF`) are
   unaffected.

### Runtime proof

- Item 1: `tests/rt-behavior/native/native-link-and-truthy-rt` — `sqlite3_column_count`
  of `"SELECT 1, 2"` is `2`; `SUCCESS_ON status AND (status <> 0)`. Before: `2 & 1 = 0`
  → `native binding call failed` (exit 255). After: `count=2` (exit 0). (Its `RESULT` is a
  bare `Var`, which emits no labels, keeping the pre-existing SUCCESS_ON/RESULT `counter`
  label-namespace overlap out of the picture so the `AND` semantics are the sole variable.)
- Item 2: `tests/syntax/native/native-link-const-cint32-overflow-invalid` — `CONST ms =
  5000000000` into a `CInt32` slot is rejected at build (exit 1) instead of truncating.

### Note (out of scope, not fixed)

Discovered while testing item 1: `lower_link_thunk` re-initializes the `emit_link_expr`
label `counter` to `0` for both the `SUCCESS_ON` and the `RESULT` expression, so a thunk with
comparisons/`NOT`s in *both* emits duplicate labels (e.g. two `…_cmp0_end`). This is a
distinct pre-existing defect (label-uniqueness, not bug-66) and was left untouched; the item-1
test is shaped to avoid it.
