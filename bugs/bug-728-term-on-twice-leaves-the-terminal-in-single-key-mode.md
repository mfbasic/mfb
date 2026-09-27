# bug-728: calling `term::on` while TUI mode is already on makes `term::off` leave the terminal in single-key mode

Last updated: 2026-09-25
Effort: small
Severity: HIGH
Class: Correctness (the user's shell is left unusable after the program exits)

Status: Open
Regression Test: none yet — see Phase 1

`term::on` saves the terminal's current line discipline so `term::off` can put it
back. It saves it every time it runs. A second `term::on` while TUI mode is already
on therefore saves the *raw* discipline the first call installed, and `term::off`
"restores" raw mode. The program exits with the user's terminal in single-key,
no-echo mode.

Measured on a real pty (`/tmp/p125-tools/term-pty-run.py`, running the program and
then `stty -a` in the same terminal):

| Program | `stty -a` after exit |
|---|---|
| `term::on()`, `term::off()` (`/tmp/p125-ex/termononce`) | `icanon … echo` |
| `term::on()`, `term::on()`, `term::off()` (`/tmp/p125-ex/termontwice`) | `-icanon … -echo` |

`mfb man term on` says calling it again "runs the whole sequence again" and that
the line-editing mode "is remembered, so `term::off` … restore[s] it"; the second
half is false after a re-entry.

**The single correct behavior a fix produces:** `term::off` always restores the
line discipline the terminal had before the *first* `term::on`, however many times
`term::on` ran in between.

Found by plan-125-D Phase 3 (the Fable review of `man-page:term/on`).
**Filed, not fixed**, by user instruction during a documentation-only plan ("file
all bugs, make no fixes").

## Reproduction

```basic
IMPORT term

SUB main()
  term::on()
  term::on()
  term::off()
END SUB
```

```
$ ./termontwice.out; stty -a | grep -o -- '-*icanon'
-icanon
```

Expected: `icanon`.

## Root cause

`src/codegen/io/terminal/io_terminal.rs:emit_configure_stdin_terminal` calls
`tcgetattr` into `slots.original` unconditionally. `src/codegen/term/core/term.rs:emit_on`
calls it without checking `TERM_STATE_RAW_ACTIVE_OFFSET`, so on re-entry the saved
"original" is the raw discipline.

## Non-goals

- Changing what a re-entered `term::on` does to the surface (it resets it; that is
  documented and intended).

## Blast-radius audit

- `io::input`/`io::readLine` swap to the saved buffer for one read: after a
  re-entry they read in raw mode too. Confirm the fix covers them.
- The app backends keep no termios state; confirm they need nothing.

## Fix

Phase 1 — RED test: on a pty, `on`, `on`, `off`, then assert canonical mode and
echo are back. Commit:

Phase 2 — skip the save when the raw-active flag is already set; re-render
`mfb man term on` and drop its interim warning. Commit:
