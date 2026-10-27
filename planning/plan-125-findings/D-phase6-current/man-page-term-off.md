Verification is complete. Both examples compiled and ran, and the exit-path claims were confirmed by code and by probe. Findings follow.

### 1. "does nothing at all" while off is false for a mouse program
UNIT:      man-page:term/off
CLAIM:     "while TUI mode is already off the call does nothing at all and reports success — on every backend, in a terminal and in an `mfb build --app` window alike."
VERDICT:   wrong
EVIDENCE:  `src/codegen/term/core/term/screen_mode.rs:emit_off` places `emit_mouse_withdraw` after the `_inactive` label, with a comment saying it runs "whether or not TUI mode was on". Probe `mouseoff.mfb` (`term::enableMouse(TRUE)` then `term::off()` with no `term::on`) printed, via `od -c`: `\e[?1000h\e[?1002h\e[?1006h` then `\e[?1006l\e[?1002l\e[?1000l` then `after`. The three `l` sequences are `term::off` withdrawing mouse tracking while TUI mode was off. In an `--app` build the gated path is a true no-op (`app_io.rs:emit_app_term_off_helper` branches straight to `term_off_done`), so the "on every backend alike" clause is also wrong in the other direction.
SUGGESTED: "It is gated: while TUI mode is already off, `term::off` leaves the screen and input mode untouched. One thing still happens either way in a terminal: if the program turned mouse reporting on with `term::enableMouse` and never turned it off, `term::off` turns it off, whether or not TUI mode was on."

### 2. The teardown list omits mouse reporting and the keyboard protocol
UNIT:      man-page:term/off
CLAIM:     "When TUI mode is on, the teardown runs in this order." (steps 1–4)
VERDICT:   incomplete
EVIDENCE:  `screen_mode.rs:emit_off` runs, in order: sync; cooked termios restore; `emit_kitty_disable` (pops the keyboard protocol `term::on` pushed, only in programs whose `io` readers decode keys); `ESC_OFF`; active flag cleared; grid freed; then mouse withdraw. Neither the keyboard-protocol pop nor the mouse withdrawal appears on the page, yet both are observable bytes left on the user's terminal if a program is killed, and the mouse one is the reason a program that forgot `term::enableMouse(FALSE)` does not leave the next shell command receiving mouse events.
SUGGESTED: Add after step 4: "5. **Mouse reporting is turned off** if `term::enableMouse` turned it on and the program never turned it off." Extend step 2 with: "and any extended keyboard reporting `term::on` enabled is switched back off."

### 3. "reports success" from a function that returns Nothing
UNIT:      man-page:term/off
CLAIM:     "the call does nothing at all and reports success"
VERDICT:   misleading
EVIDENCE:  Declaration on the same page: `term::off() AS Nothing`. The function has no return value and raises nothing in a console build, so there is nothing a program can read as "success". The phrase describes the internal result tag (`RESULT_OK_TAG` in `emit_off`), which is a compiler detail (§3).
SUGGESTED: "the call does nothing and does not raise."

### 4. App-mode step order differs from the list the page says is "the same"
UNIT:      man-page:term/off
CLAIM:     "step 3 instead swaps the window back to its scrolling text view, which regains keyboard focus, and typing returns to line input. Steps 1, 2 and 4 are the same."
VERDICT:   misleading
EVIDENCE:  `src/target/macos_aarch64/app/app_io.rs:emit_app_term_off_helper`: present (`display`), then `setContentView:`, then `makeFirstResponder:`, and only then `emit_set_input_mode_instructions(INPUT_MODE_LINE_ECHO)` and the active flag clear. Line input is restored after the view swap, not before it as the numbered list says. Not probeable here (no `--app` window in the sandbox); verdict rests on reading the emitter.
SUGGESTED: "In an `mfb build --app` window there is no cursor or alternate screen to restore: after the final present, the window swaps back to its scrolling text view, which regains keyboard focus, and then typing returns to line input. The order of the first and last steps is the same."

### 5. Second example teaches a guard the page itself says is unnecessary
UNIT:      man-page:term/off
CLAIM:     "Leave TUI mode only if it was entered:" followed by `IF term::isOn() THEN term::off()`
VERDICT:   misleading
EVIDENCE:  Probe `offwhileoff.mfb` called `term::off()` twice with TUI mode never entered and printed `isOn=FALSE` with exit 0 and no escape bytes. The description's first paragraph already says the call is gated. The example compiles and runs (exit 0, empty output) but models a redundant idiom, and a reader will infer that an unguarded `term::off` is unsafe.
SUGGESTED: Replace with an example that shows why one calls `term::off` explicitly, which the description names: draw a frame, `term::off()`, then `io::print` ordinary output, so the reader sees that output after `term::off` lands on the normal screen. Build was verified for the first example: `ex1` produced `\e[?1049h … done … \e[?25h\e[?1049l\e[0m`, matching the page's claim that the frame is presented and the screen restored.

Everything else on the page checked out. The unhandled-error path restores the terminal (probe `errexit.mfb`, exit 255, output ends in `\e[?25h\e[?1049l\e[0m`). Returning from `main` without `term::off` does the same (probe `noexplicit.mfb`). Re-entering with `term::on` resets state (probe `reenter.mfb` printed `bold-after-off=FALSE` and `bold-after-reenter=FALSE`). Ctrl-C and SIGTERM route through `process_lifecycle.rs:lower_shutdown` with `auto_term_off`, per the handler comment at line 131, not probed by signal. The `ErrWrongMode` row matches `gen_shared.rs:lower_term_helper`, which prepends the Console gate on every app-mode `term::` helper. No banned memory vocabulary and no ownership-model sentences were found.
