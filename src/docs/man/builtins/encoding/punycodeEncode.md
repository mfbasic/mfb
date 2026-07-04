# punycodeEncode

Encode a Unicode hostname to its ASCII Punycode form.

## Synopsis

```
encoding::punycodeEncode(domain AS String) AS String
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

`encoding::punycodeEncode` converts a Unicode hostname `domain` to the ASCII
representation used by internationalized domain names (IDNA), applying the
Punycode Bootstring algorithm of RFC 3492. The hostname is split on `.` into
labels, and each label is processed independently; the results are rejoined with
`.` so the dot structure of the input is preserved. [[src/builtins/encoding_package.mfb:__encoding_punycodeEncode]]

Each label is examined for non-ASCII code points. A label whose code points are
all below `128` is emitted verbatim, unchanged. A label containing any code
point at or above `128` is Punycode-encoded and prefixed with the ACE marker
`xn--`, producing the standard `xn--<encoding>` form. [[src/builtins/encoding_package.mfb:__encoding_labelHasNonAscii]]

Within an encoded label, the basic (ASCII) code points are copied out first,
followed by a `-` delimiter when any basic code points are present, and then the
generalized variable-length integers that describe the non-ASCII code points in
ascending order. The algorithm uses the RFC 3492 parameters (initial `n` = 128,
initial bias 72, base 36) and the standard bias-adaptation function. The input
`String` is decoded to Unicode scalar values through the package's UTF-8
decoder before encoding. [[src/builtins/encoding_package.mfb:__encoding_punyEncodeLabel]]

The function is **total**: every `String`, including the empty string and
all-ASCII hostnames, encodes successfully, and it never raises a runtime error.
The inverse operation is `encoding::punycodeDecode`, which converts an ASCII
Punycode hostname back to its Unicode form. [[src/builtins/encoding_package.mfb:__encoding_punycodeDecode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `domain` | `String` | The Unicode hostname to encode. Split on `.` into labels; each label is encoded independently. Any `String`, including the empty string, is accepted. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The ASCII Punycode hostname: all-ASCII labels are copied verbatim, and labels with non-ASCII code points become `xn--`-prefixed Punycode. The empty string maps to the empty string. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode a Unicode hostname to Punycode:

```
IMPORT encoding

io::print(encoding::punycodeEncode("bücher.example"))
```

Round-trip through `punycodeDecode`:

```
IMPORT encoding

LET ace AS String = encoding::punycodeEncode("münchen.de")
io::print(ace)
io::print(encoding::punycodeDecode(ace))
```

## See also

- `mfb man encoding punycodeDecode`
- `mfb man encoding percentEncode`
- `mfb man encoding utf8Encode`
- `mfb man encoding`
