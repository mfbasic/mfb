# findAll

Locate every non-overlapping regular-expression match and return their start indices.

## Synopsis

```
regex::findAll(value AS String, pattern AS String) AS List OF Integer
regex::findAll(value AS String, pattern AS String, start AS Integer) AS List OF Integer
```

## Package

regex

## Imports

```
IMPORT regex
```

## Description

`regex::findAll` compiles `pattern` as a regular expression, scans `value` for
every non-overlapping match beginning at or after the position `start`, and
returns a `List OF Integer` holding the zero-based start index of each match in
left-to-right order. It is the enumerating form of the package: `regex::match`
reports only whether a match exists, `regex::find` reports where the first one
begins, and `findAll` reports the start of every match. When there is no match,
the result is the empty list `[]` rather than a failure.
[[src/builtins/regex.rs:call_return_type_name]]

Matches are found by the same leftmost, unanchored search as `regex::find`,
applied repeatedly. After each match the scan resumes at the position just past
the end of that match, so the matches are non-overlapping and the returned
indices are strictly increasing. A zero-length match is recorded, and the scan
then advances by one scalar to make progress; a zero-length match is never
recorded twice at a position already consumed by the previous match, so a
pattern like `a*` against `"aba"` yields the starts of the real runs rather than
an empty match wedged between them. [[src/builtins/regex_package.mfb:__regex_findAll]]

`start` restricts only where the first match may begin; it does not redefine the
input, so the absolute anchors `\A` and `\z`, and `^` and `$` when the `m` flag
is off, are still evaluated against the whole value. Positions are Unicode scalar
values, never UTF-8 bytes and never grapheme clusters, consistent with `len` and
the `strings` package. A string of `n` scalars has positions `0` … `n`; position
`n` is after the last scalar. Both the `start` argument and every returned index
are scalar indexes.

`start` defaults to `0`, meaning the scan begins at the start of `value`. It must
be in the range `0` through the scalar length of `value` inclusive; the upper
bound equals the length so that the scan may begin at the end of the string
(where only a zero-length or end-anchored pattern can match). A negative `start`,
or one greater than the scalar length, is out of range and fails with
`ErrIndexOutOfRange`. [[src/builtins/regex_package.mfb:__regex_findAll]]

`pattern` is an ordinary runtime `String`, so it may be built or read at run
time; it uses MFBASIC's own portable regex dialect, defined in
`mfb spec stdlib regex` (run `mfb man regex` for the language overview), which
produces identical results on every target and never defers to a host regex
library. Because `String` literals process backslash escapes, a literal
backslash is written `"\\"` — `regex::findAll(value, "\\d")` lists the start of
every digit. An invalid pattern fails with `ErrInvalidFormat`. Pattern
compilation is checked before `start`, so `ErrInvalidFormat` takes precedence
when both apply. [[src/builtins/regex_package.mfb:__regex_findAll]]

`findAll` does not mutate `value` or `pattern` and has no side effects.

## Overloads

**`regex::findAll(value AS String, pattern AS String) AS List OF Integer`**

Scans from position `0`; equivalent to passing `start = 0`. The trailing `start`
is supplied during lowering as a default of `0`.
[[src/builtins/regex.rs:default_argument_padding]]

**`regex::findAll(value AS String, pattern AS String, start AS Integer) AS List OF Integer`**

Scans from the explicit scalar position `start`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The subject text scanned for matches. It is never modified. |
| `pattern` | `String` | The regular expression to compile and search for. It must be a valid pattern in the MFBASIC regex dialect; otherwise the call fails with `ErrInvalidFormat`. |
| `start` | `Integer` | The zero-based scalar index at or after which the first match must begin. Defaults to `0`. Must be between `0` and the scalar length of `value` inclusive; `start == len(value)` is allowed and can match a zero-length or end-anchored pattern. May be passed by name. |

## Return value

| Type | Description |
| --- | --- |
| `List OF Integer` | The zero-based Unicode scalar start index of each non-overlapping match at or after `start`, in left-to-right order with strictly increasing values. The list is empty when there is no match. A zero-length match contributes its own start position. [[src/builtins/regex.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `pattern` is not a valid regular expression: an unbalanced or unterminated group or class, a quantifier with no atom or stacked quantifiers, a counted quantifier with `m > n`, a class range whose low endpoint exceeds its high endpoint, an empty class, a backslash escape outside the defined set, a `\x...`/`\x{...}` value that is not a valid scalar, an unknown `\p{...}` property, a malformed flag or group head, or a non-goal construct (backreference or look-around). Pattern compilation is checked before `start`, so this error takes precedence when both apply. [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050001` | `ErrIndexOutOfRange` | `start` is less than `0` or greater than the scalar length of `value`. [[src/builtins/regex_package.mfb:__regex_findAll]] [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] |

## Examples

List the start of every digit (note the doubled backslash in the String literal):

```
IMPORT regex

SUB main()
  LET starts AS List OF Integer = regex::findAll("a1b2c3", "\\d")
END SUB
```

Scan only the tail of the string by passing an explicit start:

```
IMPORT regex

SUB main()
  LET tail AS List OF Integer = regex::findAll("a1b2c3", "\\d", 3)
END SUB
```

Iterate the matches, handling the empty-list "no match" case naturally:

```
IMPORT regex
IMPORT io

SUB main()
  LET starts AS List OF Integer = regex::findAll("the cat sat", "\\w+")
  FOR EACH i IN starts
    io::print("word at " & toString(i))
  NEXT
END SUB
```

## See also

- `mfb man regex find`
- `mfb man regex match`
- `mfb man regex replace`
- `mfb man strings find`
