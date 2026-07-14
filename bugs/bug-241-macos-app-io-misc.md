# bug-241: macOS app — partial-write line truncation, unclosed pipe read-end, dead terminal-size helper

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness / memory-safety / dead-code

Status: Open

Three low items in the macOS app-mode target:

- `term_view.rs:120-127` (`kd_commit`) and `:1156-1163` (`tkd_commit`) treat any
  non-negative `write()` return as full success, so a partial pipe write (line
  spanning the buffer boundary with the O_NONBLOCK input pipe ~64 KiB backed up)
  silently truncates the delivered line and still appends the trailing newline —
  the program reads a corrupted/short line. Distinct from bug-114 (EAGAIN
  freeze). Fix: on a short write (`n != len`) loop to write the remainder or drop
  the whole line and skip the newline.
- `bootstrap.rs:367-369`: in `_main` the pipe read end `fds[0]` is `dup2`'d onto
  fd 0 but never closed, leaking the original descriptor for the process
  lifetime. Fix: `close(fds[0])` after the `dup2` succeeds.
- `app_io.rs:325-475` (`emit_app_io_terminal_size_helper`, ~150 lines) and its
  full trait override chain are never invoked — the live `term::terminalSize`
  path uses `emit_app_terminal_size`. Currently retained for "plan-01-term Phase
  5" and triple-annotated `#[allow(dead_code)]`. Fix: remove it, or keep as an
  explicitly-tracked deferred item.
