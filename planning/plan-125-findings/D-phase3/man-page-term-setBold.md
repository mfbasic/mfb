### 1. Implementation-state vocabulary on a developer page
UNIT:      man-page:term/setBold
CLAIM:     "The flag is stored in the module's current-attribute state and no escape sequence is emitted."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/term/core/term.rs:emit_set_attr` (doc comment "store the flag into its term-state slot ... Emits no ANSI") is the source of this sentence; it is a backend fact. "The module's current-attribute state" and "escape sequence" are console-backend mechanics (§3), and in app mode no escape sequence is ever emitted by anything, so the sentence is also console-specific. The developer-visible fact is already carried by the next sentence ("the change becomes visible only when term::sync presents the frame"), which the pty capture of the example confirms: `script -q /dev/null ./build/ex.out | cat -v` shows nothing until the single sync frame, where "Heading" is preceded by `^[[1m` and "body text" by `^[[0m`.
SUGGESTED: Delete the sentence and start the paragraph at "Like every other drawing operation on this surface, the change becomes visible only when `term::sync` presents the frame." (also drop "retained", a spec term the developer does not need).

### 2. Which drawing calls the attribute applies to is never said
UNIT:      man-page:term/setBold
CLAIM:     "term::setBold sets whether text drawn through the term:: surface from now on is bold."
VERDICT:   incomplete
EVIDENCE:  The example relies on `io::print` honouring the flag, but no sentence says `io::` output is part of "the term:: surface". Pty capture of the example: `^[[1;1H^[[0m^[[1m...Heading` then `^[[2;1H^[[0m...body text`, so `io::print` does take the attribute while TUI mode is on. Spec `src/docs/spec/app/04_term-backend.md` ("Retained double-buffered surface") lists `io::print`/`io::write`, `drawText`, `drawGlyph`, `drawHLine`, `drawVLine`, `drawBox`, `fillRect` as the calls that stamp cells with the current attributes. A developer reading only this page cannot tell whether `io::print` counts.
SUGGESTED: "While TUI mode is on, every call that writes cells takes the current bold setting: `io::print` and `io::write`, and `term::drawText`, `drawGlyph`, `drawHLine`, `drawVLine`, `drawBox` and `fillRect`."

### 3. The AttributedString overload of drawText overrides this flag
UNIT:      man-page:term/setBold
CLAIM:     "The setting persists until the next term::setBold or the next term::on, which resets bold to off."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/term/helper_astrings_bridge.rs` lines 144–152: `term::drawText(row, col, AttributedString)` calls `term::setBold(st.bold)` per style run and then `term::setBold(saveBold)`. So an attributed string is drawn with its own per-scalar bold, not the ambient one, and the ambient value is put back afterwards. A reader who sets bold and then draws an AttributedString gets un-bold runs and the page gives no hint why.
SUGGESTED: Add after the persistence sentence: "One exception: `term::drawText` with an `AttributedString` uses the bold carried by the string for each run, not this setting, and restores this setting when it finishes."

### 4. Readback while off is not stated
UNIT:      man-page:term/setBold
CLAIM:     "The call is gated: while TUI mode is off it does nothing and reports no error."
VERDICT:   incomplete
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase3/man-page-term-setBold/probe/src/main.mfb` (run under `script -q /dev/null ./build/probe.out`) printed: `off/getBold after setBold(TRUE)=FALSE|on/default=FALSE|after TRUE=TRUE|after TRUE twice=TRUE|after setForeground=TRUE|after setUnderline=TRUE|after off=FALSE|after on again=FALSE|`. Every persistence/independence/idempotence claim on the page holds. But "does nothing" plus `term::getBold` returning FALSE after `term::off` (`emit_get_attr` is gated to 0 while inactive) means the value is unobservable while off, and the page's "persists until the next term::on" wording implies it can be seen across an off/on boundary. Also "gated" is jargon; say what happens.
SUGGESTED: "While TUI mode is off the call is ignored and raises nothing; `term::getBold` reports FALSE until the next `term::on`, which starts with bold off."

Probe record: the page example was copied verbatim into `/tmp/plan-125-scratch/D-phase3/man-page-term-setBold/ex/src/main.mfb`, built with `mfb build -q` (exit 0, `Wrote executable to ./build/ex.out`), and run under a pty; the frame showed `Heading` bold and `body text` plain on the next row, matching the page. The Errors table is empty and the descriptor's `errors: vec![]` agrees. No banned memory vocabulary was found on the rendered page.
