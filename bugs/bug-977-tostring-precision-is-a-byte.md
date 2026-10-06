# bug-977: `toString`'s `precision` is a `Byte` — a number typed as data; make it an `Integer` with a checked 0..32 range

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Correctness (built-in surface; owner-confirmed)

Status: Open
Regression Test: tests/rt-behavior/general/tostring-integer-precision-rt; tests/syntax/general/tostring-precision-literal-out-of-range-invalid; tests/rt-error/general/tostring-precision-out-of-range

**Owner decision (2026-10-05):** a `Byte` is meant for data, not numbers. `toString`'s
precision is a count, so it becomes `precision AS Integer`. Valid values are **0..32**.
Anything else is out of range. This supersedes bug-938 item 22c, which had decided to let
built-in overload selection coerce `3` to `Byte`.

## Today (measured, HEAD 846aa55fd, `/tmp/hardening/mfb-165`, `/tmp/hardening/coord/tp`)

- Signature: `general::toString(value AS Scalar, [precision AS Byte]) AS String`.
  `mfb man general toString` tells users "precision is a Byte, so an Integer literal needs
  toByte around it", and every example is written `toString(3.14159, toByte(2))`.
- `toString(x, 3)` is `TYPE_CALL_ARGUMENT_MISMATCH` (bug-938 item 22c).
- Every Byte value 0..255 renders for Float, Fixed and Money. `toString(1.5, toByte(255))`
  is 257 characters (`F255 len=257`). `toString(1.5m, toByte(255))` is too, though Money holds
  only 5 fractional digits. `toString` is the only built-in that takes a numeric `Byte`
  parameter (`grep -rn 'AS Byte' src/codegen/builtins`).

## Goal

- Every precision-taking `toString` overload (`Float`, `Fixed`, `Money`, and any other with a
  `[precision]` slot) takes `precision AS Integer`. `toString(x, 3)` compiles.
- A **constant** precision outside 0..32 (`toString(x, -1)`, `toString(x, 33)`) is a located
  compile-time error (a new or existing range rule, registered in the diagnostics registry).
- A **non-constant** precision outside 0..32 raises `ErrInvalidArgument` (77050002) at run
  time, with the call site as `e.source`. `toString` already declares fallibility (it
  allocates), so no new fallibility census change is needed. Confirm with the dead-handler
  rules.
- `toString(x, 32)` renders 32 places for Float/Fixed/Money (Money pads with zeros past its
  5 stored digits, as today). The bug-878 exact-rounding rule applies at every precision.
- `mfb man general toString`, `mfb spec language builtin-functions` (§18.1), and every
  example/fixture that writes `toByte(n)` for a precision are updated to plain `n`.

### Non-goals (must NOT change)

- Rendering below 33 places (except as bug-878 changes rounding).
- Forbidden wrong fix: accepting both `Byte` and `Integer` precision overloads.

## Blast Radius

- `src/codegen/builtins/general/func_to_string.rs`, `src/codegen/builtins/general/mod.rs`
  (registry signatures and lowering of the precision operand).
- Every `toString(…, toByte(…))` in tests/, examples/, packages/, man examples and spec.
- bug-878 (Fixed `toString` rounding) touches the same lowering. Land in either order, but
  test both together.
