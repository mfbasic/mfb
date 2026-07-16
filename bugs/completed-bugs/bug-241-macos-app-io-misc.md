# bug-241: macOS app — partial-write line truncation, unclosed pipe read-end, dead terminal-size helper

Last updated: 2026-07-16
Effort: small (<1h)
Severity: LOW
Class: correctness / memory-safety / dead-code

Status: FIXED (2026-07-16). All three items done, plus a stale test found while
verifying (below).

1. **Partial write** — `kd_commit`/`tkd_commit` now loop to deliver the
   remainder, and only write the trailing newline once the whole line is out.
   The loop cannot block the UI thread: it exits on a `<= 0` return (the
   O_NONBLOCK EAGAIN case bug-114 handles), so each pass either delivers at
   least one byte or leaves — termination is structural, not a retry budget.
   Skipping the newline on give-up is what keeps a truncated line from being
   presented to the program as a complete one. Chose "loop" over the bug's
   alternative "drop the whole line": once a partial write has landed in the
   pipe those bytes cannot be recalled, so dropping cannot actually undo the
   truncation.
   `x22` carries the remaining count across `_write` (which clobbers x0-x17).
   In `kd_*` it was free (text storage is dead on the commit path — only
   `kd_backspace` reads it, and `kd_commit` branches straight to `kd_done`); in
   `tkd_*` it was NOT saved at all, so it was added to that helper's
   save/restore list at the already-reserved slot 32.

2. **Unclosed pipe read end** — `_main` now closes `fds[0]` after the `dup2`,
   with `_close` added to `app_mode_imports`. Guarded on two cases the bug did
   not mention, both of which would be worse than the leak: a failed `dup2`
   (fds[0] is then the only read end) and `fds[0]` already being fd 0 (reachable
   only if stdin was closed before `pipe`, making dup2 a no-op — closing would
   leave the program with no stdin at all).

3. **Dead terminal-size helper** — removed (160 lines in `app_io.rs`, plus the
   `macos_aarch64/code.rs` override and the `shared/code/types.rs` trait method).
   Its retention note claimed it was held for plan-01-term Phase 5's app-mode
   `term::terminalSize`, but that backend already exists as
   `emit_app_terminal_size` and reads the TermView grid directly rather than
   deriving cols/rows from scroll-view content size + font metrics — so this was
   a superseded implementation, not a deferred one. Proof it was dead: removing
   it produced ZERO `.ncode` delta. Proof the rationale was obsolete: the GUI
   suite now reports `term::terminalSize reported window surface (114x37)`.

**Also fixed — stale GUI test (found by running `MFB_MACAPP_GUI=1`):** Case 8 of
`scripts/test-macapp.sh` still called `io::terminalSize`, removed by
plan-01-term Phase 3. Because the case is GUI-gated it had been skipped on every
default run, so the whole `MFB_MACAPP_GUI=1` suite failed at `build -app tsize`
and nobody saw it. Ported to `term::terminalSize` (with the `term::on()` TUI gate
it requires). Same `io::terminalSize`-removal debris as item 3 — pre-existing and
NOT caused by the removal here (it fails at frontend resolution:
"Built-in package `io` does not export `io.terminalSize`").

Verified: emitted `.ncode` reviewed instruction-by-instruction for both commit
loops, the x22 save/restore, and the guarded close; full acceptance 949 tests
with only the 2 unrelated stale `.audit` mismatches (bug-211's, fixed
separately); `MFB_MACAPP_GUI=1 scripts/test-macapp.sh` fully green — notably
"window keypresses delivered to io::readLine", which exercises the changed
`kd_commit` path with real keystrokes.

Original report — three low items in the macOS app-mode target:

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
