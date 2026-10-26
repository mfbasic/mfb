I have everything needed. Findings follow.

### 1. The "three ungated calls" count is wrong
UNIT:      man-page:term/isOn
CLAIM:     "term::on, term::isOn and term::didResize are the three calls in the module that are not gated."
VERDICT:   wrong
EVIDENCE:  `src/codegen/term/core/term/mouse.rs:emit_enable_mouse` doc: "Not gated on `active` … the request simply takes effect." Probe `/tmp/plan-125-scratch/D-phase6-current/man-page-term-isOn/p_probe` (`io::print("A")`, `term::enableMouse(TRUE)`, `io::print("B")`, `term::enableMouse(FALSE)`, `io::print("C")`) run with TUI mode never entered printed `A`, `^[[?1000h^[[?1002h^[[?1006hB`, `^[[?1006l^[[?1002l^[[?1000lC` — the mouse-reporting escapes were written to the terminal while `isOn()` was FALSE. `term::pollMouse()` also ran while off and returned normally (probe printed `A`, `B`, `C`), and `term::measureText` is not gated on `active` either (it raises, see finding 2). At least `enableMouse`, `pollMouse` and `measureText` are not gated, so the count is not three.
SUGGESTED: "`term::on`, `term::isOn`, `term::didResize`, `term::enableMouse` and `term::pollMouse` work regardless of TUI mode; `term::enableMouse` switches mouse reporting on or off even before `term::on`."

### 2. "Every other term:: call short-circuits" is false, and the enumeration omits two members
UNIT:      man-page:term/isOn
CLAIM:     "Every other term:: call short-circuits while TUI mode is off: … do nothing, term::getForeground/getBackground/getBold/getUnderline return inert defaults rather than live state, and term::terminalSize raises ErrUnsupported."
VERDICT:   wrong
EVIDENCE:  Probe `IF term::measureText("日本語") = 6 THEN …` with TUI mode off terminated the program with `Error: 7-705-0007 / Message: Operation is not supported by the implementation or platform. / Location: src/main.mfb:10:6`. So `measureText` raises ErrUnsupported while off, exactly like `terminalSize`, yet the sentence names `terminalSize` as the only raiser and never mentions `measureText`, `enableMouse` or `pollMouse` at all (`ls src/codegen/builtins/term/` lists 27 members; the page's list covers 19 plus the 3 "ungated"). A developer reading this page would expect `measureText` to "do nothing", and it traps instead.
SUGGESTED: "Most other `term::` calls short-circuit while TUI mode is off: the colour and attribute setters, `term::hideCursor`, `term::showCursor`, `term::clear`, `term::moveTo`, `term::sync` and all six drawing calls (`drawHLine`, `drawVLine`, `drawBox`, `fillRect`, `drawText`, `drawGlyph`) do nothing, and `term::getForeground`/`getBackground`/`getBold`/`getUnderline` return inert defaults rather than live state. Two reads have no inert answer and raise `ErrUnsupported` instead: `term::terminalSize` and `term::measureText`."

### 3. First example leaves TUI mode on with no `term::off`
UNIT:      man-page:term/isOn
CLAIM:     Example 1: `IF NOT term::isOn() THEN term::on() END IF` with no matching `term::off`.
VERDICT:   misleading
EVIDENCE:  Compiled and ran `/tmp/plan-125-scratch/D-phase6-current/man-page-term-isOn/p_ex1`: it compiles and exits cleanly (the runtime restored the screen: output ends `^[[?1049l^[[0m`), so this is not broken. But the description's own framing is "after term::on and before the matching term::off", and the example shows an unmatched `on`. The second example does pair them. A reader copying example 1 learns a shape the page itself calls unmatched.
SUGGESTED: Add `term::off()` after the `IF` block, or fold the idiom into the second example and drop the first.

### 4. App-mode ErrWrongMode claim not run
UNIT:      man-page:term/isOn
CLAIM:     "In an mfb build --app program, term::isOn — like every other term:: call — raises ErrWrongMode unless the program is in app::Mode.Console"
VERDICT:   UNVERIFIED (source-consistent)
EVIDENCE:  `src/codegen/builtins/term/gen_shared.rs:lower_term_helper` prepends `prepend_wrong_mode_gate(…, ModeRequirement::Console)` on every app-mode term helper, and `func_is_on.rs` routes through it with `errors: vec!["ErrWrongMode"]`, so the Errors table row is consistent with the code. I did not build an `--app` program (needs a window). No wording change proposed.

Everything else checked out: the state probe printed `before: FALSE`, `off-while-off: FALSE`, `on-while-on: TRUE` (with a second `^[[?1049h…^[[2J` showing `on` while on re-enters and clears the surface, matching "resets the surface again"), `after off: FALSE`, `off again: FALSE`. `emit_is_on` (`src/codegen/term/core/term/mod.rs`) is a single load of the active flag plus an Ok tag, so "reads state only … cannot fail" holds in a console build. Example 2 compiled and ran: the first `drawStatus` produced nothing, the second drew `status` at row 1. No banned vocabulary or ownership-model sentences on the page.
