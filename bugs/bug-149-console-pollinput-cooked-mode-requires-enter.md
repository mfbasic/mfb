# bug-149 — console single-key input requires Enter: `io::pollInput` polls a cooked-mode terminal

**Status:** OPEN. Filed 2026-07-11 (aarch64 host / macOS). One half of the
"unify keyboard input" pair — see [bug-150](bug-150-app-raw-input-enabled-lazily-first-key-needs-enter.md)
for the `-app`-mode half. The two share a user-visible symptom (a poll-driven
TUI loop needs Enter for every keystroke) and a fix shape (enter single-key
mode when interactive input begins, not lazily inside the read call).

## Symptom
In a non-`-app` (console) program that uses the standard real-time input pattern
— `io::pollInput(delayMs)` as a frame timer, then `io::readChar()` only when the
poll reports ready — **single keypresses are not registered until the user
presses Return.** Each key effectively needs an Enter. Only `io::input` /
`io::readLine` should wait for Return; single-key reads should not.

## Verification (empirical, this session)
`examples/life` (console build) driven in a PTY sized 80×24:

```
start pattern: random
press '5'  (no Enter)  -> pattern: random   # keypress ignored
press Enter            -> pattern: pulsar   # the '5' only took effect on Enter
```

The `5` keystroke was invisible to the loop until the Return arrived, at which
point `pollInput` reported ready and `readChar` consumed the buffered `5`. So
the loop advances generations but never sees a bare keypress.

## Root cause
Two facts combine, both in `src/target/shared/code/io_helpers.rs`:

1. **`io::pollInput` never touches the terminal mode.** `lower_io_poll_input_helper`
   (line 605) just builds a `pollfd { fd = 0, events = POLLIN }` and calls
   `platform.emit_poll_input` (a bare `poll`/`ppoll` on fd 0). It does no
   `tcsetattr`. So it observes stdin under whatever line discipline is currently
   installed.

2. **`io::readChar` / `io::readByte` enable raw mode only transiently, per read.**
   `lower_io_read_char_helper` (line 1093) calls `emit_configure_stdin_terminal(...,
   disable_echo = true, disable_canonical = true, ...)` (clears `ICANON`/`ECHO`,
   sets `VMIN = 1`, `VTIME = 0`) at entry, does the `read`, then
   `emit_restore_stdin_terminal` (line 1437) restores the *previous* termios on
   exit. `readByte` does the same. So raw mode exists only for the duration of a
   single blocking read; **between reads the terminal is back in cooked mode.**

Consequently, while the loop sits in `pollInput`, the tty is in canonical mode:
the tty driver holds typed bytes in its line buffer and does not make them
readable — so `poll` does not report POLLIN — until the line is completed with
Return. `term::on()` does **not** help: on the console backend it only emits
ANSI escapes (alternate screen + color/cursor reset in
`src/target/shared/code/term.rs`, `ESC_ON`), and never changes the terminal
mode. Nothing puts the console into a persistent cbreak/raw mode for a
poll-driven loop, and there is no public call to request it.

(Note: a program that blocks directly in `io::readChar` with no `pollInput` —
e.g. `examples/hangman` — *does* get immediate single keys, because that one
`readChar` sets raw mode around its own blocking read. The defect is specific to
the non-blocking poll-then-read pattern that any real-time TUI needs.)

## Fix direction (design — no change made)
Make single-key mode a property of "interactive TUI input is active", symmetric
with the `-app` backend, rather than a transient side effect of each `readChar`:

- Preferred: have `term::on()` put the console tty into cbreak/raw mode
  (`~ICANON`, `~ECHO`, `VMIN = 1`, `VTIME = 0`) and `term::off()` restore it, so
  `pollInput` observes single keypresses and `readChar` need not toggle per read
  while TUI mode is on. `io::input` / `io::readLine` must temporarily restore
  cooked/line mode for their own read so they still wait for Return (mirrors the
  `-app` `INPUT_MODE_LINE_ECHO` switch in `app_io.rs`).
- Alternative (no `term` dependency): make `io::pollInput` place stdin in raw
  mode for the duration of the poll (and restore after), so a keypress becomes
  readable immediately. Heavier per-call cost and races with concurrent readers,
  so the `term::on` approach is cleaner.

Either way the unifying rule is: entering interactive single-key input flips the
terminal into immediate mode once; only `io::input`/`io::readLine` fall back to
line mode. Keep the existing per-read raw toggle as the fallback for programs
that call `readChar` without `term::on`.

## Repro artifact
`examples/life` is a faithful reproducer as-is (its loop is `pollInput` + `readChar`).
It currently ships with an inline-`TRAP` + terminal guard workaround for an
*unrelated* crash ([bug-148](bug-148-loop-error-propagation-nulls-error-message.md));
neither the guard nor the trap affects this input behavior.
