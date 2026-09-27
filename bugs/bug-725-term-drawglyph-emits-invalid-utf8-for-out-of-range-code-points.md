# bug-725: `term::drawGlyph` accepts out-of-range code points and surrogates, emitting invalid UTF-8

Last updated: 2026-09-19
Effort: small
Severity: MEDIUM
Class: Correctness (invalid UTF-8 written to the terminal)

Status: Open
Regression Test: none yet — see Phase 1

`term::drawGlyph(row, column, codePoint)` checks only that the code point is at
least 32. Anything larger is encoded and written as-is, so a value above the
Unicode maximum (U+10FFFF) or inside the surrogate range (U+D800–U+DFFF) puts
byte sequences on the terminal that are not valid UTF-8.

Captured from a real pty (raw bytes, `/tmp/p125-ex/termglyph`):

| Call | Bytes written | Valid UTF-8? |
|---|---|---|
| `drawGlyph(0, 0, 1114112)` (U+110000, one past the maximum) | `f4 90 80 80` | no |
| `drawGlyph(2, 0, 55296)` (U+D800, a surrogate) | `ed a0 80` | no |
| `drawGlyph(1, 0, 65)` | `41` (`A`) | yes |

Every other text path in the language is UTF-8-safe: `String` holds well-formed
UTF-8, and `strings::fromScalars` rejects a value that is not a Unicode scalar.
`drawGlyph` is the one place that writes whatever integer it is handed.

**The single correct behavior a fix produces:** `term::drawGlyph` writes only
well-formed UTF-8. A code point above U+10FFFF or in the surrogate range is
rejected the way the rest of the language rejects a non-scalar — raising
`ErrInvalidArgument` — or is skipped exactly as a control value below U+0020 is
today. Whichever is chosen, the page states it, and no call can put invalid UTF-8
on the terminal.

Found by plan-125-D Phase 3 while triaging the `term` overview review.
**Filed, not fixed**, by user instruction during a documentation-only plan ("file
all bugs, make no fixes").

## Reproduction

```basic
IMPORT term

SUB main()
  term::on()
  term::drawGlyph(0, 0, 1114112)
  term::drawGlyph(2, 0, 55296)
  term::sync()
  term::off()
END SUB
```

Run it on a pty and capture the raw bytes:

```
$ python3 -c "...pty.fork(), read the master fd..."   # /tmp/p125-ex/termglyph
U+110000 as f4908080: True
surrogate ed a0 80: True
```

Expected: neither sequence reaches the terminal.

## Root cause

`src/codegen/term/core/term.rs:emit_draw_glyph` guards with a single `codePoint < 32`
test and then encodes the value with the generic UTF-8 emitter, which happily
produces four-byte forms above U+10FFFF and three-byte forms in the surrogate range.
The Unicode-scalar range is never checked.

## Non-goals

- Changing how code points below U+0020 are handled (skipped today).
- Changing `drawGlyph`'s signature to take a `Scalar` — that is a language-surface
  change, not this fix (note it if the fix makes it attractive).
- Validating text drawn by `term::drawText`, which comes from a `String` and is
  already well-formed.

## Blast-radius audit

- `term::drawText` and the `astrings` bridge: confirm they cannot pass an unchecked
  code point to the same emitter.
- The macOS and Linux GTK app term backends: check whether they encode the glyph
  themselves, and whether an invalid code point corrupts the grid rather than the
  output stream.
- `canvas` text drawing, if it shares the encoder.

## Fix

Phase 1 — RED test drawing U+110000 and U+D800 and asserting the emitted bytes are
valid UTF-8 (or that the call raises). Commit:

Phase 2 — range-check the code point and do what the page states; re-render the
`term` pages; full suite and term runtime goldens. Commit:
