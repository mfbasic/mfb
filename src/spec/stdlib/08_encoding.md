# Encoding Codecs

The `encoding` package is a pure-MFBASIC source package that converts between raw
bytes and text and between text and Unicode code units. It is built on the
built-in `bits` package (bitwise/shift/rotate primitives), `strings::toBytes`
(the raw UTF-8 bytes of a `String`, the inverse of `toString(List OF Byte)`), and
`collections`. [[src/builtins/encoding_package.mfb]]

This topic owns the codec *models* (algorithms, alphabets, padding, and error
conditions). The per-function API — signatures, parameters, return types, errors
— is owned by `./mfb man encoding`. The integer bitwise/shift/rotate primitives
the codecs lean on are native single-instruction operations owned by
`./mfb man bits`.

Outputs are standardized, so the native and Binary Representation execution paths
produce identical results, and every encoder/decoder pair round-trips. Encoders
are **total**; decoders fail closed with `ErrInvalidFormat` (`77050003`) on
malformed input, so a `TRAP` can recover from bad data.

## The String↔bytes seam

Every text-oriented codec rests on one native primitive,
`strings::toBytes(value AS String) AS List OF Byte`, which exposes the UTF-8
bytes that already back a `String`. Its inverse is the universal
`toString(List OF Byte)`. The package adds two derived helpers on top:

- `__encoding_codepoints(String) AS List OF Integer` decodes the UTF-8 bytes into
  Unicode scalar values (used by `utf16Encode`, `utf32Encode`, and
  `punycodeEncode`).
- `__encoding_fromCodepoint(Integer) AS String` UTF-8-encodes one scalar value
  (used by every `*Decode` that rebuilds text).

## Unicode transforms

- **`utf8Encode`** is a *return-type overload*. With an expected type of
  `List OF Byte` it returns the raw bytes; with `List OF Integer` it returns the
  identical `0..255` values as Integers (for arithmetic on code units). An
  unannotated call is the compile-time `TYPE_OVERLOAD_AMBIGUOUS` error (resolved
  in the monomorphizer by the call's expected type). `utf8Decode` is selected by
  its **parameter** type (`List OF Byte` or `List OF Integer`).
- **`utf8Decode`** validates well-formedness before building the `String`: it
  rejects overlong forms, continuation/lead-byte violations, code points above
  `0x10FFFF`, surrogate code points, and (for the `List OF Integer` form) elements
  outside `0..255`.
- **`utf16Encode`/`Decode`** map scalar values to/from 16-bit code units; astral
  code points (`> 0xFFFF`) become surrogate pairs. These are numeric code units,
  not a byte serialization, so endianness does not apply. Decoding rejects an
  element outside `0..65535` and any unpaired surrogate.
- **`utf32Encode`/`Decode`** are one element per scalar value. Decoding rejects a
  code point outside `0..0x10FFFF` or inside the surrogate range
  `0xD800..0xDFFF`.

## Base-N byte↔text codecs

`hex`, `base32`, `base64`, and `base64Url` serialize bytes to text and back.

- **Hex** is two lowercase characters per byte, no separators (`strings::upper`
  for uppercase). Decoding fails on a non-hex character or an odd length.
- **Base32/Base64/Base64url** share one bit-buffer engine: bytes are streamed
  into an accumulator and drained `bitsPer` bits at a time (5 for Base32, 6 for
  Base64) through the codec's alphabet. The alphabets are RFC 4648: Base32 §6
  (uppercase `A–Z 2–7`, `=` padding), Base64 §4 (`A–Za–z0–9+/`, `=` padding), and
  Base64url §5 (`-`/`_`, **no** padding). Decoding validates the alphabet, that
  `=` appears only as a trailing run, the group length, and (Base64) that the
  total input length is a multiple of 4. Base64url decoding accepts input with or
  without padding.

## URL and HTML escaping

- **`percentEncode`/`Decode`** implement RFC 3986: the unreserved set
  `A–Z a–z 0–9 - . _ ~` passes through; every other byte of the UTF-8 encoding
  becomes `%XX` with uppercase hex. Decoding interprets the recovered bytes as
  UTF-8 and fails on a malformed `%XX` escape or invalid UTF-8. `+` is **not**
  a space here.
- **`formUrlEncode`/`Decode`** implement `application/x-www-form-urlencoded`:
  spaces become `+`, all other non-alphanumeric bytes become `%XX`, and decoding
  reverses both (`+`→space, `%XX`→byte) before UTF-8 validation.
- **`htmlEscape`** replaces `<`, `>`, `&`, `"`, and `'` with `&lt;`, `&gt;`,
  `&amp;`, `&quot;`, and `&apos;` (ampersand first, so nothing is double-escaped).
  **`htmlUnescape`** decodes numeric entities (`&#233;`, `&#xE9;`) and a named
  entity set (the core five plus the common Latin-1/symbol names); it fails on a
  malformed entity structure (no terminating `;`) or an unknown name.

## Punycode (RFC 3492)

`punycodeEncode`/`punycodeDecode` apply the Bootstring algorithm per host *label*
(splitting on `.`): a label with any non-ASCII code point is encoded with the
`xn--` prefix, and the standard parameters (`base 36`, `tmin 1`, `tmax 26`,
`skew 38`, `damp 700`, `initial_bias 72`, `initial_n 128`) drive the
delta/bias adaptation. Decoding reverses the generalized variable-length integers
and inserts each code point at its computed position; it fails on an invalid digit
or a truncated sequence.

## LEB128 and varints

- **`uleb128Encode`/`Decode`** are unsigned LEB128 (7 data bits per byte, high bit
  = continuation). Encoding fails on a negative value; decoding fails on a
  sequence wider than 64 bits or one that ends without a terminating byte.
- **`sleb128Encode`/`Decode`** are signed LEB128 with the standard sign-bit
  termination test and sign extension on decode.
- **`varintEncode`/`Decode`** map the signed value through ZigZag
  (`(n << 1) XOR (n >> 63)`) and then unsigned LEB128, so small-magnitude negative
  numbers stay short. Decoding reverses the ZigZag mapping.

## See Also

* ./mfb man encoding — the per-function API reference
* ./mfb man bits — the integer bitwise/shift/rotate primitives the codecs use
* ./mfb spec architecture frontend — how this source package is injected
