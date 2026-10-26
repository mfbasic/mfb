### 1. `isOn` and the four getters do not raise ErrWrongMode outside Console in an --app build
UNIT:      man-page:term/overview
CLAIM:     "In an --app program the character grid exists only in app::Mode.Console: every term:: call made while the program is in app::Mode.None or app::Mode.Canvas raises ErrWrongMode."
VERDICT:   wrong
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase6-current/man-page-term-overview/appgate/src/main.mfb`, built with `mfb build --app .` and run with `MFB_HEADLESS=1 build/appgate.app/Contents/MacOS/appgate`: after `app::setMode(app::Mode.Canvas)` it calls `term::isOn()` (returns 10 on ErrWrongMode), then `term::getForeground()` (returns 20 on ErrWrongMode), then `term::moveTo(0, 0)` (returns 30 on ErrWrongMode). Exit status was **30**: `isOn` and `getForeground` returned normally in Canvas mode and only `moveTo` trapped. Source agrees: `src/target/macos_aarch64/app/app_io.rs:emit_app_term_helper` returns `None` for `isOn` and the attribute getters ("Pure readers — term::isOn and the attribute getters — keep the shared console implementation", `src/docs/spec/app/04_term-backend.md`), and `src/codegen/builtins/term/gen_shared.rs:lower_term_helper` prepends `prepend_wrong_mode_gate` only when the backend returns `Some`, or for the two mouse members on the fall-through. `func_is_on.rs`/`func_get_*.rs` add no gate of their own (`grep -L prepend_wrong_mode_gate src/codegen/builtins/term/func_*.rs` lists all of them). The `term::isOn` page repeats the same wrong claim ("like every other term:: call — raises ErrWrongMode").
SUGGESTED: "In an --app program the character grid exists only in app::Mode.Console: every term:: call that touches the surface, the cursor, the mouse, or the size raises ErrWrongMode while the program is in app::Mode.None or app::Mode.Canvas. The five pure readers — term::isOn, term::getForeground, term::getBackground, term::getBold and term::getUnderline — never raise it; they answer in any mode." (Or make the gate cover them and keep the sentence; either way the page and the binary must agree.)

### 2. "Three calls are exceptions" undercounts: measureText raises, enableMouse acts
UNIT:      man-page:term/overview
CLAIM:     "Three calls are exceptions to the no-op rule. term::isOn and term::didResize are not gated at all and answer either way — term::isOn reports whether TUI mode is on, and term::didResize reports (and clears) a pending resize, reading FALSE before any term::on. term::terminalSize is the third: it raises ErrUnsupported rather than returning a meaningless size."
VERDICT:   wrong
EVIDENCE:  Probe `offmode/src/main.mfb` (no `term::on` anywhere), run as `./offmode/build/offmode.out > offmode.raw`; `cat -v offmode.raw` printed:
```
isOn=FALSE
didResize=FALSE
fg=255,255,255,255
bg=0,0,0,255
bold=FALSE ul=FALSE
measureText off -> raised code 77050007
terminalSize off -> raised code 77050007
BEGIN^[[?1000h^[[?1002h^[[?1006hEND
pollMouse off kind=None button=None
^[[?1006l^[[?1002l^[[?1000lDONE
```
`term::measureText` raises ErrUnsupported (77050007) while TUI mode is off — a fourth exception, confirmed by `src/codegen/builtins/term/func_measure_text.rs` ("raises ErrUnsupported before term::on() and after term::off()"). `term::enableMouse(TRUE)` while off writes the mouse-reporting sequences straight to the terminal (the bytes between BEGIN and END), so the user's terminal starts reporting the mouse before any TUI exists; `src/codegen/term/core/term/mouse.rs:emit_enable_mouse` says so ("Not gated on active … the request simply takes effect"). That is a visible side effect, not a no-op. The getters also answer (with the defaults) rather than doing nothing, which the sentence leaves unsaid.
SUGGESTED: "Some calls are not no-ops while TUI mode is off. term::isOn and term::didResize answer either way — term::isOn reports FALSE and term::didResize reports (and clears) a pending resize, reading FALSE before any term::on. The getters (term::getForeground, getBackground, getBold, getUnderline) answer with the defaults. term::terminalSize and term::measureText raise ErrUnsupported rather than invent an answer. term::enableMouse takes effect immediately, on or off: calling it before term::on hands the mouse to the program at once, so call it after term::on."

### 3. measureText's off-mode behavior is stated as "available only", not as the error it raises
UNIT:      man-page:term/overview
CLAIM:     "It is available only while the Console TUI is active; measure before term::off when the result is needed afterwards."
VERDICT:   incomplete
EVIDENCE:  Same probe as finding 2: `measureText off -> raised code 77050007` (ErrUnsupported). "Available only" does not tell the reader whether the call returns zero, is ignored, or raises; it raises, and that is one of the two conditions behind the page's ErrUnsupported Errors row.
SUGGESTED: "It raises ErrUnsupported while TUI mode is off, before term::on and after term::off; measure before term::off when the result is needed afterwards."

### 4. ErrOutOfMemory row has no condition on the page
UNIT:      man-page:term/overview
CLAIM:     Errors table row "77010001 | ErrOutOfMemory | Allocation failed." with no sentence on the page naming which call raises it.
VERDICT:   incomplete
EVIDENCE:  `grep -n OutOfMemory page.txt` hits only the derived table row. The raisers are `src/codegen/term/core/term/mouse.rs:436` (`raise_error_into(symbol, "ErrOutOfMemory", …)` in `emit_enable_mouse`, when the event ring cannot be created) and `term::terminalSize` ("Besides ErrUnsupported it can raise ErrOutOfMemory while producing its result", `func_terminal_size.rs`). `.ai/man-content.md` §2 requires "every raisable error and the condition that raises it".
SUGGESTED: Add after the terminalSize/measureText sentence: "term::enableMouse(TRUE) and term::terminalSize can raise ErrOutOfMemory when the program cannot get the memory for the event queue or the size record."

### 5. Example verified; no finding
UNIT:      man-page:term/overview
CLAIM:     The page example (term::on / clear / terminalSize / drawBox / drawText / sync / off / io::print).
VERDICT:   (no defect — recorded for audit)
EVIDENCE:  Built from `example/src/main.mfb` with `mfb build .`, run under a 24x80 pseudo-terminal (`python3 -I ptyrun.py ./example/build/example.out example.pty`). Decoded frame: row 1 `┌───…───┐`, row 2 `│ hello … │`, row 24 `└───…───┘`, then alternate screen left and `drew on 80x24` printed. Under `script`'s unsized pty (0x0) the example instead aborts with `Error: 7-705-0007` at `main.mfb:7:32`, which matches the terminalSize page's documented zero-size ErrUnsupported and is a harness artifact, not a page defect.
SUGGESTED: none.

Every other sentence I could test held: `onmode` probe printed `on defaults fg=255,255,255,255 bg=0,0,0,255 bold=FALSE ul=FALSE`, `after set fg=10,20,30,a=255` (alpha 7 ignored, getter reports 255), `measure empty=0 zw=1 cjk=4`, `idle poll kind=None button=None`; `draw` probe showed `drawText(0,-2,"hello")` keeps `llo` at column 0, `drawText(1,77,"world")` keeps `wor`, row 99 draws nothing, `drawGlyph(2,0,10)` is skipped while `drawGlyph(2,1,65)` stamps `A`, `drawHLine(Heavy,3,-5,3)` clamps to four cells, and `moveTo(500,500)` lands the cursor at `24;80`; `ioprint` probe showed `io::print`/`io::write` landing on the surface rows 1 and 2; `nocolor` probe failed to compile with `Package color is used but not imported` as the page says; `mfb man term types` lists 2 records and 4 enums; the banned-word sweep over the rendered page hit only the derived `Allocation failed.` Errors cell (carve-out 2), and the internals sweep hit nothing.
