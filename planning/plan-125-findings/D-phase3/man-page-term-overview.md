### 1. `drawGlyph` does not require a Unicode scalar
UNIT:      man-page:term/overview
CLAIM:     “term::drawGlyph stamps a single scalar by code point.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/term/core/term.rs:emit_draw_glyph` rejects only values below 32; it does not validate the Unicode-scalar range. Probe `drawGlyph(0, 1, 1114112)` compiled and ran; captured output contained `f4 90 80 80`, an invalid UTF-8 encoding.
SUGGESTED: `term::drawGlyph stamps one glyph from an Integer code point. Values below U+0020 are skipped; use a Unicode scalar from U+0020 through U+10FFFF, excluding surrogate values.`

### 2. No runnable package-overview example
UNIT:      man-page:term/overview
CLAIM:     No example is rendered on this page.
VERDICT:   incomplete
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man term` renders Description, Functions, and See also, but no Examples section. `.ai/man-content.md` §2 requires one compiled-and-run example per page.
SUGGESTED: Add an `Examples` section showing `term::on()`, a draw call, `term::sync()`, and `term::off()`.