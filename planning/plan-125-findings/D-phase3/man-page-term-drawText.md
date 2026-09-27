### 1. Parameter overstates control-character handling
UNIT:      man-page:term/drawText
CLAIM:     “Control characters are skipped.”
VERDICT:   wrong
EVIDENCE:  `src/codegen/term/core/term.rs:emit_draw_text` sends input to its control-character path only when the first UTF-8 byte is below `32` (`branch_lt(&skip_ctrl)`); U+007F and C1 controls are not skipped by that condition. The longer description correctly narrows this to “below U+0020,” so the parameter row contradicts the implementation and page.
SUGGESTED: “Control characters below U+0020 are skipped; each advances one column but stamps nothing.”

### 2. Attributed text can split a grapheme cluster
UNIT:      man-page:term/drawText
CLAIM:     “It stamps the same visible text as the String overload but honours the per-scalar styling the value carries:”
VERDICT:   wrong
EVIDENCE:  The probe `/tmp/plan-125-scratch/D-phase3/man-page-term-drawText/src/attr_cluster_probe.mfb` drew plain `é`, then `X` at column 1, and repeated it with bold applied only to scalar 1 (the combining acute). Its captured output rendered the plain row as `éX` but the attributed row as `eX`. `src/codegen/builtins/term/helper_astrings_bridge.rs:__term_drawTextAttr` splits runs at scalar-style boundaries, calls `strings::mid` for each run, and advances by each segment’s `strings::displayWidth`; a style boundary inside one grapheme therefore gives the combining scalar its own grid position.
SUGGESTED: “The overload preserves grapheme handling when every scalar in a grapheme has the same terminal style. A style change within a grapheme splits it into separate runs, which can change its grid positions and visible result.”