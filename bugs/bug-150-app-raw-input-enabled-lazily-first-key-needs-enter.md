# bug-150 — `-app` single-key input: raw mode enabled lazily by the first `readChar`, so the first keypress needs Enter

**Status:** OPEN. Filed 2026-07-11 (macOS `-app`). One half of the "unify
keyboard input" pair — see
[bug-149](bug-149-console-pollinput-cooked-mode-requires-enter.md) for the
console half. Same user-visible symptom (a poll-driven TUI loop needs Enter),
different mechanism (the app keyDown line/raw flag, not a tty line discipline).

## Symptom
In an `-app` (windowed) program that uses `io::pollInput(delayMs)` + `io::readChar()`,
the **first** keypress does nothing until the user presses Return once; **after
that one Return, every subsequent keypress is delivered immediately** (no Enter
needed). The initial Return should not be required at all — single-key reads
should come through on keypress from the start, and only `io::input` /
`io::readLine` should wait for Return.

## Root cause
App-mode keyboard input is delivered by the TUI view's `keyDown:` IMP
(`_mfb_macapp_term_keyDown`, `emit_term_key_down_helper`,
`src/target/macos_aarch64/app/term_view.rs:1048`) writing UTF-8 bytes to a pipe
whose read end is dup2'd onto fd 0. The IMP branches on an "input mode" flag
stored as an associated object on `NSApp` under `INPUT_MODE_KEY`:

- `INPUT_MODE_LINE_ECHO = 1`, `INPUT_MODE_RAW_NO_ECHO = 2`
  (`src/target/macos_aarch64/app/mod.rs:199`).
- keyDown delivers a key to the pipe **immediately only when the mode ==
  `RAW_NO_ECHO` (2)** (`term_view.rs:1108`: `cmp x26, 2 ; b.eq tkd_raw`).
  Otherwise it buffers the key into an input-line string and only flushes to the
  pipe on CR/LF/Enter (`tkd_commit`).

Two facts make the first key require Enter:

1. **The initial mode is neither 1 nor 2 — it is nil (0).** `INPUT_MODE_KEY` is
   only registered as an associated-object *key symbol* at startup
   (`mod.rs:676`), never assigned a value, so `objc_getAssociatedObject` returns
   nil until something sets it. nil is not `== 2`, so keyDown starts in the
   line-buffering branch (buffer until Return).

2. **Raw mode is set lazily, only by `readChar`/`readByte`, and is never set by
   `term::on()` or `io::pollInput`.** The only callers of
   `emit_app_raw_input_mode` → `emit_set_raw_input_mode`
   (`app_io.rs:260`, which writes `INPUT_MODE_RAW_NO_ECHO`) are
   `lower_io_read_char_helper` (`io_helpers.rs:1144`) and
   `lower_io_read_byte_helper` (`io_helpers.rs:965`), at read entry. `term::on`
   makes the TermView first responder but does not set the input mode.

So the startup sequence for a `pollInput` + `readChar` loop is: mode = nil (line
buffering) → user's first key is buffered, not written to the pipe → `pollInput`
(a `poll` on fd 0 = the pipe) sees nothing → `readChar` never runs → **the user
must press Return** to flush the buffered line to the pipe → now `pollInput`
reports ready, `readChar` runs and sets mode = `RAW_NO_ECHO` (2). Raw mode is
**sticky** (nothing restores it to line in the app read path), so from then on
keyDown writes every keystroke immediately and input works per-keypress —
exactly the observed "after one Return, keys track on keypress."

## Verification
Confirmed by code analysis (the keyDown mode dispatch above is conclusive: only
mode 2 delivers immediately, mode is nil at startup, and only `readChar`/
`readByte` set mode 2). Not reproduced by an automated run here because `-app`
mode needs a live macOS window/NSApplication, which is not drivable headlessly
in this environment; the mechanism is fully determined by the emitted keyDown
logic and the (absent) mode initialization.

## Fix direction (design — no change made)
Enable raw single-key delivery when interactive single-key input begins, not
lazily inside the first `readChar`:

- Have `term::on()` (app backend) set `INPUT_MODE_KEY = RAW_NO_ECHO` so the
  TermView delivers keys immediately from the moment TUI mode is entered, and
  `term::off()` restore line mode. Alternatively (or additionally), have
  `io::pollInput` ensure raw mode when it runs, so a poll-driven loop works even
  without `term::on`.
- Keep `io::input` / `io::readLine` explicitly switching to
  `INPUT_MODE_LINE_ECHO` for their own read (already done in
  `emit_app_io_input_helper`, `app_io.rs:245`) so they still commit on Return.
- Do the symmetric thing on the console backend (bug-149) so both modes behave
  identically: single-key reads are immediate; only `io::input`/`io::readLine`
  wait for Return.

Apply the same mode change to both keyDown IMPs — the transcript view
(`_mfb_macapp_key_down`) and the TUI TermView (`_mfb_macapp_term_keyDown`) — and
to the `linux_gtk` app backend, whose keyDown/key-press path has the same
line-vs-raw structure.
