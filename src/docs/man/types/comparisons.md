# comparisons

Comparison operators and operand rules

## Synopsis

```
=  <>  <  >  <=  >=
```

## Description

MFBASIC comparison operators return `Boolean`. The equality operators are `=`
(equal) and `<>` (not equal); the ordering operators are `<`, `>`, `<=`, and
`>=`.

`=` and `<>` accept any two numeric operands directly — any cross-numeric pairing
such as `Integer = Float` or `Byte <> Fixed` is accepted with no compatibility
requirement — or any two other compatible comparable operands such as `Boolean`
or `String`. `Money` is the one exception: because it is dimensioned, a
comparison involving a `Money` requires *both* operands to be `Money`, so
`amount = 5` is `TYPE_MONEY_OPERATION_INVALID` and `amount = toMoney(5)` is the
way to write it. The ordering operators require two numeric operands, two
`String` operands, or two `Scalar` operands; mixed `String`/numeric (or
`Scalar`/anything else) ordering is a compile-time type error.

Comparisons do not chain specially. `a < b < c` parses left-associatively as
`(a < b) < c`, and because `a < b` is `Boolean` and `Boolean` is not orderable,
that is normally a type error (`TYPE_BINARY_OPERATOR_MISMATCH`) — there is no
Python-style chained comparison.

Two `String` operands are ordered lexicographically by Unicode scalar value: the
strings are compared scalar by scalar, the first differing position decides, and
a string that is a prefix of the other compares less. This order is deterministic
and identical on every target — it does not depend on host locale, collation, or
libc. It is not a human collation and is not grapheme-cluster aware; call
`strings::caseFold` or normalize first when case-insensitive or locale-aware
ordering is needed.

`Float` comparisons follow IEEE 754: any ordered comparison involving `NaN` is
false, `NaN = NaN` is false (and `NaN <> NaN` is true), and `+0.0 = -0.0` is
true. (Map-key equality is a separate bitwise comparison and does not follow this
rule.)

Numeric comparisons use converted values. For example, Integer
`9007199254740993` compares equal to Float `9007199254740992.0`: converting the
Integer to Float loses one unit. The same two values stored as Integers remain
unequal.

Currently a Float/Fixed comparison uses Float, unlike arithmetic's Fixed-first
rule. For example, `1F = 1.00000000001` is false, while comparing `1F` to
`toFixed(1.00000000001)` is true. Explicitly convert both operands when that
representation choice matters.

| Context | Float matching | Positive and negative zero |
| --- | --- | --- |
| `=` and `<>` | numerical value | equal |
| Map lookup | exact bit pattern | distinct |
| Set deduplication and membership | exact bit pattern | distinct |
| List `collections::contains` | exact bit pattern | distinct |

Thus a Set can hold both signed zeros, and a List containing only positive
zero does not contain negative zero according to `collections::contains`.
Collection lookup requires the collection's key or element type; it does not
perform the cross-numeric conversions used by comparison operators.

## Comparable and orderable

Comparable types (`=`, `<>`) are `Integer`, `Float`, `Fixed`, `Money`,
`Boolean`, `String`, `Byte`, `Scalar`, `Nothing`, enums, the built-in
`Error`/`ErrorLoc` records, and records whose fields are all comparable.

Orderable types (`<`, `>`, `<=`, `>=`) are the narrower set `Integer`, `Float`,
`Fixed`, `Money`, `Byte`, `String`, and `Scalar`. `Boolean`, `Nothing`, enums,
unions, and records are comparable but not orderable. A `Money` compares and
orders only against another `Money`, and a `Scalar` only against another `Scalar`
(by code-point value) — a `Scalar` is non-numeric and never orders against a
`String` or a numeric type. `collections::sort` and `collections::sortBy` require
an orderable element or key type.

## Errors

An Integer/Fixed comparison converts the Integer to Fixed first and can raise
`ErrOverflow` if it is outside the Fixed range. Errors while computing either
operand also propagate. Float comparisons themselves do not reject transient
NaN or infinity; the observation rules are in `mfb man types numeric`.

## Examples

```
FUNC main() AS Nothing
  LET same AS Boolean = "Ada" = "Ada"
  LET ordered AS Boolean = 1.5 < 2
  LET textOrder AS Boolean = "Ada" < "Grace"
  LET mixed AS Boolean = 1 = toByte(1)
END FUNC
```

## See also

- `mfb man types numeric`
- `mfb man types logical`
