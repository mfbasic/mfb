### 1. Right-edge wide glyph: the macOS app backend drops it, the page says the first cell is drawn
UNIT:      man-page:term/drawGlyph
CLAIM:     "A scalar the terminal draws double-width also occupies the following column; at the right edge only its first cell is on the surface."
VERDICT:   misleading
EVIDENCE:  Console: probe `term::drawGlyph(1, size.columns - 1, 20013)` in a 10x40 pty (`script -q pr.log ...`, `cat -v pr.log`) emits `中` at column 40 of row 2, so the first cell is drawn. Linux GTK app: `src/target/linux_gtk/term_draw.rs:emit_term_glyph_helper` comment "a wide glyph in the last column simply loses its sentinel rather than wrapping" (first cell stamped). macOS app: `src/target/macos_aarch64/app/term_view.rs:app_stamp_cluster` — "A wide cluster needs col+1 on the grid, else it is dropped (never split)": `compare_registers(tmp, cols); branch_ge(skip)` before any stamp, so nothing is drawn. Read from source; an `--app` build cannot be run in this sandbox.
SUGGESTED: "A scalar the terminal draws double-width also occupies the following column. In the last column there is no room for it: a terminal and the Linux and Windows app backends draw its first cell, while the macOS app backend draws nothing."

### 2. DEL and the C1 controls are not skipped; they go into the frame raw
UNIT:      man-page:term/drawGlyph
CLAIM:     "Code points below U+0020, including negative ones, are skipped — they would corrupt the presented frame — and so is any value that is not a Unicode scalar"
VERDICT:   incomplete
EVIDENCE:  `src/codegen/term/core/term/text.rs:emit_draw_glyph` checks only `cp < 32`, `cp > 1114111` and the surrogate band. Probe `drawGlyph(2, 1, 127)` and `drawGlyph(2, 2, 128)` in the pty log show `^?` (0x7F) and `M-BM-^@` (UTF-8 for U+0080) written straight into the escape stream, each occupying a cell. Those are control characters too (U+009B is a CSI introducer on many terminals), so the page's stated rationale does not hold for them. This is an implementation gap for the main thread to decide on, not a doc-only fix.
SUGGESTED: Either extend the skip to U+007F and U+0080–U+009F and keep the sentence, or add: "U+007F and the U+0080–U+009F controls are not filtered; they are sent to the terminal as-is and may disturb the frame."

### 3. A zero-width scalar takes a whole cell on its own
UNIT:      man-page:term/drawGlyph
CLAIM:     "`term::drawGlyph` stamps the scalar named by `codepoint` into the cell at `row`/`column`"
VERDICT:   incomplete
EVIDENCE:  `text.rs:emit_draw_glyph` comment "a zero-width glyph (a lone combining mark) falls back to width 1" and the code `compare_immediate(width,"0"); branch_ne; move_immediate(width,1)`. Probe `drawGlyph(2, 7, 769)` (U+0301 combining acute) emits `M-LM-^A` as its own cell at row 3 column 8 in the pty log; it does not attach to the glyph in column 7.
SUGGESTED: Add after the double-width sentence: "A combining mark or other zero-width scalar is placed in the cell by itself, one column wide; it does not combine with a neighbouring glyph."

### 4. A macOS terminal draws this call too, not only a Linux one
UNIT:      man-page:term/drawGlyph
CLAIM:     "A Linux terminal and all three app backends — macOS, Linux and Windows — draw this call."
VERDICT:   misleading
EVIDENCE:  The example and probe were built as plain (non-app) programs on Darwin 24.6 with the release binary and run under `script` in a 10x40 pty; the example log shows `☃` at row 6, column 21, and the probe shows `A`, `☃`, `中` in place. The console emitter is shared, not Linux-specific (`src/codegen/builtins/term/gen_shared.rs:lower_term_helper`).
SUGGESTED: "A terminal on every platform and all three app backends — macOS, Linux and Windows — draw this call."

### 5. ErrWrongMode row could not be exercised
UNIT:      man-page:term/drawGlyph
CLAIM:     "In an `mfb build --app` program, `term::drawGlyph` — like every other `term::` call — raises `ErrWrongMode` unless the program is in `app::Mode.Console`"
VERDICT:   UNVERIFIED
EVIDENCE:  An `--app` build cannot be launched headless in this sandbox. The descriptor declares `errors: vec!["ErrWrongMode"]` (`src/codegen/builtins/term/func_draw_glyph.rs:register`) and the app dispatch lives in `gen_shared::lower_term_helper`, which I read but did not run.
SUGGESTED: No wording change; main thread may confirm with an `--app` build in `app::Mode.None`.

Everything else checked out. Probe (`/tmp/plan-125-scratch/D-phase6-current/man-page-term-drawGlyph/probe/src/main.mfb`): a call before `term::on()` is a silent no-op; off-grid rows and columns (negative, equal to the size) draw nothing and exit 0; negative, U+D800, and U+110000 are skipped while U+10FFFF is drawn; the surrogate band is inclusive at both ends (`branch_lt 55296` / `branch_le 57343`); the emitter never writes the cursor position. The example compiles with `mfb build -q .` and draws the snowman at the surface centre. No banned memory vocabulary and no compiler-internal sentences on the page.
