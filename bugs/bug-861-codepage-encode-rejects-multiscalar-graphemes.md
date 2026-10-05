# bug-861: `encoding::codepageEncode` rejects every multi-scalar grapheme — CRLF text and combining marks cannot be encoded, breaking the stated round trip

Last updated: 2026-10-05
Effort: small (<1h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/encoding/codepage-roundtrip-multiscalar-rt (to add)

`codepageEncode` walks its input with `strings::graphemes` and fails any grapheme whose
`len` is not 1. In this runtime `"\r\n"` is a single two-scalar grapheme, so **any
Windows-1252 / KOI8 / IBM866 text with CRLF line endings fails to encode** with
`ErrInvalidFormat`, which is the most common content those codepages carry. The same goes
for a base letter followed by a combining mark that the codepage *does* define as its own
byte: Windows-1258's combining acute (0xEC), Windows-1255's Hebrew points, and
ISO-8859-6's Arabic harakat. `codepageDecode` produces exactly those strings from valid
bytes, so the spec's round-trip guarantee is false.

Correct behavior: encode scalar by scalar, so each scalar maps to its own byte. Then every
byte sequence that decodes re-encodes to the same bytes.

References:

- src/docs/spec/stdlib/08_encoding.md §Legacy single-byte codepages:
  "**Round-trip.** For every single-byte `Codepage`, every byte sequence that decodes
  without raising re-encodes to exactly those bytes."
- Same section, Encoding bullet: "A character the codepage cannot spell — including a
  grapheme wider than one scalar … raises `ErrInvalidFormat`". This rule is what breaks the
  round trip. A codepage maps code points (scalars), not graphemes.

## Failing Reproduction

`/tmp/hardening/B04/cp2/src/main.mfb`:

```
IMPORT io
IMPORT encoding
IMPORT strings

FUNC rt(label AS String, cp AS encoding::Codepage, bs AS List OF Byte) AS Nothing
  LET s AS String = encoding::codepageDecode(cp, bs)
  io::print(label & " decoded len=" & toString(len(s)) & " hex=" & encoding::hexEncode(strings::toBytes(s)))
  LET back AS List OF Byte = encoding::codepageEncode(cp, s) TRAP(e)
    io::print(label & " re-encode FAIL " & toString(e.code) & " " & e.message)
    RETURN
  END TRAP
  io::print(label & " re-encoded " & encoding::hexEncode(back))
END FUNC

FUNC main() AS Nothing
  rt("1258 a+U+0301", encoding::Codepage.Windows1258, [toByte(97), toByte(236)])
  rt("1258 lone U+0301", encoding::Codepage.Windows1258, [toByte(236)])
  rt("1255 bet+qamats", encoding::Codepage.Windows1255, [toByte(225), toByte(200)])
  rt("8859-6 alef+fatha", encoding::Codepage.Iso8859_6, [toByte(199), toByte(238)])
  rt("1252 CRLF", encoding::Codepage.Windows1252, [toByte(13), toByte(10)])
  rt("utf8 e+U+0301", encoding::Codepage.Utf8, [toByte(101), toByte(204), toByte(129)])
END FUNC
```

```
/tmp/hardening/mfb build -q cp2 && ./cp2/build/cp2.out
```

- Observed:
  ```
  1258 a+U+0301 decoded len=2 hex=61cc81
  1258 a+U+0301 re-encode FAIL 77050003 character not representable in this codepage
  1258 lone U+0301 decoded len=1 hex=cc81
  1258 lone U+0301 re-encoded ec
  1255 bet+qamats decoded len=2 hex=d791d6b8
  1255 bet+qamats re-encode FAIL 77050003 character not representable in this codepage
  8859-6 alef+fatha decoded len=2 hex=d8a7d98e
  8859-6 alef+fatha re-encode FAIL 77050003 character not representable in this codepage
  1252 CRLF decoded len=2 hex=0d0a
  1252 CRLF re-encode FAIL 77050003 character not representable in this codepage
  utf8 e+U+0301 decoded len=2 hex=65cc81
  utf8 e+U+0301 re-encoded 65cc81
  ```
- Expected: `re-encoded 61ec`, `e1c8`, `c7ee`, `0d0a` for the four failing rows.

Contrast: every single high byte (0x80–0xFF) of all 28 single-byte codepages decodes to the
WHATWG index value and re-encodes to itself (3584 cases, all pass:
`/tmp/hardening/B04/cp`, compared against `tools/codepage-index/index-*.txt`). Only
sequences that form a multi-scalar grapheme fail. The `Utf8` member is unaffected.

## Root Cause

`src/codegen/builtins/encoding/func_codepage_encode.rs:__encoding_codepageEncode`:

```
  FOR EACH ch IN strings::graphemes(text)
    ' A grapheme wider than one scalar has no single-byte form at all.
    IF len(ch) <> 1 THEN
      FAIL error(77050003, "character not representable in this codepage")
```

The premise in the comment is wrong. A codepage encodes scalars, and a grapheme of N
scalars is N bytes when each scalar is in the table. The loop should iterate scalars
(`__encoding_codepoints(text)` + `__encoding_fromCodepoint`, or a scalar iterator) and keep
the existing per-scalar `U+FFFD` sentinel check and table search.

## Goal

- The four failing rows re-encode to the original bytes.
- The spec's Round-trip claim holds for every decodable byte sequence. Test it with random multi-byte sequences per codepage, not only single bytes.

### Non-goals (must NOT change)

- `U+FFFD` is still rejected before the table search; unmappable scalars still raise `ErrInvalidFormat`.
- Decode behavior unchanged.

## Blast Radius

- `__encoding_codepageEncode`: fixed by this bug. The spec sentence "including a grapheme wider than one scalar" must be corrected in the same change.
- `__encoding_codepageDecode`: unaffected (byte-wise, verified).
