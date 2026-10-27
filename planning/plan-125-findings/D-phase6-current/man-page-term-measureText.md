### 1. The `AttributedString` overload is missing from the page
UNIT:      man-page:term/measureText
CLAIM:     "`term::measureText(value AS String) AS Integer`" (the Declaration, and the parameter table that lists only `value AS String`)
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/term/helper_astrings_bridge.rs:__term_measureTextAttr` and `src/codegen/builtins/term/mod.rs:MEASURE_TEXT` route `term::measureText(AttributedString)` when both `term` and `astrings` are imported; the spec confirms it (`src/docs/spec/app/04_term-backend.md`: "It accepts `String`, or `AttributedString` when `astrings` is imported; attributes do not affect the answer"). Probe `/tmp/plan-125-scratch/D-phase6-current/man-page-term-measureText/attr/src/main.mfb` (`IMPORT astrings`, `term::measureText(astrings::fromString("日本語x"))`) built with `mfb build -q attr` and printed `attr=7`. The page never mentions the overload, so a developer measuring attributed text has no way to learn it works or that styling does not change the count.
SUGGESTED: Add to the description: "When `astrings` is imported, `value` may also be an `AttributedString`; the answer is the width of its visible text and its attributes never change it, so a styled label measures the same as `toString(value)`."

### 2. Control characters count one cell each, and the page does not say so
UNIT:      man-page:term/measureText
CLAIM:     "The empty string measures `0`; ordinary characters measure one cell; East Asian wide and fullwidth characters generally measure two"
VERDICT:   incomplete
EVIDENCE:  Probe `probe/src/main.mfb` printed `a-nl-b=3`, `tab=1`, `esc[31m=5` (a newline, a tab, and ESC each measure one cell; an SGR escape sequence measures five). This follows the lowering (`func_measure_text.rs:measure_text_of`: a cluster whose scalars all have width zero contributes one), and it matches `term::drawText`, whose page explicitly says control characters "advance one column but stamp nothing". The measureText page lists the zero-width rule only for "zero-width scalars", which a reader does not naturally take to include newline, tab or escape sequences. A developer measuring a string that still carries a trailing newline or an ANSI colour code gets a count that is too large with no warning.
SUGGESTED: Append after the wide-character sentence: "Control characters, including newline, tab and escape sequences, are not stripped: each one measures one cell, exactly as `term::drawText` advances one column and draws nothing for it. Strip them before measuring."

### 3. The combining/emoji sentence names the cluster but not its width
UNIT:      man-page:term/measureText
CLAIM:     "combining sequences and joined emoji are each one grapheme cluster."
VERDICT:   misleading
EVIDENCE:  Every other clause in that sentence states a cell count; this clause does not, and the two cases it joins measure differently. Probe printed `e+combining=1` (`"e"` plus U+0301), `e-acute-nfc=1`, `zwj-family=2` (`"👨‍👩‍👧"`), `emoji=2` (`"😀"`). Lowering: a cluster contributes its first non-zero scalar width (`measure_text_of`), so a base-plus-marks sequence measures as its base and a ZWJ emoji sequence measures as its first emoji. A reader who takes "one grapheme cluster" to mean "one cell" gets the family emoji wrong by one.
SUGGESTED: "a base character plus its combining marks measures as the base alone (`e` plus a combining acute is `1`), and an emoji sequence joined with zero-width joiners measures as its first emoji (a family emoji is `2`) — each is one grapheme cluster, counted once."

### 4. Which error wins when the mode is wrong AND the terminal is off is unstated
UNIT:      man-page:term/measureText
CLAIM:     "It raises `ErrUnsupported` before `term::on()` and after `term::off()`. In an app build it raises `ErrWrongMode` unless the current mode is `app::Mode.Console`."
VERDICT:   incomplete
EVIDENCE:  In an app build in `app::Mode.None` before `term::on()`, both sentences apply and the page does not say which error is raised. `src/codegen/app/hook/app.rs:prepend_wrong_mode_gate` splices the mode check at function entry, ahead of the active-terminal check in `lower_measure_text`; the spec states the order (`src/docs/spec/unicode/02_strings-model.md`: "the mode check precedes the active-surface check"). Probe `appmode/src/main.mfb` built with `mfb build -q --app appmode` and run from `appmode.app/Contents/MacOS/appmode` printed `mode=None` then `None, before on: raised 77050020` (`ErrWrongMode`, not `ErrUnsupported`).
SUGGESTED: "In an app build the mode is checked first: outside `app::Mode.Console` it raises `ErrWrongMode` whether or not the terminal is on. In `app::Mode.Console`, and in every non-app build, it raises `ErrUnsupported` before `term::on()` and after `term::off()`."

### 5. Flag emoji measure one cell, not two
UNIT:      man-page:term/measureText
CLAIM:     "A cluster made entirely from zero-width scalars still measures one cell, matching the terminal drawing surface."
VERDICT:   incomplete
EVIDENCE:  Probe printed `flag=1` for `"🇯🇵"` and `lone-regional=1` for a single regional indicator. A regional-indicator pair is one cluster whose scalars have width 1 in the embedded width table, so the pair measures one cell, while `"😀"` measures `2`. The count does match what `term::drawText` advances (same `measure_text_of` walk), so the sentence is true of MFBASIC's surface, but most terminal emulators render a flag two cells wide, so a layout built on this count overlaps the next cell on screen. The page lists wide, fullwidth and joined emoji but says nothing about flags, the one common emoji case that goes the other way.
SUGGESTED: Add: "A country flag built from two regional-indicator letters measures one cell, the same as `term::drawText` advances for it, even though many terminals draw it two cells wide; leave a spare cell after one if the layout must not overlap."
