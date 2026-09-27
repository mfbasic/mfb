### 1. Re-entering TUI mode loses the remembered line-editing mode (code bug)
UNIT:      man-page:term/on
CLAIM:     "The normal line-editing mode is remembered, so term::off, io::input and io::readLine restore it." together with "calling it while TUI mode is already on runs the whole sequence again"
VERDICT:   wrong
EVIDENCE:  Under a pseudo-terminal, `term::on(); term::off()` leaves the tty sane, but `term::on(); term::on(); term::off()` leaves it in single-key, no-echo mode after the program exits. Probe `/tmp/plan-125-scratch/D-phase3/man-page-term-on/ptyrun.py` runs each binary then `stty -a`:
```
=== once   → lflags: icanon isig iexten echo echoe ...
=== twice  → lflags: -icanon isig iexten -echo echoe ...
```
Cause: `emit_configure_stdin_terminal` (`src/codegen/io/terminal/io_terminal.rs`) unconditionally `tcgetattr`s the *current* discipline into `slots.original`; on the second `on` the current discipline is already the raw one set by the first, so `term::off` "restores" raw mode. `emit_on` (`src/codegen/term/core/term.rs`) has no guard on `TERM_STATE_RAW_ACTIVE_OFFSET` before re-saving. This is a compiler bug, not a prose bug; the page's guidance ("Guard with term::isOn if that is not wanted") is currently the only thing standing between a developer and a wedged shell.
SUGGESTED: Fix the code (skip the save, or restore-then-resave, when the raw-active flag is already 1) and file it. Until then the page must warn: "Calling term::on while TUI mode is already on is not safe: term::off afterwards leaves the terminal in single-key mode. Always guard with term::isOn." Once fixed, the current wording is fine.

### 2. "only term::sync puts a frame on screen" is contradicted by term::off and by program exit
UNIT:      man-page:term/on
CLAIM:     "only term::sync puts a frame on screen. A program that draws without calling `term::sync` displays nothing."
VERDICT:   misleading
EVIDENCE:  Probe `nosync.bas` (on, `io::print`, `io::write`, no sync, no off) with stdout captured: the drawn text is presented at exit, then the alternate screen is left:
```
\e[?1049h...\e[1;1H...SHOULD-NOT-SHOW...\e[2;1H...NOR-THIS...\e[?1049l\e[0m
```
`src/codegen/engine/builder/mod.rs` ("The auto-restore on exit (§6.5) ... need the `off` helper emitted") and `emit_off` (`term.rs`, "present the final frame before restoring the user's screen"). The sibling overview page already says "term::off performs a final term::sync before restoring the screen, so the last frame is always shown", so this page disagrees with its own package. The developer-visible truth: without sync the frame is shown only at the instant `term::off` (or exit) runs, and is then immediately hidden by leaving the alternate screen.
SUGGESTED: "Only term::sync presents a frame while TUI mode is on. term::off — and program exit, which calls it for you — presents whatever is pending and then leaves the alternate screen, so a program that never calls term::sync shows its frame only for the instant before the screen is restored, which in practice is nothing."

### 3. io::input / io::readLine restore line-editing mode only for the duration of the read
UNIT:      man-page:term/on
CLAIM:     "The normal line-editing mode is remembered, so term::off, io::input and io::readLine restore it."
VERDICT:   misleading
EVIDENCE:  `emit_console_raw_line_mode` doc comment, `src/codegen/io/terminal/io_terminal.rs`: "`line_mode = true` selects the saved cooked buffer (a line read waits for Return and echoes as usual); `line_mode = false` re-applies the raw buffer after the read." So a line read swaps to normal mode, reads, and swaps single-key mode back. The sentence as written reads as if a single `io::readLine` permanently ends single-key mode, which it does not. Not probed (needs a tty with interactive input); source evidence only.
SUGGESTED: "term::off restores the normal line-editing mode. io::input and io::readLine switch to it just for that one read — waiting for Return and echoing as usual — and put single-key mode back afterwards."

### 4. ErrOutOfMemory is described but the Errors table is absent
UNIT:      man-page:term/on
CLAIM:     "If the surface cannot be set up, ErrOutOfMemory is raised and the terminal is left completely untouched"
VERDICT:   incomplete
EVIDENCE:  Rendered page has no Errors section because the descriptor declares `errors: vec![]` (`src/codegen/builtins/term/func_on.rs`, `register`), while `emit_on` (`term.rs`) does `raise_error_into(symbol, "ErrOutOfMemory", …)` on the `alloc_fail` label. `.ai/man-content.md` §2 requires every raisable error to be "consistent with the auto-derived Errors table"; here the table is missing, which §0 says is a descriptor bug, not prose. The same `errors: vec![]` is on every term page (`grep -n "errors:" src/codegen/builtins/term/func_*.rs`), including `terminalSize`, which raises `ErrUnsupported` (probe `gated.bas` printed "terminalSize raised: 77050007"). The raise itself is UNVERIFIED by probe: I cannot force an allocation failure.
SUGGESTED: Descriptor fix, main thread only: add `ErrOutOfMemory` to `term::on`'s `errors` (and `ErrUnsupported` to `terminalSize`'s) so the derived table matches the prose. Prose unchanged.

### 5. "not gated" is untrue for app builds outside Console mode
UNIT:      man-page:term/on
CLAIM:     "term::on is one of the calls that are not gated"
VERDICT:   incomplete
EVIDENCE:  `lower_term_helper` in `src/codegen/builtins/term/gen_shared.rs`: for `--app` builds every term helper "(including `on`)" gets `prepend_wrong_mode_gate(…, ModeRequirement::Console)`, raising `ErrWrongMode` in `None` or `Canvas` mode. `mfb man app` says only that "term:: is unavailable" in Canvas; this page says nothing. UNVERIFIED by probe (needs an `--app` build with a window); source evidence only.
SUGGESTED: Append to the paragraph: "In an app build (mfb build --app), term::on — like every term:: call — raises ErrWrongMode unless the window is in Console mode; see mfb man app."

### 6. Fallback size is not what term::terminalSize then reports
UNIT:      man-page:term/on
CLAIM:     "It asks the terminal for its size, falling back to 24 rows by 80 columns when the terminal cannot say."
VERDICT:   incomplete
EVIDENCE:  True for the surface: with stdout piped, every probe paints rows 1 through 24 (`out_ex1.txt`: `\e[24;1H` is the last row). But `sizeon.bas` (`term::on()` then `term::terminalSize()` with stdout piped) raised uncaught:
```
Error: 7-705-0007
Operation is not supported by the implementation or platform.
```
exit 255. So when the fallback is in use, `terminalSize` does not return 24 by 80; it raises `ErrUnsupported` even though TUI mode is on. A developer reading this page would expect to read the fallback back. `emit_grid_alloc` (`term_grid.rs`) takes the `size_fail` branch to `DEFAULT_ROWS`/`DEFAULT_COLS`; `emit_terminal_size` does not.
SUGGESTED: "...falling back to a 24-row by 80-column surface when the terminal cannot say. In that case term::terminalSize still raises ErrUnsupported rather than reporting the fallback, so trap it if the program may run with output redirected."

Everything else on the page checked out: order of the four steps matches `emit_on`; defaults (white fg, black bg, bold/underline off, cursor visible, home) match `gated.bas` output and the `writes` table; alternate screen and colour reset match `ESC_ON`; not-a-tty skip and `raw_failed` fallback match `emit_configure_stdin_terminal`; re-entry discarding the old frame and restoring defaults verified by `reenter.bas` (output `bold=FALSE fg=255` at row 1, no `OLD-FRAME`); both examples compile and run (`p_ex1`, `p_ex2`, exit 0, `hello in red` painted in `38;2;255;0;0`). No banned vocabulary and no compiler-internal sentences on the page.
