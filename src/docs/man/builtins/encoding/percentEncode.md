# percentEncode

Percent-encode (URL-encode) a `String` per RFC 3986.

## Synopsis

```
encoding::percentEncode(text AS String) AS String
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

`encoding::percentEncode` percent-encodes `text` following the RFC 3986 rules for
the *unreserved* character set. The input is first converted to its UTF-8 byte
sequence, then each byte is emitted in order. [[src/builtins/encoding_package.mfb:__encoding_percentEncode]]

A byte passes through unchanged when it is a member of the unreserved set:
the ASCII letters `A`–`Z` (65–90) and `a`–`z` (97–122), the digits `0`–`9`
(48–57), and the four marks `-` (45), `.` (46), `_` (95), and `~` (126). Every
other byte — including space, reserved and sub-delimiter characters, control
bytes, and every continuation byte of a multi-byte UTF-8 character — is emitted
as a three-character escape `%XX`, where `XX` is the byte value in **uppercase**
hexadecimal. [[src/builtins/encoding_package.mfb:__encoding_isUnreserved]] [[src/builtins/encoding_package.mfb:__encoding_percentByte]]

Because non-ASCII characters are encoded from their UTF-8 bytes, a single such
character expands to one `%XX` escape per byte (two escapes for most Latin and
symbol characters, three or four for higher code points). The function is
**total**: every `String`, including the empty string (which yields the empty
string), encodes successfully and it never raises a runtime error.

The inverse operation is `encoding::percentDecode`, which parses `%XX` escapes
back into text. For `application/x-www-form-urlencoded` data, where space is
encoded as `+`, use `encoding::formUrlEncode` instead. [[src/builtins/encoding_package.mfb:__encoding_formUrlEncode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `text` | `String` | The text to encode. Any string, including the empty string, is accepted; it is interpreted as its UTF-8 byte sequence. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The percent-encoded form of `text`: unreserved bytes verbatim, all others as `%XX` with uppercase hex. The empty string for empty input. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode a path segment containing reserved characters:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::percentEncode("a b/c"))
END SUB
```

Round-trip through `percentDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET enc AS String = encoding::percentEncode("café & tea")
  io::print(enc)
  io::print(encoding::percentDecode(enc))
END SUB
```

## See also

- `mfb man encoding percentDecode`
- `mfb man encoding formUrlEncode`
- `mfb man encoding hexEncode`
- `mfb man encoding`
