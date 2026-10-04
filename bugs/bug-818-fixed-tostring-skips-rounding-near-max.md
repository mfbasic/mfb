# bug-818: `toString(Fixed)` truncates instead of rounding near the top of the range

Last updated: 2026-10-04
Effort: small (fix); the native goldens of every Fixed `toString` move
Severity: LOW
Class: Correctness

Status: Open
Regression Test: proposed — a runtime fixture over values next to the maximum
Fixed, at 2 decimal places and at 1.

Found while fixing bug-813 (bug-staging-9).

## Failing Reproduction

- `toString(2147483647.996F)` prints `2147483647.99`; correct is `2147483648.00`
  (half-away rounding at 2 places).
- At 1 decimal place, `2147483647.996F` prints `…7.9`; correct is the value
  rounded up.

## Root Cause

`builder_strings.rs:emit_fixed_to_string_value` deliberately skips its
half-away rounding step near the top of the range. Its comment calls truncating
"the safe answer". The rounding cannot overflow: 2^63 plus the rounding increment
fits in an unsigned 64-bit value. So the guard is unnecessary, and it gives wrong
digits.

## Goal

`toString` of a Fixed rounds half away from zero at every magnitude, including
next to the maximum and the minimum. No overflow, and no change for values the
current path already rounds.

## Blast Radius

The native code of every Fixed `toString`: `.ncode`/`.ncodesum` goldens move in
the batch gate.

## Phases

- [ ] RED fixture: values next to ±max at 2 places and at 1.
- [ ] Remove the truncation guard; round on unsigned magnitudes.
- [ ] Verify on macOS plus one non-macOS box; goldens in the gate.
