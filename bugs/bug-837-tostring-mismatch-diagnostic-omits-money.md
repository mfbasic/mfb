# bug-837: `toString` argument-mismatch diagnostic lists accepted types without `Money`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (diagnostic text)

Status: Open
Regression Test: the goldens that print this message (grep `Fixed[, Byte], Boolean` under tests/)

When `toString` is called with an unsupported argument, `TYPE_CALL_ARGUMENT_MISMATCH`
says `expected Integer, Float[, Byte], Fixed[, Byte], Boolean, String, Byte, Scalar, List OF Byte, or an enum.`
But `toString(Money)` and `toString(Money, precision)` are accepted. This session's
programs printed `toString(5.00m)` and `toString(x, toByte(5))` for Money values. A user
reading the error is told Money is not convertible.

The single correct behavior: the list includes `Money[, Byte]`.

References:

- `src/docs/spec/language/04_types.md` §4.1 (`Money` is a primitive; every crossing is an
  explicit `to*(money)` call).
- `mfb man general toString`.

## Failing Reproduction

```basic
IMPORT io
FUNC main() AS Nothing
  LET x AS Byte = 256
  io::print(toString(x, toByte(12)))
END FUNC
```
(Any toString mismatch works. Here `(Byte, Byte)` is not an overload.)

- Observed: `Call to `toString` has argument type(s) (Byte, Byte), expected Integer, Float[, Byte], Fixed[, Byte], Boolean, String, Byte, Scalar, List OF Byte, or an enum.`
- Expected: `… Fixed[, Byte], Money[, Byte], Boolean, …`

## Root Cause

The hard-coded string in `src/codegen/builtins/general/mod.rs` (`expected_arguments`,
the `TO_STRING` arm):
`"Integer, Float[, Byte], Fixed[, Byte], Boolean, String, Byte, Scalar, List OF Byte, or an enum"`.

## Goal

- Add `Money[, Byte]`. Better: derive the list from the registry overloads so it cannot
  drift.

## Validation Plan

- Re-baseline only the lines carrying this message, after the full suite (AGENTS.md
  golden rule).

## Summary

This is a text-only fix.
