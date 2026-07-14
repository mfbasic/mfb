# string

The `String` and `Scalar` text primitives

## Synopsis

```
String   Scalar
```

## Description

MFBASIC has two text primitives. `String` is an immutable UTF-8 sequence;
`Scalar` is one Unicode scalar value. A `String` is, conceptually, a sequence of
`Scalar`s, and the `strings::` package bridges the two.

- **`String`** — an immutable, UTF-8-encoded sequence of Unicode scalars.
  Strings are value types: assignment and passing copy the value, and no
  operation mutates a string in place. Written with double quotes (`"…"`).
- **`Scalar`** — a single 32-bit Unicode scalar value: a code point in
  `U+0000..U+D7FF` or `U+E000..U+10FFFF` (surrogates excluded). It is
  register-carried like `Byte`, never heap-allocated. Written with a backtick
  literal (`` `x` ``). The name is `Scalar`, not `Char`, because it is exactly
  one Unicode scalar — not a grapheme cluster (a user-perceived character such as
  `é` or a flag emoji may span several scalars; use `strings::graphemes` for
  those).

`Scalar` is **comparable** and **orderable** by code-point value but **not
numeric**: it does not participate in numeric promotion, and the arithmetic
operators reject it. Code-point math goes through the conversions below.

## Literals

A string literal is delimited by `"` and may not span a line. Inside it, `\`
introduces an escape: `\"`, `\\`, `\n`, `\t`, `\r`, `\0`, and `\u{HEX}` (1–6 hex
digits naming a Unicode scalar). See `mfb spec language lexical-structure`.

A scalar literal is delimited by backticks and holds exactly one scalar — a raw
scalar or an escape reusing the string machinery: `` `A` ``, `` `中` ``,
`` `\n` ``, `` `\\` ``, the backtick escape `` `\`` ``, and `` `\u{1F600}` ``.
An empty (`` `` ``) or multi-scalar (`` `ab` ``) literal, an unterminated
literal, or an invalid escape is a compile-time `TYPE_SCALAR_LITERAL_*` error.
The backtick is otherwise unused, so `'` line comments are unaffected and
`` `'` `` is the apostrophe scalar.

```
LET s = "héllo中"       ' String
LET c = `中`             ' Scalar
MUT d AS Scalar          ' defaults to `\u{0}` (U+0000)
```

## Comparison and ordering

Two `String`s compare with `=`/`<>` and order with `<`, `>`, `<=`, `>=`
lexicographically by Unicode scalar value — deterministic across targets, not
locale collation, and not grapheme-aware. Two `Scalar`s compare and order by
code-point value. `Scalar` is non-numeric and never orders against a `String` or
a numeric type; a mixed comparison is a compile-time type error.

## Conversions and the scalar seam

- `toInt(Scalar)` — the code point, as `Integer` (infallible).
- `toScalar(Integer)` — the scalar for a code point; fails `ErrInvalidArgument`
  for a surrogate or a value outside `0..U+10FFFF`.
- `toScalar(Byte)` — widen a byte to a scalar (infallible).
- `toByte(Scalar)` — narrow a code point to a `Byte`; fails past 255.
- `toString(Scalar)` — the one-scalar UTF-8 `String` (infallible).
- `toScalar(String)` — the single scalar of a one-scalar string.
- `strings::toScalars(String)` / `strings::fromScalars(List OF Scalar)` — walk a
  string into scalars and rebuild it (an exact round trip).
- `strings::isLetter`, `isDigit`, `isWhitespace`, `isUpper`, `isLower` — classify
  a `Scalar` by its Unicode general category.

## Defaults and storage

`String` defaults to `""`; `Scalar` defaults to `` `\u{0}` `` (U+0000). Both are
defaultable, so a `List OF Scalar` and a `Map ... TO Scalar` are defaultable. A
`String` is heap-allocated (a length-prefixed UTF-8 buffer); a `Scalar` is a
4-byte register value (see `mfb spec memory scalar-storage`). A `Scalar`
collection element occupies a 4-byte, 4-aligned inline payload.

## See also

- `mfb spec language types`
- `mfb spec language lexical-structure`
- `mfb man strings toScalars`
- `mfb man builtins general toScalar`
