# find

Locate the first regular-expression match and return its start index.

## Synopsis

```
regex::find(value AS String, pattern AS String) AS Integer
regex::find(value AS String, pattern AS String, start AS Integer) AS Integer
```

## Package

regex

## Imports

```
IMPORT regex
```

## Description

`regex::find` compiles `pattern` as a regular expression, searches `value` for
the first match beginning at or after the position `start`, and returns the
zero-based index where that match starts. It is the locating form of the
package: `regex::match` reports only whether a match exists, `find` reports
where the first one begins, and `regex::findAll` reports the start of every
non-overlapping match.

The search is unanchored and leftmost. A match is sought at each position
`start`, `start+1`, … in turn, and the smallest position at which the pattern
can match is reported; at that position the engine resolves the match by
preference order (earlier alternatives, greedy quantifiers as long as possible,
lazy ones as short as possible), but only the start index is returned. `start`
restricts only where a match may begin; it does not redefine the input, so the
absolute anchors `\A` and `\z`, and `^` and `$` when the `m` flag is off, are
still evaluated against the whole value. For example `regex::find("abc", "^b", 1)`
finds nothing, because `^` is absolute position `0`. A zero-length match is valid
and reports its own start position; an empty or empty-matching pattern matches
immediately at `start`.

Positions are Unicode scalar values, never UTF-8 bytes and never grapheme
clusters, consistent with `len` and the `strings` package. A string of `n`
scalars has positions `0` … `n`; position `n` is after the last scalar. Both the
`start` argument and the returned index are scalar indexes.

`start` defaults to `0`, meaning the search begins at the start of `value`. It
must be in the range `0` through the scalar length of `value` inclusive; the
upper bound equals the length so that a search may begin at the end of the
string (where only a zero-length or end-anchored pattern can match). A negative
`start`, or one greater than the scalar length, is out of range and fails with
`ErrIndexOutOfRange`. [[src/builtins/regex_package.mfb:__regex_find]]

`pattern` is an ordinary runtime `String`, so it may be built or read at run
time; it uses MFBASIC's own portable regex dialect, defined in
`mfb spec stdlib regex` (run `mfb man regex` for the language overview), which
produces identical results on every target and never defers to a host regex
library. Because `String` literals process backslash escapes, a literal
backslash is written `"\\"` — `regex::find(value, "\\d")` searches for the first
digit. An invalid pattern fails with `ErrInvalidFormat`; when no match exists at
or after `start`, `find` returns `-1` rather than failing. Because every real
match position is `>= 0`, `-1` is an unambiguous "no match" sentinel. (This
differs from `strings::find`, which fails with `ErrNotFound` on absence.)

`find` does not mutate `value` or `pattern` and has no side effects.

## Overloads

**`regex::find(value AS String, pattern AS String) AS Integer`**

Searches from position `0`; equivalent to passing `start = 0`. The trailing
`start` is supplied during lowering as a default of `0`.
[[src/builtins/regex.rs:default_argument_padding]]

**`regex::find(value AS String, pattern AS String, start AS Integer) AS Integer`**

Searches from the explicit scalar position `start`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The subject text searched for a match. It is never modified. |
| `pattern` | `String` | The regular expression to compile and search for. It must be a valid pattern in the MFBASIC regex dialect; otherwise the call fails with `ErrInvalidFormat`. |
| `start` | `Integer` | The zero-based scalar index at or after which the match must begin. Defaults to `0`. Must be between `0` and the scalar length of `value` inclusive; `start == len(value)` is allowed and can match a zero-length or end-anchored pattern. May be passed by name. |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The zero-based Unicode scalar index where the first match at or after `start` begins, or `-1` when there is no match. A zero-length match reports its own start position. [[src/builtins/regex.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `pattern` is not a valid regular expression: an unbalanced or unterminated group or class, a quantifier with no atom or stacked quantifiers, a counted quantifier with `m > n`, a class range whose low endpoint exceeds its high endpoint, an empty class, a backslash escape outside the defined set, a `\x...`/`\x{...}` value that is not a valid scalar, an unknown `\p{...}` property, a malformed flag or group head, or a non-goal construct (backreference or look-around). Pattern compilation is checked before `start`, so this error takes precedence when both apply. [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050001` | `ErrIndexOutOfRange` | `start` is less than `0` or greater than the scalar length of `value`. [[src/builtins/regex_package.mfb:__regex_find]] [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] |

## Examples

Find the first occurrence, and the first at or after a start position:

```
IMPORT regex
LET firstL AS Integer = regex::find("hello", "l")
LET nextL AS Integer = regex::find("hello", "l", 3)
```

Find the first digit (note the doubled backslash in the String literal):

```
IMPORT regex
LET firstDigit AS Integer = regex::find("a1b2c3", "\\d")
```

Handle absence with the `-1` sentinel:

```
IMPORT regex

LET i AS Integer = regex::find("abc", "\\d")
IF i >= 0 THEN
  io::print("matched at " & toString(i))
ELSE
  io::print("no match")
END IF
```

## See also

- `mfb man regex match`
- `mfb man regex findAll`
- `mfb man regex replace`
- `mfb man strings find`
