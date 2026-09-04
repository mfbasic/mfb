# audit-3 — Surface 5: untrusted-data decoders in emitted programs

Part of `planning/goal-08-platform-security-review.md`. Finding prefix `DEC-`
(text: encoding/json/csv/regex, DEC-01..12; media: PNG/inflate/font/MML,
DEC-50..60). Untrusted party: the author of any data file or byte stream a
compiled MFBASIC program decodes.

**Verdict: 8 HIGH · 3 MEDIUM · rest LOW. No memory corruption on this surface.**
Every decoder here is MFBASIC source — reads go through `collections::getOr` and
integer `*` traps on overflow — so the entire finding set is
denial-of-service (decompression bombs / super-linear blowup / amplification),
not OOB. The uniform defect is the **absence of any cap** on a file-derived size.
The memory-safety "usual suspects" were checked and are actually handled (see
Negatives).

## HIGH — text (bug-510)

- **DEC-01** — regex matcher charges recursion depth per matched atom:
  `^([a-z0-9-]+\.)+[a-z]{2,}$` raises at 54 labels (162 B); caps any pattern at 300
  sequential atoms (`regex/helper_match_cont.rs:20`).
- **DEC-02** — backtracking budget resets per match → `findAll`/`replace` cost is
  `matches × budget`; 368 B → 16.9 s CPU (`regex/helper_search_from.rs:14`).
- **DEC-03** — json/regex/csv materialize the whole input as a per-element
  collection; **lead-reproduced**: 1.2 MB JSON → ~1.05 GB RSS (~875×)
  (`json/func_parse.rs:211`). Spike: `spikes/audit-3/DEC-03/`.

## HIGH — media (bug-509)

- **DEC-50** — PNG `width*height*4` allocated from IHDR with no max, before the
  data-present check; 69 B → 4.95 GB (`canvas/helper_png.rs:397-405`).
- **DEC-51** — inflate has no output cap (zlib bomb) and reports success; 389 KB →
  25 GB (`canvas/helper_inflate.rs:291`).
- **DEC-53** — glyph raster `w*h` unbounded from font `unitsPerEm`/coords, bypasses
  the 1 MiB budget; 2-byte font mutation → 62.7 s/7.57 GB for one letter
  (`canvas/helper_glyph_cache.rs:174-184`).
- **DEC-54** — `cmap` format 12 `numGroups` is a raw u32 driving an unbounded scan;
  583 s CPU to draw one char (`canvas/helper_font.rs:203-205`).
- **DEC-55** — MML `{ … }<count>` repeat count unbounded and nests
  multiplicatively; 15-char tune → 38 GB (`audio/helper_mml_expand.rs:34-56`).

## MEDIUM

- **DEC-04** — `json::parse` tokenizes by grapheme cluster → rejects every
  CRLF-formatted JSON doc and any string starting with a combining mark
  (`json/func_parse.rs:211`). (Correctness, but a smuggling-adjacent parser
  divergence.)
- **DEC-05** — `punycodeDecode` O(n²) + no 63-octet cap; 32 KB → 32.75 s
  (`encoding/helper_puny_decode_label.rs:82`).
- **DEC-07** — hex escapes delegate to `toInt(_,16)` which accepts a sign:
  `"\u+041"` → `"A"`; regex `\x{-41}` → U+0000 (`json/helper_parse_hex_quad.rs:18`).
- **DEC-52** — `__canvas_pngSlice` copies the accumulator per chunk → quadratic +
  arena-leaking multi-IDAT; 400 KB → 2.47 GB (`canvas/helper_png.rs:56-57`).

## LOW

DEC-06 (no RFC 3492 overflow check; wrong error code), DEC-08 (uleb128 drops top 6
bits of byte 10, accepts padding, `i64::MIN` from unsigned), DEC-09 (base64/base32
accept non-zero trailing bits), DEC-10 (embedded NUL in decoded strings — `fs`
rejects at the sink, other sinks unverified), DEC-11 (two Unicode versions in one
binary: `\p{gc}` = pinned 16.0.0 vs case/NFC = utf8proc 17.0.0), DEC-12 (regex
compile size-unbounded), DEC-56 (font table offsets never checked vs file), DEC-57
(PNG CRC / zlib Adler never verified), DEC-58 (inflate accepts over-subscribed
Huffman trees), DEC-59 (glyph cache key truncates gid to 16 bits), DEC-60
(`audio::render` `noteFrames` unbounded).

## Negatives (checked and handled — recorded so a later audit does not re-derive)

**No memory corruption on this surface.** Verified handled: UTF-8/16/32 strict
(overlongs, surrogates, >U+10FFFF rejected — twice); JSON depth cap; regex
parser/matcher caps; CSV quote state machine; LEB128 shift bound; PNG palette index
bounds-checked; bit-depth/colour-type table enforced; chunk length vs file checked;
stored-block LEN/NLEN checked; inflate distance-vs-produced checked; invalid
length/dist symbols (286/287, 30/31) rejected; Adam7 pass-0 of 1×1 correct;
composite glyphs unsupported (no recursion). **There is no runtime utf8proc FFI** —
utf8proc data is compiled into emitted tables at build time, so no
pointer/length/sentinel/free question arises; the constant-fold oracles agree with
the runtime tables on all 2,981 case-mappable scalars (mismatches=0).

## Bug docs filed

bug-509 (media bombs DEC-50..55), bug-510 (text DoS DEC-01/02/03/05). Spikes:
`spikes/audit-3/{DEC-03,DEC-50,DEC-51,DEC-55}/` with generators under
`spikes/audit-3/gen/`.

## Coverage

Read: `builtins/encoding/**`, `builtins/json/**`, `builtins/csv/**`,
`builtins/regex/**`, `canvas/helper_{png,inflate,font,glyph,glyph_cache}.rs`,
`audio/helper_mml_*.rs`/`func_render.rs`. All measurements macOS-aarch64.

Gaps: `astrings/` attribute-span decode (3 of 59 files); `json` accessors /
`stringify`; regex `\p{Script}` lookup cost; `canvas::createImage` pixel-count
validation and `func_measure_text.rs` (same unvalidated `head`/`hhea` on the worker
thread); the embedded-NUL string (DEC-10) against `process`/`net`/`term` sinks.
