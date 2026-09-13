# unicode

Unicode behavior, indexes, normalization, and licensing

## Synopsis

```
mfb man unicode
```

## Description

MFBASIC strings are UTF-8 text. String length, search, and substring indexes
are zero-based Unicode scalar indexes. They are **not** byte offsets and they
are **not** grapheme-cluster indexes.

Use `strings::byteLen` when byte length is required. Use `strings::graphemes`
when user-perceived extended grapheme clusters are required.

Terminal columns are a fourth measure, distinct from bytes, scalars, and
graphemes: `strings::displayWidth("日本語")` is `6`. When text must line up in a
terminal, pad it with `strings::padLeftToWidth` or `strings::padRightToWidth`,
which count columns rather than scalars.

Unicode string functions are deterministic and locale-independent. Case
mapping, case folding, NFC normalization, Unicode whitespace, and extended
grapheme cluster behavior follow the Unicode data embedded in the
compiler/runtime.

`strings::normalizeNfc` returns canonical NFC text. `strings::caseFold` is
intended for caseless comparison and can change string length. `strings::upper`
and `strings::lower` can also change string length because Unicode case
mappings can expand one scalar into multiple scalars.

## License

MFBASIC is licensed under the MIT/Expat license.

Unicode runtime tables and algorithms may include or derive from utf8proc,
which is licensed under the MIT/Expat license. Unicode data is covered by the
Unicode data license. See the repository LICENSE file for full notices.

## See also

- `mfb man types string` — `String` and `Scalar` literals and `\u{HEX}` escapes
- `mfb man strings`
- `mfb man strings byteLen`
- `mfb man strings graphemes`
- `mfb man strings normalizeNfc`
- `mfb man strings caseFold`
- `mfb man strings displayWidth`
- `mfb man strings padLeftToWidth`
- `mfb man general len`
- `mfb man general find`
- `mfb man general mid`
