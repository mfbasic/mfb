# bug-887: `AttributedString & String` fails with an unlocated diagnostic naming the internal helper `#astrings_concat`; `String & AttributedString` gives a different, generic error

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (diagnostic quality)

Status: Open
Regression Test: tests/compile-error/astrings/concat-mixed-string (to add)

The spec says `AttributedString & AttributedString` concatenates and "There is no
mixing with `String`". The mix is rejected, but the diagnostic depends on operand
order, and the left-attributed form is broken:

- `a & "x"` (and `a & 5`) prints `error: TYPE_CALL_ARGUMENT_MISMATCH: Argument 2 for
  `#astrings_concat` has type String, expected AttributedString.` It has **no file or
  line**, and it names a compiler-internal helper the user never wrote.
- `"x" & a` prints a located but generic `TYPE_UNKNOWN_VALUE … Initializer for binding
  `v` does not have a known type.`

Correct behavior: both orders give the same located diagnostic at the `&`, naming the
operand types. For example: "`&` cannot mix `AttributedString` and `String`; wrap the
text with `astrings::fromString`".

References:

- src/docs/spec/stdlib/15_astrings.md §"Attribute-aware strings:: overloads": "`AttributedString & AttributedString` concatenates … There is no mixing with `String`."

## Failing Reproduction

```
IMPORT astrings
IMPORT io
FUNC main AS Integer
  LET a AS AttributedString = astrings::fromString("ab")
  LET v = a & "x"
  io::print(toString(v))
  RETURN 0
END FUNC
```

`cd /tmp/hardening/B06/as3 && /tmp/hardening/mfb build`

- Observed: `error: TYPE_CALL_ARGUMENT_MISMATCH: Argument 2 for `#astrings_concat` has type String, expected AttributedString.` (no `./src/main.mfb:5` prefix, no source excerpt)
- Expected: `./src/main.mfb:5 error[...]` at the `&`, naming both operand types.

Contrast: `"x" & a` → `./src/main.mfb:5 error[2-203-0043 TYPE_UNKNOWN_VALUE]`; `a & a` → builds, prints `abab`.

## Root Cause

`src/ir/lower.rs` `expression_type` (the `BinaryOp::Concat` arm) returns
`AttributedString` whenever the **left** operand is an `AttributedString`, and does not
look at the right operand. The value lowering of `HirExpression::Binary` then routes
the expression to `IrValue::Call { target: "__astrings_concat", … }`. The call
verifier later rejects argument 2 of that synthetic call. The synthetic call reports
under the internal name, and in this path the diagnostic loses the `loc`. With
`String` on the left, the same arm returns `String`, and the binary verifier
(`src/ir/verify/values.rs`, `Concat => string(&lt) && string(&rt)`) fails differently.

## Goal

- Mixed `AttributedString`/non-`AttributedString` `&` is rejected in the front end with one located diagnostic in both operand orders.

### Non-goals (must NOT change)

- `AttributedString & AttributedString` concatenation semantics.

## Blast Radius

- Only the `&` operator with an `AttributedString` operand. The self-update arm (`src/codegen/collection/assign/self_update.rs`, `#astrings_concat`), which handles `s = s & t`, needs the same check (not run).
