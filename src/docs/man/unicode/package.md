# unicode

Unicode behavior, indexes, normalization, and licensing

## Synopsis

```
mfb man unicode
```

## Description

MFBASIC strings are UTF-8 text. String length, search (`strings::find`), and
substring indexes (`strings::mid`), including every `regex::` position, are
zero-based Unicode scalar indexes. They are **not** byte offsets and they
are **not** grapheme-cluster indexes.

Use `strings::byteLen` when byte length is required. Use `strings::graphemes`
when user-perceived extended grapheme clusters are required.

Terminal columns are a fourth measure, distinct from bytes, scalars, and
graphemes: `term::measureText("日本語")` is `6` while the terminal surface is
active. When text must line up without activating that surface, pad it with
`strings::padLeftToWidth` or `strings::padRightToWidth`, which use the same
column rules rather than counting scalars.

Unicode string functions are deterministic and locale-independent. Case
mapping, case folding, NFC normalization, Unicode whitespace, and extended
grapheme cluster behavior follow the Unicode 16.0 data embedded in the
compiler, so results do not depend on the host system's Unicode tables or
locale.

`strings::normalizeNfc` returns canonical NFC text. `strings::caseFold` is
intended for caseless comparison and can change string length. `strings::upper`
and `strings::lower` can also change string length because Unicode case
mappings can expand one scalar into multiple scalars.

String `=`, `<>` and ordering compare Unicode scalar values directly, with no
normalization, case folding or locale collation: `"e\u{301}" = "é"` is `FALSE`
until both sides are passed through `strings::normalizeNfc`, and a caseless
comparison needs `strings::caseFold` on both sides. See `mfb man types string`.

A single scalar is the `Scalar` primitive (written `` `中` ``).
`strings::toScalars` splits a `String` into them and `strings::fromScalars`
rebuilds it; `toInt` and `toScalar` convert between a `Scalar` and its code
point. `strings::isWhitespace`, `strings::trim` and friends use the Unicode
White_Space property.

A `String` is always well-formed UTF-8. Bytes arrive as `List OF Byte` and
become text through `encoding::utf8Decode`, which raises `ErrInvalidFormat` on a
malformed sequence rather than substituting a replacement character;
`strings::toBytes` and `encoding::utf8Encode` go the other way, and `encoding`
also provides the UTF-16 and UTF-32 codecs.

## License

MFBASIC is licensed under the MIT/Expat license.

Unicode runtime tables and algorithms may include or derive from utf8proc,
which is licensed under the MIT/Expat license. Unicode data is covered by the
Unicode data license. See the repository LICENSE file for full notices.

## See also

- `mfb man types string` — `String` and `Scalar` literals, `\u{HEX}` escapes, comparison and ordering
- `mfb man strings`
- `mfb man strings byteLen`
- `mfb man strings graphemes`
- `mfb man strings normalizeNfc`
- `mfb man strings caseFold`
- `mfb man term measureText`
- `mfb man strings padLeftToWidth`
- `mfb man general len`
- `mfb man strings find`
- `mfb man strings mid`
- `mfb man strings toScalars`
- `mfb man strings isWhitespace`
- `mfb man encoding utf8Decode`
- `mfb man regex`
