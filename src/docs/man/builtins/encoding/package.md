# encoding

Byte-to-text and Unicode codecs: UTF-8/16/32, Base32/64, hex, percent, HTML, LEB128, and Punycode

## Synopsis

```
IMPORT encoding
encoding::utf8Encode(text)
encoding::base64Encode(bytes)
encoding::hexDecode(text)
encoding::percentEncode(text)
```

## Description

The `encoding` package converts between text and its encoded byte or code-unit
representations. It is a built-in package written in MFBASIC source over the
`bits`, `strings`, and `collections` packages, so `IMPORT encoding` needs no
manifest dependency. [[src/builtins/encoding.rs:augmented_project]]

The package defines no new types. It works with three ordinary value shapes:
`String` (always UTF-8 text), `List OF Byte` (raw octets, each `0..255`), and
`List OF Integer` (code units or scalar values as `Integer`). Byte transport uses
`strings::toBytes` — the raw UTF-8 bytes of a `String`, the inverse of
`toString(List OF Byte)` — as its foundation. [[src/builtins/encoding_package.mfb:__encoding_utf8EncodeBytes]]

Functions come in encode/decode pairs and are total round-trips over well-formed
input. Encoders always succeed; decoders validate their input and fail on
malformed data (see Errors). The character codecs cover the Unicode
transformation formats — `utf8Encode`/`utf8Decode`, `utf16Encode`/`utf16Decode`
(with surrogate pairing), and `utf32Encode`/`utf32Decode` (raw scalar values).
The binary-to-text codecs (`base32`, `base64`, `base64Url`, `hex`) map a
`List OF Byte` to an ASCII `String` and back; `base64`/`base32` emit `=` padding
while `base64Url` is unpadded. The web codecs (`percentEncode`/`percentDecode`,
`formUrlEncode`/`formUrlDecode`, `htmlEscape`/`htmlUnescape`) operate `String` to
`String`. The integer codecs (`uleb128`, `sleb128`, `varint`) serialize a single
`Integer` to a `List OF Byte`; `varint` applies zig-zag mapping. `punycodeEncode`/
`punycodeDecode` implement the RFC 3492 Bootstring transform per dot-separated
label, adding or stripping the `xn--` prefix only on labels that need it. [[src/builtins/encoding_package.mfb:__encoding_punycodeEncode]]

`utf8Encode` and `utf8Decode` are overloaded. `utf8Encode` is a *return-type*
overload: the same `String` argument yields either a `List OF Byte` or a
`List OF Integer` depending on the expected (contextual) type, and a call with no
type context to select the overload is a compile-time `TYPE_OVERLOAD_AMBIGUOUS`
error, not a runtime failure. `utf8Decode` is a parameter overload selected by
whether its argument is a `List OF Byte` or a `List OF Integer`. Both are resolved
during monomorphization. [[src/builtins/encoding.rs:resolve_overload_target]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | raised by any decoder on malformed input — `utf8Decode`/`utf16Decode`/`utf32Decode` on an out-of-range code unit, invalid or overlong UTF-8, a surrogate code point, or an unpaired surrogate; `hexDecode` on odd length or a non-hex digit; `base32Decode`/`base64Decode`/`base64UrlDecode` on a bad length, an out-of-alphabet character, or misplaced padding; `percentDecode`/`formUrlDecode` on a truncated or invalid `%` escape or non-UTF-8 result; `htmlUnescape` on a malformed or unknown entity; `uleb128Decode`/`sleb128Decode`/`varintDecode` on truncated input or overflow; `punycodeDecode` on an invalid Punycode label; and the integer encoders (`uleb128Encode`) on a negative value [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
