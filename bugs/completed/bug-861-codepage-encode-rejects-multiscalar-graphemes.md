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

## Golden impact

- Every fixture under `tests/rt-behavior/encoding/**`, `tests/rt-error/encoding/**`
  and `tests/byte-identity/encoding/**` that embeds the assembled `encoding`
  package source has its `.ir` golden move, because `Body::mfb` injects
  `__encoding_codepageEncode`'s source verbatim into every program that imports
  `encoding`, and every *other* function's line numbers in that assembled file
  shift along with it (`.ai/compiler.md`: editing one member's description/body
  shifts line numbers in every `.ir` golden of that package). Measured:
  `scripts/test-accept.sh target/release/mfb '*' 'func_encoding_codepage*'` →
  4 `.ir` mismatches (`func_encoding_codepageEncode_rt`,
  `func_encoding_codepageDecode_rt`, `func_encoding_codepageEncode_unrepresentable`,
  `func_encoding_codepageDecode_unmapped`), all pure content/line-number diffs
  in the injected helper body — their `.ast`/`.run` goldens (observable output)
  are byte-identical, and the two `syntax/encoding/*_invalid` fixtures have no
  mismatch at all.
  `scripts/artifact-gate.sh target/release/mfb encoding` → 6/7 mismatches, all
  `encoding_codegen_cover_rt` (`.ir` + 5 `.ncode` sums); no other diff.
- `tests/rt-error/encoding/func_encoding_codepageEncode_unrepresentable`'s `.run`
  golden is unaffected even though its one borderline case
  (`"e\u{0301}"` on `Windows1252`) is exactly a multi-scalar grapheme: it still
  raises, because Windows-1252's table has no entry for `U+0301` either way —
  only the *reason* changed (grapheme-width check → per-scalar table miss on the
  combining mark), not the observable "raised" output.

## STATUS: FIXED

`__encoding_codepageEncode` now walks `__encoding_codepoints(text)` (Unicode
scalars) instead of `strings::graphemes(text)`, converting each scalar `>= 128`
back to its one-character form with `__encoding_fromCodepoint` before the
`U+FFFD` check and table search. `U+FFFD` (`65533`) is still rejected before
the search; an unmappable scalar (standalone or inside a larger grapheme) still
raises `ErrInvalidFormat`; decode is untouched.

Regression test: `tests/runtime/rt_codepage_encode_multiscalar.rs` — covers the
one `codepageEncode` overload across all 28 single-byte `Codepage` members
(CRLF round-trip, universal since `\r`/`\n` are both below `U+0080`), the bug
doc's three named combining-mark cases (Windows-1258 a+acute → `61ec`,
Windows-1255 bet+qamats → `e1c8`, ISO-8859-6 alef+fatha → `c7ee`), a `Utf8`
control, and the non-goals (`U+FFFD`, an unmappable scalar, the empty string).
RED on current main (all CRLF cases and all three combining-mark cases raise
`77050003` instead of round-tripping); green after the fix.

Checks run (all under the shared build lock):
- `cargo test --test rt_codepage_encode_multiscalar` → 4 passed (RED before,
  green after).
- `cargo test --bin mfb encoding::` → 16 passed, 0 failed, including
  `codepage_roundtrip_counts_match_the_vendored_index_files`,
  `codepage_tables_match_the_vendored_index_files` and
  `codepage_digests_match_the_vendored_index_files` (the vendored-index
  differential checks).
- `scripts/spec-census.sh --render stdlib` → 0 leaks; `--citations stdlib` →
  0 miss-path/miss-line/miss-symbol.
- `scripts/test-accept.sh target/release/mfb '*' 'func_encoding_codepage*'` →
  see Golden impact above (4 `.ir`-only mismatches, no `.ast`/`.run` diff).
- `scripts/artifact-gate.sh target/release/mfb encoding` → 6/7 mismatches, all
  `encoding_codegen_cover_rt` (expected codegen drift from the body rewrite).

Spec updated in the same change: `src/docs/spec/stdlib/08_encoding.md`
§Legacy single-byte codepages' "Encoding" bullet no longer claims a
multi-scalar grapheme is categorically unrepresentable, and
`encoding::codepageEncode`'s own descriptor prose (`DESC` in
`func_codepage_encode.rs`) is corrected the same way.
