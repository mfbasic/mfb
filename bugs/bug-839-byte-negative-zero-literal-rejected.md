# bug-839: `LET b AS Byte = -0` is `TYPE_BINDING_MISMATCH`, while the spec and the range checker treat `-0` as an in-range Byte literal

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (spec/code disagreement; owner chose (b): spec + dead-code fix)

Status: Open
Regression Test: tests/syntax/types/byte-negative-zero-literal (to add)

§4.12 defines the Byte literal check as: "any **nonzero** negated literal is
`TYPE_BYTE_LITERAL_UNDERFLOW`". The code mirrors this. In
`check_negated_const_literal`, the Byte arm is guarded with `value != "0"`, so `-0`
deliberately passes the range check. Even so, `LET x AS Byte = -0` is rejected one step
earlier:
`error[2-203-0007 TYPE_BINDING_MISMATCH]: … Binding `x` has initializer type Integer, expected Byte.`
Unary minus on an untyped literal produces `Integer` rather than coercing to the `Byte`
slot. The `-0` exemption in the verifier is therefore unreachable from source, and the
spec wording implies a program it rejects.

This is classed UNCLEAR. One of two things should hold:

- (a) `-0` in a Byte slot is accepted as `0`, as the spec's "nonzero" and the verifier's
  exemption imply.
- (b) The spec says that a negated literal is never a Byte literal, and the dead `-0`
  exemption is deleted.

**Owner decision (2026-10-05): (b).** A Byte is a data carrier (`0..=255`), not a
general number, so negation has no meaning for it and `-0` is not a Byte literal.
Rejecting `LET x AS Byte = -0` is correct. The fix: §4.12 says that **any** negated
literal in a Byte slot is rejected (no "nonzero" carve-out), and the unreachable
`value != "0"` exemption in `check_negated_const_literal` is deleted. Forbidden wrong
fix: making `-0` coerce to Byte `0`.

References:

- `src/docs/spec/language/04_types.md` §4.12 **Byte** paragraph.
- `src/ir/verify/values.rs:check_negated_const_literal` (`ParameterType::Byte if !value.contains('.') && value != "0"`).

## Failing Reproduction

```basic
IMPORT io
FUNC main() AS Nothing
  LET x AS Byte = -0
  io::print(toString(x))
END FUNC
```
- Observed: `error[2-203-0007 TYPE_BINDING_MISMATCH]: binding initializer type does not match declared type` / `Binding `x` has initializer type Integer, expected Byte.`
- Expected (spec as written): the program builds and prints `0`.

Contrast: `LET x AS Byte = -1` gives `TYPE_BYTE_LITERAL_UNDERFLOW` (the range error
suppresses the mismatch).

## Root Cause

Type inference for `Unary(-, literal)` with expected type `Byte` does not propagate the
Byte expectation into the negated literal, so the result is `Integer`. The verifier's
`-0` arm assumes it does.

## Open Decisions

- (a) accept `-0` vs (b) spec says negated literals never initialize a Byte, and the
  dead branch is deleted. Recommended: (b). It is simpler, and `-0` has no use.

## Summary

This is a corner-case disagreement. Either fix is small.
