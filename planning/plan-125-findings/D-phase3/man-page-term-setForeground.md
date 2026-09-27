### 1. In an `--app` build the colour is not taken from the `color::Color` at all
UNIT:      man-page:term/setForeground
CLAIM:     "term::setForeground sets the colour that subsequent text drawn through the term:: surface will be written in, from a single color::Color."
VERDICT:   wrong
EVIDENCE:  True for a console build, false for an `--app` build. The console emitter `emit_set_color` in `src/codegen/term/core/term.rs` loads red/green/blue off the record pointer in `c_arg(0)`. The app emitters were never migrated by the plan-122-F commit `a56daca28` (`git show a56daca28 --stat` lists no `app_io.rs`): `emit_app_set_color` in `src/target/macos_aarch64/app/app_io.rs` still packs `mfb_arg(0)`, `mfb_arg(1)`, `mfb_arg(2)` as r, g, b, so it packs the low byte of the record pointer plus two stale registers. `emit_app_term_set_color` in `src/target/linux_gtk/app_io.rs` and `emit_term_set_color` in `src/target/win_x86_64/app/mod.rs` have the same three-scalar shape (read, not run). Live repro on macOS, `mfb build -q --app appprobe` then running `appprobe.app/Contents/MacOS/appprobe`:
  ```
  term::on()
  term::setForeground(color::rgb(10, 20, 30))
  LET c AS color::Color = term::getForeground()
  term::off()
  fs::writeText(".../app-result.txt", color::toHex(c))
  ```
  wrote `#50583f`. The identical program as a console build (`probe/`) prints `#0a141e`. This is a compiler bug, not a doc bug; the page cannot be made true by rewording. Needs a bug document: app-mode `setForeground`/`setBackground` on all three app backends must load the four channel fields off the record pointer exactly as `emit_set_color` does.
SUGGESTED: No wording change. File the bug; once fixed the sentence is true as written.

### 2. `term::off` also ends the setting, but the page names only `setForeground` and `on`
UNIT:      man-page:term/setForeground
CLAIM:     "The setting persists until the next term::setForeground or the next term::on, which resets the foreground to white (255, 255, 255)."
VERDICT:   incomplete
EVIDENCE:  Probe (`probe/src/main.mfb`, console build): after `setForeground(rgba(10,20,30,40))`, `getForeground` returns `#0a141e`; immediately after `term::off()` it returns `#ffffff`; after a following `term::on()` it is still `#ffffff`. `emit_get_color` in `src/codegen/term/core/term.rs` is gated on the active flag and returns `DEFAULT_FOREGROUND_PACKED` while off, and `emit_on` rewrites `TERM_STATE_FG_OFFSET` to white. So there is no observable path by which a colour survives `off`; a reader of this sentence would expect it to.
SUGGESTED: "The setting lasts for the current TUI session: it persists until the next term::setForeground, and is reset to white (255, 255, 255) by term::off and again by term::on. The background colour and the bold and underline attributes are independent and are left untouched; the current value can be read back with term::getForeground."

### 3. "Does nothing and reports no error" is false in an `--app` program outside Console mode
UNIT:      man-page:term/setForeground
CLAIM:     "The call is gated: while TUI mode is off it does nothing and reports no error."
VERDICT:   incomplete
EVIDENCE:  Console build probe confirms the no-op: `setForeground(rgb(1,2,3))` before `term::on()` then `getForeground` prints `#ffffff`, exit 0. But `lower_term_helper` in `src/codegen/builtins/term/gen_shared.rs` prepends `prepend_wrong_mode_gate(..., ModeRequirement::Console)` to every app-mode `term::` helper: in an `--app` build that has left `Console` presentation mode (for example `app::Mode.Canvas`), the call raises the trappable `ErrWrongMode` rather than doing nothing. The sibling `term::on` page is where the full rule belongs, but this sentence as written asserts silence unconditionally. UNVERIFIED by run: I did not build a Canvas-mode app probe; the claim rests on reading the gate.
SUGGESTED: "The call is gated: while TUI mode is off it does nothing and reports no error. In an --app build it must also be in Console presentation mode; in any other mode it raises the trappable wrong-mode error, as every term:: call does — see term::on."

### 4. "packed into the module's current-attribute state" is compiler-side vocabulary
UNIT:      man-page:term/setForeground
CLAIM:     "The colour is packed into the module's current-attribute state and no escape sequence is emitted."
VERDICT:   out-of-scope
EVIDENCE:  `.ai/man-content.md` §3: a sentence only meaningful to someone reading compiler source does not belong. "Packed", "module's ... state" describe the `r|g<<8|b<<16` store in `emit_set_color`; the developer-observable fact is only the second half (nothing reaches the terminal until `term::sync`), which the next sentence already states. Verified observably: ex1's output (`od -c`) shows the `38;2;255;0;0` sequence only inside the frame `sync` presents, immediately before "alert".
SUGGESTED: "Nothing is written to the terminal by this call. Like every other drawing operation on this retained surface, the effect becomes visible only when term::sync presents the frame."

### 5. Example 2 compiles and runs, but demonstrates nothing a reader can see
UNIT:      man-page:term/setForeground
CLAIM:     "The colour round-trips through term::getForeground, so saving and restoring needs no channel unpacking:" followed by the save/restore example.
VERDICT:   misleading
EVIDENCE:  Built and ran (`mfb build -q ex2`, `./ex2/build/ex2.out`): exit 0, output is only the on/off escape sequences. Nothing is drawn between the save and the restore, so the example never shows the round-trip it is captioned as demonstrating. The round-trip itself is true: the probe's `setForeground(rgba(10,20,30,40))` read back as `#0a141e`.
SUGGESTED: Draw something between the two calls so the restore is visible, e.g. after `term::setForeground(color::fromName("teal"))` add `term::drawText(0, 0, "teal")`, then after `term::setForeground(saved)` add `term::drawText(1, 0, "default again")` and `term::sync()` before `term::off()`.

Everything else on the page checked out against the console build. Alpha is ignored and not an error: `rgba(10,20,30,40)` read back as `#0a141e` and the program exited 0. Background, bold and underline leave the foreground untouched: after `setBackground`/`setBold(TRUE)`/`setUnderline(TRUE)` the foreground still read `#0a141e`. The default after `term::on` is `#ffffff`. Example 1 builds and its presented frame carries `ESC[38;2;255;0;0m` directly before `alert`, which matches the caption. No banned memory vocabulary and no ownership mental model found on the page. Scratch programs are under `/tmp/plan-125-scratch/D-phase3/man-page-term-setForeground/{ex1,ex2,probe,appprobe}`.
