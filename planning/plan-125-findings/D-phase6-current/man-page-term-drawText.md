All probes are done. Writing up the findings.

### 1. "A stray control character can never corrupt the presented frame" is overclaimed
UNIT:      man-page:term/drawText
CLAIM:     "Control characters (below U+0020, including newline and tab) are skipped — they advance one column but stamp nothing — so a stray control character can never corrupt the presented frame"
VERDICT:   misleading
EVIDENCE:  Only bytes below 0x20 are filtered (`src/codegen/term/core/term/text.rs:emit_draw_text`, the `compare_immediate(b0, "32")` / `branch_lt(&skip_ctrl)` pair). U+007F and the C1 controls U+0080..U+009F go straight into the escape stream. Probe `edge/src/main.mfb`, `term::drawText(5, 0, "a\u{7f}b")`, then parsing the `term::sync` output: row 6 is `'a\x7fb'`, the DEL byte emitted raw and charged one column, so a real terminal (which does not advance on DEL) shows `b` one cell left of where the surface believes it is. Probe `c1/src/main.mfb`, `term::drawText(0, 0, "a\u{9b}b")`: the output contains the bytes `61 C2 9B 62`, and U+009B is the C1 CSI introducer, which a UTF-8 terminal interprets as the start of an escape sequence. The parameter description repeats the same "below U+0020" claim. This is also a likely code bug for the main thread: the filter should cover U+007F and U+0080..U+009F (and `term::drawGlyph` shares the `< 0x20` guard).
SUGGESTED: "Characters below U+0020 (including newline and tab) are skipped — they advance one column but stamp nothing. U+007F and the C1 controls U+0080..U+009F are not filtered and are sent to the terminal as-is, where they can shift or corrupt the frame; strip them first. Use io::write for flowing text with newline handling." (Or, if the main thread fixes the filter, keep the original sentence and say "control characters, U+0000..U+001F and U+007F..U+009F".)

### 2. The text parameter says nothing about an empty string
UNIT:      man-page:term/drawText
CLAIM:     "The text to stamp, one grid position per grapheme cluster (a double-width cluster takes two columns)."
VERDICT:   incomplete
EVIDENCE:  §2 requires every parameter to state what happens at empty. Probe `edge/src/main.mfb`, `term::drawText(9, 0, "")`: nothing drawn, no error, exit 0 (`emit_draw_text` branches to `inactive` when the remaining length is 0).
SUGGESTED: Append: "An empty string draws nothing and is not an error."

### 3. The AttributedString overload silently drops colour alpha
UNIT:      man-page:term/drawText
CLAIM:     "the attributes the terminal surface can represent — bold, underline, foreground colour and background colour — are applied per run"
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/term/helper_astrings_bridge.rs`, `__term_applyFg`/`__term_applyBg`, hand the packed colour to `term::setForeground`/`setBackground`, which read only the colour channels; the bridge comment states "THE TERMINAL HAS NO ALPHA AND IGNORES IT, deliberately". The emitted SGR in `ex2.txt` is `38;2;R;G;B` with no alpha channel. A reader who built the attribute from a translucent `color::Color` gets opaque output with no warning on this page.
SUGGESTED: Add after the sentence: "A foreground or background colour is applied without its alpha: a half-transparent colour draws exactly as the opaque one does."

Audit note on everything else. Both examples compile with `mfb build .` in a project wrapper. Example 2 runs to exit 0 and the sync output shows `ESC[1m` around `Save` and `ESC[4m` around `Quit` at row 1 column 3. Example 1 aborts with ErrUnsupported from `term::terminalSize` when stdout is not a terminal, which is that function's documented contract, not a drawText defect; run inside a 24x80 pseudo-terminal (`pty2.py`) it exits 0 with the title at row 1 column 3 and the status line at row 24 column 1. Edge probe `edge/src/main.mfb` confirmed: negative column skips (`"abcdef"` at column -2 shows `cdef` at column 0, and `left/` shows a wide glyph at column -1 leaves column 0 untouched with the next character at column 1); a wide cluster with one column left is dropped and the run stops (`x` at 78, 世 absent); tab and newline each take one blank cell; e plus combining acute is one cell; rows -1 and 24 draw nothing; a ZWJ family emoji stays one cluster. Probe `cur/` confirmed the cursor stays at the `moveTo` position (final `ESC[6;8H`) after drawText. Probes `gate/` and `gate2/` confirmed both overloads do nothing and raise nothing while TUI mode is off. The `--app` ErrWrongMode claim was read in `gen_shared.rs:lower_term_helper` (`prepend_wrong_mode_gate` with `ModeRequirement::Console`) but not probed, since it needs an app build; UNVERIFIED by probe. A grep of the rendered page against `scripts/man-census.sh --banned-list` returned no hits, and no sentence teaches an ownership model.
