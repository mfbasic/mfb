# toScalars

Decode a string into its Unicode scalar values.

## Synopsis

```
strings::toScalars(value AS String) AS List OF Scalar
strings::toScalars(value AS AttributedString) AS List OF Scalar
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. `toScalars`
is one of seven `strings` members implemented in MFBASIC source rather than in
native codegen; the companion is injected automatically when a program imports
`strings` and references the scalar seam. [[src/codegen/registry/mod.rs:rewrite_target]] [[src/codegen/registry/mod.rs:references_any]]

## Description

`strings::toScalars` decodes `value` into its Unicode scalar values and returns
them, in order, as a `List OF Scalar`. It walks the UTF-8 once, yielding one
element per code point. [[src/codegen/builtins/strings/seam.mfb:__strings_toScalars]]

Each element is one `Scalar` — a 32-bit Unicode scalar value — not a grapheme
cluster. A base letter followed by a combining mark is two separate elements,
while an astral character such as an emoji is a single element. The element count
therefore equals `len(value)` and is generally smaller than
`strings::byteLen(value)`. Use `strings::graphemes` when user-perceived
characters are what matters.

This is the entry point for walking a string one scalar at a time: compare each
`Scalar`, `MATCH` on it, or classify it with `strings::isLetter` and its
siblings, then rebuild a `String` with `strings::fromScalars`. The round trip is
exact — `fromScalars(toScalars(s))` equals `s` for every `String s` — because
every `String` is well-formed UTF-8 by construction, so decoding cannot fail.
[[src/codegen/builtins/strings/seam.mfb:__strings_fromScalars]]

The scalars appear in the same left-to-right order as in `value`. The empty
string yields the empty list. `value` is not mutated; the returned list is a
fresh owned value.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors). [[src/codegen/builtins/strings/mod.rs:is_tier_a_query]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to decode. Any `String` is accepted, including the empty string. [[src/codegen/registry/mod.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Scalar` | The Unicode scalar values of `value`, in order, one per element. The empty string yields the empty list. [[src/codegen/builtins/strings/mod.rs:register]] |

## Errors

No errors.

## Examples

Count the scalars in a string with an astral character:

```
IMPORT io
IMPORT strings
IMPORT collections

FUNC main() AS Integer
  LET scalars AS List OF Scalar = strings::toScalars("a中😀")
  io::print(toString(len(scalars)))
  io::print(toString(collections::get(scalars, 0) = `a`))
  RETURN 0
END FUNC
```

Keep only the letters and digits, then rebuild the string:

```
IMPORT io
IMPORT strings
IMPORT collections

FUNC main() AS Integer
  MUT kept AS List OF Scalar = []
  FOR EACH sc IN strings::toScalars("a1 b2! c3")
    IF strings::isLetter(sc) OR strings::isDigit(sc) THEN
      kept = collections::append(kept, sc)
    END IF
  NEXT
  io::print(strings::fromScalars(kept))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings fromScalars`
- `mfb man strings isLetter`
- `mfb man strings isDigit`
- `mfb man strings graphemes`
- `mfb man strings toBytes`
- `mfb man strings`
