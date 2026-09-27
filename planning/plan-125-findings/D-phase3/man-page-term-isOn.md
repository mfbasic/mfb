Verification is complete. Both examples compile and run, and every behavioural claim held up under probes. Findings follow.

### 1. Gated-call list omits the two cursor calls
UNIT:      man-page:term/isOn
CLAIM:     "Every other term:: call short-circuits while TUI mode is off: the setters, term::clear, term::moveTo, term::sync and all six drawing calls (drawHLine, drawVLine, drawBox, fillRect, drawText, drawGlyph) do nothing, term::getForeground/getBackground/getBold/getUnderline return inert defaults rather than live state, and term::terminalSize raises ErrUnsupported."
VERDICT:   incomplete
EVIDENCE:  The list is presented as exhaustive ("Every other", "all six") but never names `term::hideCursor` and `term::showCursor`, which are gated the same way: `src/codegen/term/core/term.rs:emit_set_cursor_visible` opens with `emit_gate_inactive`, and `mfb man term hideCursor` says "The call is gated: while TUI mode is off it does nothing". Probe `probe3.bas` called both while off and ran to completion with no effect and no error. A reader counting "the setters" as the four colour/attribute setters will not know the cursor pair is covered.
SUGGESTED: "...the colour and attribute setters, term::hideCursor, term::showCursor, term::clear, term::moveTo, term::sync and all six drawing calls (...) do nothing, ..."

### 2. "the module's active flag read directly" is implementation phrasing
UNIT:      man-page:term/isOn
CLAIM:     "The result is the module's active flag read directly, so it changes only at term::on and term::off."
VERDICT:   out-of-scope
EVIDENCE:  `.ai/man-content.md` §1/§3: a sentence that only makes sense against the implementation belongs in the spec. "The module's active flag" and "read directly" describe the term-state slot loaded by `emit_is_on` (`src/codegen/term/core/term.rs`); the developer-observable fact is only the second half of the sentence. Probe `probe1.bas` (built with `mfb build`, run with stdin from /dev/null) confirms the observable part: before → FALSE, off-while-off → FALSE, after on → TRUE, on twice → TRUE, after off → FALSE, off twice → FALSE.
SUGGESTED: "The answer changes only at term::on and term::off. term::off while already off leaves it FALSE; term::on while already on resets the surface again but the answer stays TRUE throughout."

### 3. Second example prints nothing when run as written
UNIT:      man-page:term/isOn
CLAIM:     "Draw only when the surface is live:" followed by the `IF term::isOn() THEN ... term::sync()` program.
VERDICT:   misleading
EVIDENCE:  Copied verbatim to `q_ex2/src/main.mfb`, `mfb build -q q_ex2` succeeds and `./q_ex2/build/q_ex2.out` exits 0 with zero bytes of output, because nothing in the program ever calls `term::on`. A reader who runs the page's example sees no "status" and no evidence the guard did anything. The first example has the same shape issue in reverse: it enters TUI mode and never leaves (the runtime restored the screen at exit in my run, so it is harmless, but the page does not say so).
SUGGESTED: Make the second example self-contained so the guard is visibly exercised, e.g. a `SUB drawStatus()` holding the guarded body, called once before `term::on()` (no output) and once after (draws "status"), ending with `term::off()`. Alternatively keep the fragment but state above it that it is a helper meant to be called from code that may or may not have entered TUI mode.

No other findings. Every remaining sentence was checked: `emit_is_on` is a single load plus an OK tag, so "reads state only ... and it cannot fail" is true and the descriptor's empty error list matches; `probe2.bas` confirmed `term::terminalSize` raises `ErrUnsupported` while off (trapped and matched against `errorCode::ErrUnsupported`); `probe3.bas` confirmed `getForeground` returns white, `getBold` returns FALSE, `setBold(TRUE)` while off leaves `getBold` FALSE, and `didResize` returns FALSE while off. No banned memory vocabulary appears on the page. App-mode behaviour of `isOn` (`emit_app_term_is_on` in the GTK and Win64 app backends) was not probed and is UNVERIFIED, but the page makes no app-mode claim.
