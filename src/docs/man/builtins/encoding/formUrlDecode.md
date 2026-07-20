# formUrlDecode

Decode `application/x-www-form-urlencoded` text back into a `String`.

## Synopsis

```
encoding::formUrlDecode(text AS String) AS String
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

`encoding::formUrlDecode` reverses `encoding::formUrlEncode`, parsing
`application/x-www-form-urlencoded` data — the format HTML forms apply to
query-string values — back into text. The input is read as its UTF-8 byte
sequence and scanned left to right, producing a sequence of decoded bytes.
[[src/builtins/encoding_package.mfb:__encoding_formUrlDecode]] [[src/builtins/encoding_package.mfb:__encoding_percentDecodeBytes]]

Each byte is handled as follows:

- A `%` (byte 37) begins a three-character escape `%XX`, where `XX` is two
  hexadecimal digits. The two digits are decoded (case-insensitively) into a
  single byte and the scan advances past all three characters.
- A `+` (byte 43) is replaced by a single space (byte 32). This is the one
  behavior that distinguishes form decoding from `encoding::percentDecode`,
  which leaves `+` untouched. [[src/builtins/encoding_package.mfb:__encoding_percentDecode]]
- Every other byte is copied through unchanged.

After the whole input has been decoded, the resulting byte sequence is
validated as UTF-8 and returned as a `String`. The empty string decodes to the
empty string. Hexadecimal digits in escapes may be upper- or lowercase.
[[src/builtins/encoding_package.mfb:__encoding_utf8Valid]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `text` | `String` | The form-encoded text to decode. Any string, including the empty string, is accepted; it is read as its UTF-8 byte sequence. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The decoded text: `%XX` escapes turned into their bytes, `+` turned into space, all other bytes verbatim, with the whole result validated as UTF-8. The empty string for empty input. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | A `%` has fewer than two characters after it (a truncated escape), a `%XX` escape contains a non-hexadecimal digit, or the fully decoded byte sequence is not valid UTF-8. [[src/builtins/encoding_package.mfb:__encoding_percentDecodeBytes]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Decode a form field value, turning `+` into a space:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::formUrlDecode("name+%3D+a+b"))
END SUB
```

Round-trip through `formUrlEncode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET enc AS String = encoding::formUrlEncode("café & tea")
  io::print(enc)
  io::print(encoding::formUrlDecode(enc))
END SUB
```

## See also

- `mfb man encoding formUrlEncode`
- `mfb man encoding percentDecode`
- `mfb man encoding percentEncode`
- `mfb man encoding`
