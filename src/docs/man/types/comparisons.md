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
or `String`. The ordering operators require either two numeric operands or two
`String` operands; mixed `String`/numeric ordering is a compile-time type error.

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

## Comparable and orderable

Comparable types (`=`, `<>`) are `Integer`, `Float`, `Fixed`, `Boolean`,
`String`, `Byte`, `Nothing`, enums, the built-in `Error`/`ErrorLoc` records, and
records whose fields are all comparable.

Orderable types (`<`, `>`, `<=`, `>=`) are the narrower set `Integer`, `Float`,
`Fixed`, `Byte`, and `String`. `Boolean`, `Nothing`, enums, unions, and records
are comparable but not orderable. `collections::sort` and `collections::sortBy`
require an orderable element or key type.

## Errors

No errors.

## Examples

```
LET same AS Boolean = "Ada" = "Ada"
LET ordered AS Boolean = 1.5 < 2
LET textOrder AS Boolean = "Ada" < "Grace"
LET mixed AS Boolean = 1 = toByte(1)
```

## See also

- `mfb man types numeric`
- `mfb man types logical`
