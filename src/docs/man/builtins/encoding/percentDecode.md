# percentDecode

Decode a percent-encoded (URL-encoded) `String` back into text.

## Synopsis

```
encoding::percentDecode(text AS String) AS String
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

`encoding::percentDecode` reverses `encoding::percentEncode`, expanding every
`%XX` escape in `text` back into the byte it names. The input is scanned as its
raw byte sequence: each `%` (byte 37) introduces a two-digit hexadecimal escape
whose value becomes a single output byte, and every other byte is copied through
unchanged. The accumulated bytes are then interpreted as UTF-8 to produce the
returned `String`. [[src/builtins/encoding_package.mfb:__encoding_percentDecodeBytes]]

The two hex digits after a `%` accept either case (`0`–`9`, `a`–`f`, `A`–`F`) and
may be mixed. Unlike `encoding::formUrlDecode`, a literal `+` (byte 43) is *not*
translated to a space — it passes through verbatim — because plus-as-space is an
`application/x-www-form-urlencoded` convention, not part of RFC 3986 percent
encoding. [[src/builtins/encoding_package.mfb:__encoding_percentDecode]] [[src/builtins/encoding_package.mfb:__encoding_hexValue]]

The empty string decodes to the empty string. The function is a strict decoder:
a `%` with fewer than two following bytes, a `%` followed by a non-hex digit, or
a decoded byte sequence that is not valid UTF-8 all raise an error rather than
being passed through or replaced. [[src/builtins/encoding_package.mfb:__encoding_utf8Valid]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `text` | `String` | The percent-encoded text to decode. Every `%` must be followed by two hexadecimal digits (`0`–`9`, `a`–`f`, `A`–`F`); all other bytes are literal. The empty string is accepted and decodes to the empty string. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The decoded text: each `%XX` escape replaced by its byte, all other bytes verbatim, and the whole interpreted as UTF-8. The empty string for empty input. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | A `%` is not followed by two more bytes (truncated escape), a `%` is followed by a byte that is not a hexadecimal digit, or the decoded bytes are not valid UTF-8. [[src/builtins/encoding_package.mfb:__encoding_percentDecodeBytes]] |

## Examples

Decode a percent-encoded string containing a space escape:

```
IMPORT encoding
IMPORT io

io::print(encoding::percentDecode("a%20b"))
```

Round-trip through `percentEncode`, including a non-ASCII character:

```
IMPORT encoding
IMPORT io

LET enc AS String = encoding::percentEncode("café & tea")
io::print(enc)
io::print(encoding::percentDecode(enc))
```

## See also

- `mfb man encoding percentEncode`
- `mfb man encoding formUrlDecode`
- `mfb man encoding hexDecode`
- `mfb man encoding`
