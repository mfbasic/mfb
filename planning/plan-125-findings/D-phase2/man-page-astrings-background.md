### 1. `fromPacked` cannot take the returned attribute directly
UNIT:      man-page:astrings/background
CLAIM:     “That is `color::toPacked`'s order, so `color::fromPacked` reads the attribute back and the colour round-trips exactly, alpha included.”
VERDICT:   misleading
EVIDENCE:  `/tmp/plan-125-scratch/D-phase2/man-page-astrings-background/direct-from-packed/src/main.mfb` calls `color::fromPacked(astrings::background(color::rgb(1, 2, 3)))`; the release binary rejects it with `TYPE_CALL_ARGUMENT_MISMATCH`: argument type `astrings.Attribute`, expected `Integer`. `src/codegen/builtins/astrings/func_background.rs:BODY` puts the packed integer in `AttrNumber.value`.
SUGGESTED: Match the returned attribute as `astrings::AttrNumber` to read its `value`; `color::fromPacked(value)` then reconstructs the original colour exactly, including alpha.

### 2. `drawText` coordinates are documented in the wrong order
UNIT:      man-page:astrings/background
CLAIM:     “When such an AttributedString is drawn with `term::drawText(x, y, value)` (both term and astrings imported), the colour is emitted as a truecolor background.”
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/term/helper_astrings_bridge.rs:127` declares `__term_drawTextAttr(row AS Integer, column AS Integer, value AS AttributedString)`; `src/codegen/builtins/term/func_draw_text.rs` documents the public order as `row`, then `column`. Conventional `x, y` implies the opposite order.
SUGGESTED: When such an `AttributedString` is drawn with `term::drawText(row, column, value)` (with both `term` and `astrings` imported), its background colour is emitted as truecolor.