# bug-114 — app-mode keyDown commit writes to input pipe from UI thread with no full-pipe handling → permanent freeze

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G7).
**Severity:** LOW — requires a program that never reads stdin and >64 KiB of
committed input.
**Class:** footgun (UI deadlock).

## Finding

`src/target/macos_aarch64/app/term_view.rs:110-121` and :1138-1149 (transcript
+ TermView keyDown commit `_write` to the pipe),
`src/target/linux_gtk/bootstrap.rs:371-381` (`emit_key_pressed_handler` commit
`write`).

On Return, the key handler (running on the AppKit/GTK main thread) does a
**blocking** `write()` of the buffered line into the window-input pipe. If the
program never reads stdin, typed-and-committed input accumulates in the pipe;
once the pipe buffer (64 KiB) fills, the next commit blocks the UI thread
forever (the worker never drains it), freezing the window. There is no
O_NONBLOCK/short-write handling on the pipe write, and the write return is
unchecked.

## Trigger

App-mode program that never reads input; user pastes/types > 64 KiB of
committed lines → window becomes permanently unresponsive.

## Fix sketch

Make the pipe write end non-blocking (O_NONBLOCK) and drop / bounded-buffer
input on EAGAIN, or move the commit write off the UI thread. Check the write
return.
