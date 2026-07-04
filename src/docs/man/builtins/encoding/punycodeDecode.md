# punycodeDecode

Decode an ASCII Punycode hostname back to its Unicode form.

## Synopsis

```
encoding::punycodeDecode(asciiDomain AS String) AS String
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

`encoding::punycodeDecode` converts an ASCII hostname in the internationalized
domain name (IDNA) representation back to Unicode, reversing the Punycode
Bootstring algorithm of RFC 3492. It is the inverse of
`encoding::punycodeEncode`. [[src/builtins/encoding_package.mfb:__encoding_punycodeDecode]]

The hostname is split on `.` into labels, and each label is processed
independently; the results are rejoined with `.` so the dot structure of the
input is preserved. A label that begins with the ACE marker `xn--` is decoded:
the `xn--` prefix is stripped and the remainder is run through the Punycode
label decoder. A label without the `xn--` prefix is emitted verbatim, unchanged.
[[src/builtins/encoding_package.mfb:__encoding_punycodeDecode]]

Within an encoded label, the basic (ASCII) code points up to and including the
last `-` delimiter are copied out first, and the trailing generalized
variable-length integers are decoded to reconstruct the non-ASCII code points and
their insertion positions. The decoder uses the RFC 3492 parameters (initial
`n` = 128, initial bias 72, base 36) and the standard bias-adaptation function.
The reconstructed code points are re-encoded to a UTF-8 `String` on return.
[[src/builtins/encoding_package.mfb:__encoding_punyDecodeLabel]]

The input is expected to be well-formed Punycode. Malformed input — a basic
(pre-delimiter) byte at or above `128`, a variable-length integer that is
truncated before it terminates, a byte that is not a valid base-36 digit, or a
decoded scalar value outside the Unicode range — raises a runtime error rather
than producing a partial result. [[src/builtins/encoding_package.mfb:__encoding_punyValue]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `asciiDomain` | `String` | The ASCII Punycode hostname to decode. Split on `.` into labels; each `xn--`-prefixed label is Punycode-decoded and every other label is copied verbatim. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The decoded Unicode hostname. All-ASCII (non-`xn--`) labels are copied verbatim, and `xn--`-prefixed labels are expanded to their Unicode form. The empty string maps to the empty string. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | An `xn--` label is malformed Punycode: a pre-delimiter byte is `>= 128`, a variable-length integer is truncated or contains a non-base-36 digit, or a decoded scalar value falls outside the valid Unicode range. [[src/builtins/encoding_package.mfb:__encoding_punyDecodeLabel]] |

## Examples

Decode a Punycode label to Unicode:

```
IMPORT encoding

io::print(encoding::punycodeDecode("xn--mnchen-3ya.de"))
```

Round-trip through `punycodeEncode`:

```
IMPORT encoding

LET ace AS String = encoding::punycodeEncode("bücher.example")
io::print(ace)
io::print(encoding::punycodeDecode(ace))
```

## See also

- `mfb man encoding punycodeEncode`
- `mfb man encoding percentDecode`
- `mfb man encoding utf8Decode`
- `mfb man encoding`
