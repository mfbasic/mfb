# bug-61: `Fixed` division of the exact minimum value wrongly traps `ErrOverflow`, and the integer/`Fixed` `^` operator hangs for a bounded base with a huge exponent

Last updated: 2026-07-09
Effort: small (<1h)

Two numeric-codegen defects in `builder_numeric.rs` (and `builder_fixed_math.rs`),
batched (same subsystem, distinct root causes).

**(1) `Fixed` divide of the representable minimum wrongly traps (LOW correctness).**
`emit_fixed_divide` range-checks the *magnitude* of the integer quotient part with
`compare_registers(integer, 2147483647)` + unsigned `branch_hi`. For a result whose real
value is exactly `-2147483648.0` (the representable minimum `Fixed`, raw `i64::MIN`), the
magnitude integer part is `2^31`, which is `> 2^31 - 1`, so it traps `ErrOverflow` — even
though the *signed* value `-2^31` is in range (only `+2^31` is not). Runtime-confirmed:
`toFixed(-2147483648) / toFixed(1)` fails with `77050010 ErrOverflow`, while the sibling
`toFixed(-2147483648) * toFixed(1)` correctly returns `-2147483648.00`.

**(2) `^` linear loop hangs for `|base| <= 1` and a huge exponent (LOW footgun).**
`emit_integer_pow`/`emit_fixed_pow` (and `builder_fixed_math.rs:emit_fixed_pow_general`)
implement exponentiation as a linear countdown loop, iterating once per unit of the
exponent. For a base whose powers are bounded (`|base| <= 1`), the running product never
overflows, so the only early exit — the overflow trap inside the checked multiply — never
fires, and the loop runs the full exponent count (up to `i64::MAX` iterations): an
effective non-terminating hang / CPU DoS. E.g. `1 ^ 9223372036854775807`, `0 ^ big`,
`-1 ^ big`, or `Fixed 1.0 ^ 1e9`.

The single correct behavior a fix produces: (1) a division whose exact result is
`-2147483648.0` returns it, not `ErrOverflow`; (2) `^` with a bounded base terminates
immediately with the correct value.

References:

- `src/target/shared/code/builder_numeric.rs:emit_fixed_divide` (`:1335-1340`, magnitude
  range check). Contrast: `emit_fixed_multiply` (`:1288-1300`) checks the **signed** high
  word against `[-2^31, 2^31-1]` and correctly admits `-2^31`.
- `src/target/shared/code/builder_numeric.rs:emit_integer_pow` (`:1242-1270`),
  `emit_fixed_pow` (`:1382-1415`); `builder_fixed_math.rs:emit_fixed_pow_general` integer
  loop (`:775-784`).
- KNOWN (not re-filed): bug-07 (Fixed min literal), bug-11 (Fixed literal exponent).
- Both items runtime-confirmed on macOS/aarch64 during the goal-01 review of
  `src/target/shared/code/`.

## Failing Reproduction

(1)
```
IMPORT io
FUNC main AS Integer
  io::print(toString(toFixed(-2147483648) / toFixed(1)))
  RETURN 0
END FUNC
```
- Observed: exits 255, "numeric overflow" (`77050010`).
- Expected: `-2147483648.00` (the `*` form already returns this).

(2)
```
IMPORT io
FUNC main AS Integer
  io::print(toString(1 ^ 9223372036854775807))
  RETURN 0
END FUNC
```
- Observed: hangs (loops ~9.2e18 times).
- Expected: `1`, immediately.

Contrast: (1) `emit_fixed_multiply` admits `-2^31` correctly; other Fixed divisions are
fine. (2) `|base| >= 2` overflows the checked multiply within ~63 iterations and traps
quickly, so only `base ∈ {-1, 0, 1}` (integer) / `|base| <= 1.0` (Fixed) hang.

## Root Cause

(1) `emit_fixed_divide`'s magnitude-only check cannot distinguish the representable `-2^31`
from the unrepresentable `+2^31`; `emit_fixed_multiply` avoids this by testing the signed
high word. (2) The pow loop's only termination for a non-overflowing product is the
overflow trap, which cannot fire when the product stays bounded.

## Goal

- `emit_fixed_divide` admits an exact `-2147483648.0` result (matching `emit_fixed_multiply`).
- `^` with `|base| <= 1` returns the correct value without iterating the exponent.

### Non-goals (must NOT change)

- Genuinely-overflowing Fixed divisions (still trap).
- `^` with `|base| >= 2` (already terminates via the overflow trap) — its results and
  overflow behavior must be unchanged.
- Fixed `*` rounding (truncation-toward-−∞ is spec-permitted "deterministic").

## Blast Radius

- `emit_fixed_divide` (`builder_numeric.rs`) — item (1).
- `emit_integer_pow`, `emit_fixed_pow`, `emit_fixed_pow_general` — item (2).
- No other consumer.

## Fix Design

(1) Allow the boundary in `emit_fixed_divide`: accept when `sign` is negative AND
`integer == 2^31` AND `remainder == 0` (exactly `-2^31`), or restructure to test the final
signed `dst` against the `i64` bounds the way the positive-result check (`:1370-1374`)
already does — mirroring `emit_fixed_multiply`'s signed high-word test.
(2) Special-case `|base| <= 1` before the loop (result is trivially `1` / `0` / `±1` by
base and exponent parity), or switch to exponentiation-by-squaring so the iteration count
is `O(log exponent)` for all bases.

## Phases

### Phase 1 — failing tests

- [x] `toFixed(-2147483648) / toFixed(1)` returns `-2147483648.00` (fails today).
- [x] `1 ^ big`, `0 ^ big`, `-1 ^ big`, `Fixed 1.0 ^ 1e9` return immediately (hang today).

### Phase 2 — the fixes

- [x] Fix the `emit_fixed_divide` boundary check; special-case bounded-base `^` (or use
      exponentiation-by-squaring).

### Phase 3 — validation

- [x] Runtime-proved both items (values + timeouts); `tests/native_numeric_pow_div_runtime.rs`
      (6 cases) passes. Golden regen + `scripts/artifact-gate.sh`/`scripts/test-accept.sh`
      left to the orchestrator (native codegen for `^`, `/`, `math::pow(Fixed)` shifts).

## Validation Plan

- Regression test(s): the min-Fixed division test and the bounded-base `^` tests.
- Runtime proof: build and run both reproductions — correct value, no hang.
- Doc sync: none expected.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

`Fixed` division rejects its own representable minimum because it range-checks a magnitude
instead of the signed value, and `^` iterates the exponent linearly so a bounded base with
a huge exponent hangs. Fix (1) mirrors `emit_fixed_multiply`'s signed check; fix (2)
special-cases bounded bases or uses squaring. Both are runtime-confirmed and local.

## Resolution

Fixed 2026-07-09 in `src/target/shared/code/builder_numeric.rs` and
`src/target/shared/code/builder_fixed_math.rs`.

**(1) `emit_fixed_divide` (`builder_numeric.rs`).** The early magnitude guard now admits an
integer part up to `2^31` (was `2^31 - 1`); a strictly-greater magnitude still traps before
the `integer << 32` (which would otherwise lose bits). The final sign step was rewritten to
range-check the signed value like `emit_fixed_multiply`: a positive result must have its top
bit clear; a negative result is `-(magnitude)`, admissible iff `magnitude <= 2^63` — i.e.
`dst >= 0` (negate) or `dst == i64::MIN` (already the exact `-2147483648.0`, kept as-is),
else `ErrOverflow`. Boundary matrix verified at runtime: `-2^31/1 = -2147483648.00`,
`-2^31/2 = -1073741824.00`, `-2^31/-2 = 1073741824.00`, `-2147483647/1` and `2147483647/1`
correct; genuine overflows still trap (`-2^31/-1` = +2^31, and a 2e10-magnitude quotient).

**(2) bounded-base `^` (three sites).**
- `emit_integer_pow`: closed-form fast path for base `∈ {-1, 0, 1}` (parity for `-1`,
  `0^0 == 1` / `0^n == 0`), else the unchanged overflow-terminated loop. `1 ^ i64::MAX`,
  `0 ^ i64::MAX`, `-1 ^ big` now return instantly; `|base| >= 2` unchanged.
- `emit_fixed_pow` (the `Fixed ^ Fixed` operator) and `emit_fixed_pow_general`
  (`math::pow(Fixed, Fixed)`): closed-form `±1.0` fast path (parity), plus a
  `product == 0` loop early-exit (a truncated-to-zero product stays zero, so this never
  changes a result but stops `|base| < 1.0` / `0.0` from iterating an enormous exponent).
  `math::pow(Fixed)` small-exponent outputs are byte-identical to HEAD.

**Preserved / out of scope (pre-existing, unrelated bugs found while validating — left
unchanged to honor the non-goals):**
- Integer `^` with `|base| >= 2` and a *negative* base (e.g. `(0-2) ^ 3`) traps `ErrOverflow`
  on HEAD too — a separate defect in the operator path, not this loop.
- The `Fixed ^ Fixed` *operator* (`emit_fixed_pow`) returns `0.00` for any `|base| >= 2`
  (and garbage for fractional bases) on HEAD — a register-lifetime clobber of `base` across
  `emit_fixed_multiply` in its loop (the stack-slot `math::pow` path is correct). Bug-61's
  non-goal requires `|base| >= 2` operator results to be *unchanged*, so this is left as-is.

Regression test: `tests/native_numeric_pow_div_runtime.rs` (6 cases, all green; the `^`
cases run under a 30s bound that fails if the linear loop is reintroduced).
