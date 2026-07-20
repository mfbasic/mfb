# fromScalars

Build a string from a list of Unicode scalar values.

## Synopsis

```
strings::fromScalars(scalars AS List OF Scalar) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required.
`fromScalars` is one of seven `strings` members implemented in MFBASIC source
rather than in native codegen; the companion is injected automatically when a
program imports `strings` and references the scalar seam. [[src/builtins/strings.rs:implementation_name]] [[src/builtins/strings.rs:uses_package]]

## Description

`strings::fromScalars` encodes a `List OF Scalar` into a `String` by
concatenating the UTF-8 encoding of each element, in order.
[[src/builtins/strings_package.mfb:__strings_fromScalars]]

It is the inverse of `strings::toScalars`: `fromScalars(toScalars(s))` equals `s`
for every `String s`. Encoding always succeeds because a `Scalar` is by
construction a valid, non-surrogate Unicode code point, so there is no
ill-formed input to reject. [[src/builtins/strings_package.mfb:__strings_toScalars]]

Each element contributes one to four bytes depending on its code point, so the
byte length of the result is generally larger than the element count, while
`len` of the result equals the element count exactly. The empty list yields the
empty string.

The input list is not modified; the returned `String` is a fresh owned value.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `scalars` | `List OF Scalar` | The scalars to encode, in order. Any `List OF Scalar` is accepted, including the empty list. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The UTF-8 string formed by concatenating each scalar's encoding. The empty list yields `""`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Build a string from scalar literals:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET chars AS List OF Scalar = [`h`, `i`, `!`]
  io::print(strings::fromScalars(chars))
  RETURN 0
END FUNC
```

Round-trip a string through its scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET original AS String = "héllo中😀"
  LET rebuilt AS String = strings::fromScalars(strings::toScalars(original))
  io::print(toString(rebuilt = original))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings toScalars`
- `mfb man strings isLetter`
- `mfb man strings isDigit`
- `mfb man general toScalar`
- `mfb man strings`
