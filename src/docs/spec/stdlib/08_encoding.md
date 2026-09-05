# Encoding Codecs

The `encoding` package is a pure-MFBASIC source package that converts between raw
bytes and text, between text and Unicode code units, and between bytes and text in
the legacy single-byte codepages. It is built on the
built-in `bits` package (bitwise/shift/rotate primitives), `strings::toBytes`
(the raw UTF-8 bytes of a `String`, the inverse of `toString(List OF Byte)`), and
`collections`. [[src/codegen/builtins/encoding/mod.rs]]

This topic owns the codec *models* (algorithms, alphabets, padding, and error
conditions). The per-function API — signatures, parameters, return types, errors
— is owned by `./mfb man encoding`. The integer bitwise/shift/rotate primitives
the codecs lean on are native inline operations (each one, or a few, machine
instructions) owned by `./mfb man bits`.

Outputs are standardized, so the native and Binary Representation execution paths
produce identical results, and every encoder/decoder pair round-trips. Encoders
are **total** except `uleb128Encode`, which rejects a negative value, and
`codepageEncode`, which rejects a character its codepage cannot spell; decoders
fail closed with `ErrInvalidFormat` (`77050003`) on malformed input, so a `TRAP`
can recover from bad data.

## The String↔bytes seam

Every text-oriented codec rests on one native primitive,
`strings::toBytes(value AS String) AS List OF Byte`, which exposes the UTF-8
bytes that already back a `String`. Its inverse is the universal
`toString(List OF Byte)`. The package adds two derived helpers on top:

- `__encoding_codepoints(String) AS List OF Integer` decodes the UTF-8 bytes into
  Unicode scalar values (used by `utf16Encode`, `utf32Encode`, and
  `punycodeEncode`).
- `__encoding_fromCodepoint(Integer) AS String` UTF-8-encodes one scalar value
  (used by every `*Decode` that rebuilds text from individual code points).

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
and inserts each code point at its computed position (RFC 3492's in-place shift);
it fails with `ErrInvalidFormat` on an invalid digit, a truncated sequence, a
variable-length integer that would overflow (the RFC §6.4 checks), or an encoded
label longer than 1024 octets — the insertion is quadratic in the label's length,
and 1024 is sixteen times the 63-octet DNS label limit (RFC 1034 §3.1, RFC 5890
§2.3.1) and past every RFC 3492 sample string, so the bound is unreachable by any
host label or by a round trip of ordinary text (bug-510).

## LEB128 and varints

- **`uleb128Encode`/`Decode`** are unsigned LEB128 (7 data bits per byte, high bit
  = continuation). Encoding fails on a negative value; decoding fails on a
  sequence wider than 64 bits or one that ends without a terminating byte.
- **`sleb128Encode`/`Decode`** are signed LEB128 with the standard sign-bit
  termination test and sign extension on decode.
- **`varintEncode`/`Decode`** map the signed value through ZigZag
  (`(n << 1) XOR (n >> 63)`) and then unsigned LEB128, so small-magnitude negative
  numbers stay short. Decoding reverses the ZigZag mapping.

## Legacy single-byte codepages

`codepageDecode`/`codepageEncode` move between a `List OF Byte` and a `String` in
one of the `Codepage` members, so content that is not UTF-8 — a `windows-1252` page
body, a `KOI8-R` mail part, an `IBM866` DOS file — can be read and written without
a second package. `Codepage` is an `EXPORT ENUM` of the WHATWG Encoding Standard's
**28 legacy single-byte labels** plus `Codepage.Utf8`.

The table data is the standard's own: each codepage is one of the 27 distinct
`index-<label>.txt` files vendored under `tools/codepage-index/`, generated into
`helper_codepage_table.rs` by `scripts/gen_codepage_tables.py` and checked back
against those files at test time. ISO-8859-8-I has no index of its own and shares
ISO-8859-8's mapping — the two differ only in bidi display direction, not in the
byte↔code-point mapping.

- **Representation.** A codepage's high half is one 128-scalar `String` literal:
  scalar *i* is the code point for byte `128 + i`. A byte the codepage leaves
  undefined carries `U+FFFD`, which is unambiguous because the highest code point
  across all 27 tables is `U+FB02`. `U+FFFD` is a table sentinel only; it is never
  a decoded output value.
- **Decoding** follows the standard's single-byte decoder. Bytes `0x00`–`0x7F` are
  ASCII in every single-byte codepage and decode to themselves; `0x80`–`0xFF` index
  the table. A byte the codepage leaves undefined raises `ErrInvalidFormat`
  (`77050003`) rather than becoming `U+FFFD`, so a decode either spells the whole
  input or fails — the standard's replacement-character error mode is a caller
  policy, not this package's.
- **Encoding** is the same table read backwards, and is exact: within any one index
  no code point appears twice, so a character maps to at most one byte. A scalar
  below `U+0080` emits its own byte; anything else is searched for in the table. A
  character the codepage cannot spell — including a grapheme wider than one scalar,
  and including `U+FFFD` itself, which must be rejected before the search or it
  would match a table hole — raises `ErrInvalidFormat`.
- **Round-trip.** For every single-byte `Codepage`, every byte sequence that decodes
  without raising re-encodes to exactly those bytes.
- **`Codepage.Utf8`** is not a single-byte codepage: both directions delegate to
  `utf8Decode`/`utf8Encode`, so one call site can dispatch on a charset label
  without a separate branch for the most common encoding.
- **Out of scope here.** Resolving a charset *label* (`"windows-1252"` and the
  standard's ~220 aliases) to a `Codepage`, and the multi-byte legacy encodings
  (GBK, gb18030, Big5, EUC-JP, ISO-2022-JP, Shift_JIS, EUC-KR), whose four index
  tables hold 67,302 mappings against these 3,342 and need a different
  representation.

## See Also

* ./mfb man encoding — the per-function API reference
* ./mfb man bits — the integer bitwise/shift/rotate primitives the codecs use
* ./mfb spec architecture frontend — how this source package is injected
