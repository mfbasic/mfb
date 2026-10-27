### 1. The ErrWrongMode claim is false on every backend: getBold never raises in an --app build
UNIT:      man-page:term/getBold
CLAIM:     "In an mfb build --app program, term::getBold — like every other term:: call — raises ErrWrongMode unless the program is in app::Mode.Console; see mfb man app."
VERDICT:   wrong
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase6-current/man-page-term-getBold/p_app/src/main.mfb` (built with `mfb build -q --app p_app`), a program that references `app::setMode` so it starts in `app::Mode.None`, then calls `term::getBold()` before any `term::on`:

```
IMPORT term
IMPORT io
IMPORT app
FUNC main() AS Nothing
  io::print("mode-none start")
  LET a AS Boolean = term::getBold()
  io::print("got " & toString(a))
  app::setMode(app::Mode.None)
END FUNC
```
Output: `mode-none start` / `got FALSE`, exit code 0. No trap. The control with `term::setBold(TRUE)` in the same position exits 255 with `Error: 7-705-0020` and the ErrWrongMode message, so the gate exists for siblings and the program really was in `Mode.None`.

Cause, read in source: `gen_shared.rs:lower_term_helper` prepends `prepend_wrong_mode_gate` only when the platform's `emit_app_term_helper` returns `Some`. No backend claims `"term.getBold"`: the match in `src/target/macos_aarch64/app/app_io.rs:emit_app_term_helper` has no arm for it (falls to `_ => return None`), and neither does `src/target/linux_gtk/app_io.rs:emit_app_term_helper` nor `src/target/win_x86_64/app/mod.rs:emit_app_term_helper`. The only fall-through members that get a gate anyway are `term.enableMouse`/`term.pollMouse` (explicit `matches!` in `lower_term_helper`). So `getBold` (and by the same path `getForeground`, `getBackground`, `getUnderline`, and on macOS/Windows `isOn`) read the console-backend state ungated and return the inert default in `None`/`Canvas`. The descriptor still declares `errors: vec!["ErrWrongMode"]` (`func_get_bold.rs:register`), so the derived Errors table lists an error the binary cannot raise.

This is a doc/code disagreement, not just a prose slip: the plan-62-E comment in `lower_term_helper` says "every app-mode `term::` helper … is gated", and sibling pages (`getForeground`, `isOn`) repeat the same sentence. The main thread must decide whether the fix is the gate (add the getters to the `matches!` fall-through list, as pollMouse was) or the page plus the declared error. Until the gate lands, the sentence and the Errors row are untrue at HEAD.
SUGGESTED: If the gate is added: keep the sentence as is. If the page is corrected to the binary instead: "In an mfb build --app program, term::getBold is one of the few term:: calls that does not raise ErrWrongMode outside app::Mode.Console: in app::Mode.None or app::Mode.Canvas it returns FALSE, the same inert default it returns while TUI mode is off. Use app::getMode to tell the cases apart." and drop `"ErrWrongMode"` from the descriptor's `errors` so the Errors table stops promising it.

Everything else on the page checked true. Probe `p_probe/src/main.mfb` (console build) printed `before-on=FALSE after-on=FALSE set-true=TRUE toggled=FALSE after-off=FALSE re-on=FALSE on-again-after-set=FALSE`, confirming: inert FALSE before `term::on` and after `term::off`; `term::on` resets bold to off even when it was TRUE; `setBold` round-trips; the toggle idiom works. `emit_get_attr` (`src/codegen/term/core/term/attributes.rs`) only loads a u64 and sets the OK tag, so "reads state only … cannot fail" holds in a console build. Both page examples compiled with `mfb build -q` and ran to exit code 0. No banned vocabulary and no borrow/ownership framing; the page is in scope for the developer audience.
