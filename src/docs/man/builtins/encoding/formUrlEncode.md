# formUrlEncode

Encode a `String` as `application/x-www-form-urlencoded` data.

## Synopsis

```
encoding::formUrlEncode(text AS String) AS String
```

## Package

encoding

## Imports

```
IMPORT encoding
```

`encoding` is a built-in package written in MFBASIC source, so no manifest
dependency is required. [[src/builtins/encoding.rs:augmented_project]]

## Description

`encoding::formUrlEncode` encodes `text` using the
`application/x-www-form-urlencoded` rules that HTML forms apply to query-string
values. The input is first converted to its UTF-8 byte sequence, then each byte
is emitted in order. [[src/builtins/encoding_package.mfb:__encoding_formUrlEncode]]

A byte passes through unchanged only when it is an ASCII alphanumeric: the
letters `A`–`Z` (65–90) and `a`–`z` (97–122) and the digits `0`–`9` (48–57).
The space byte (32) is emitted as a single `+`. Every other byte — including
`-`, `.`, `_`, `~`, reserved and sub-delimiter characters, control bytes, and
every continuation byte of a multi-byte UTF-8 character — is emitted as a
three-character escape `%XX`, where `XX` is the byte value in **uppercase**
hexadecimal. [[src/builtins/encoding_package.mfb:__encoding_isAlphaNum]] [[src/builtins/encoding_package.mfb:__encoding_percentByte]]

This differs from `encoding::percentEncode`, which leaves the four unreserved
marks `-`, `.`, `_`, and `~` untouched and escapes space as `%20` rather than
`+`. Because non-ASCII characters are encoded from their UTF-8 bytes, a single
such character expands to one `%XX` escape per byte (two escapes for most Latin
and symbol characters, three or four for higher code points).

The function is **total**: every `String`, including the empty string (which
yields the empty string), encodes successfully and it never raises a runtime
error. The inverse operation is `encoding::formUrlDecode`, which parses `%XX`
escapes and `+` back into text. [[src/builtins/encoding_package.mfb:__encoding_formUrlDecode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `text` | `String` | The text to encode. Any string, including the empty string, is accepted; it is interpreted as its UTF-8 byte sequence. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The form-encoded form of `text`: ASCII alphanumeric bytes verbatim, space as `+`, all other bytes as `%XX` with uppercase hex. The empty string for empty input. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode a form field value containing a space and reserved characters:

```
IMPORT encoding
IMPORT io

io::print(encoding::formUrlEncode("name = a b & c"))
```

Round-trip through `formUrlDecode`:

```
IMPORT encoding
IMPORT io

LET enc AS String = encoding::formUrlEncode("café & tea")
io::print(enc)
io::print(encoding::formUrlDecode(enc))
```

## See also

- `mfb man encoding formUrlDecode`
- `mfb man encoding percentEncode`
- `mfb man encoding percentDecode`
- `mfb man encoding`
