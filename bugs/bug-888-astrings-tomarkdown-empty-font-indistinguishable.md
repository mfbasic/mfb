# bug-888: `astrings::toMarkdown` renders an empty font name (documented as allowed) exactly like an omitted slot, so `font("")` + size reads as "font unchanged"

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness (format ambiguity; owner chose (a))

Status: Open
Regression Test: tests/rt-behavior/astrings/tomarkdown-fontsize-rt (extend)

**Owner decision (2026-10-05):** an empty font name is treated the same as an unknown
font name such as `font("sdgf")`. It is an ordinary stored name with no special
meaning: it is accepted (not `ErrInvalidArgument`, so option (b) is rejected), it is not
a reset, and `toMarkdown` must preserve it as distinctly as it preserves `sdgf`. The
fix is option (a): give the empty name a spelling that cannot be mistaken for an
omitted slot.

`mfb man astrings font`: "name … The font family name, stored as given; **it may be
empty**." The `toMarkdown` marker vocabulary in the spec says "a value sets, `-`
resets to default, and an omitted slot is left unchanged (`::font::` font-only,
`::;size::` size-only)". An empty font name escapes to the empty string, so the
marker for "set font to `""` and size to 12" is `::;12::`. That is byte-identical to
"size 12, font unchanged". `font("")` alone renders as `::::`. The output is lossy:
the toolchain that reads this format (the spec says it "is read by the `astrings`
toolchain") cannot recover the attribute.

Correct behavior (owner decision): either (a) the empty font name gets an escape of
its own, the same treatment `-` already gets (e.g. `\` followed by nothing, or a
reserved spelling), or (b) `font("")` is rejected with `ErrInvalidArgument` and the
man page drops "it may be empty".

References:

- src/docs/spec/stdlib/15_astrings.md §toMarkdown: "Font/size switch forward via a minimal-delta `::font;size::` marker … an omitted slot is left unchanged (`::font::` font-only, `::;size::` size-only, `::-::` font reset)."
- `mfb man astrings font`.

## Failing Reproduction

```
IMPORT astrings
IMPORT io
FUNC main AS Integer
  MUT a AS AttributedString = astrings::fromString("ab")
  a = astrings::addAttribute(a, 0, 0, astrings::font(""))
  a = astrings::addAttribute(a, 0, 0, astrings::fontSize(12))
  io::print("[" & astrings::toMarkdown(a) & "]")
  MUT b AS AttributedString = astrings::fromString("ab")
  b = astrings::addAttribute(b, 0, 0, astrings::fontSize(12))
  io::print("[" & astrings::toMarkdown(b) & "]")
  MUT c AS AttributedString = astrings::fromString("ab")
  c = astrings::addAttribute(c, 1, 1, astrings::font(""))
  io::print("[" & astrings::toMarkdown(c) & "]")
  RETURN 0
END FUNC
```

(`/tmp/hardening/B06/as2`)

- Observed:
  ```
  [::;12::a::-;-::b]
  [::;12::a::;-::b]
  [a::::b]
  ```
  The first marker of line 1 (font "" + size 12) equals the first marker of line 2
  (size only). Line 3 sets a font that the reader sees as an empty `::::` marker.
- Expected: line 1's opening marker is distinguishable from line 2's.

## Root Cause

`src/codegen/builtins/astrings/helper_md_escape_font.rs:__astrings_mdEscapeFont`
special-cases only `s = "-"` (so a literal `-` font is not read as a reset) and
returns `""` for `""`. `src/codegen/builtins/astrings/func_to_markdown.rs` then emits
`"::" & fpart & ";" & spart & "::"`, so an empty `fpart` is the omitted-slot spelling.

## Goal

- Every distinct resolved font/size state transition renders to a distinct marker.

### Non-goals (must NOT change)

- Existing markers for non-empty fonts (tomarkdown-fontsize-rt golden).

## Blast Radius

- `toMarkdown` only.
