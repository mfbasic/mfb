# bug-111 — GTK app-mode term:: setters ignore the §4.2.1 inactive gate (platform-divergent semantics)

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G7).
**Severity:** MED — same builtin behaves differently on Linux app vs macOS app
vs console.
**Class:** correctness (platform divergence vs the locked design).

## Finding

`src/target/linux_gtk/app_io.rs:79-144` (`emit_app_term_set_color`,
`emit_app_term_set_attr`, `emit_app_term_set_cursor`), :246-292
(`emit_app_term_clear`), :295-325 (`emit_app_term_move_to`), :46-73
(`emit_app_term_terminal_size`).

plan-01-term.md §4.2.1 requires every term:: setter to be a no-op while TUI
mode is off, and macOS app-mode enforces it (`emit_term_active_gate`,
macos_aarch64/app/app_io.rs:740-748, applied in every setter; `terminalSize`
returns ERR_UNSUPPORTED while inactive, app_io.rs:1054-1061). The GTK bodies
have **no gate**: `setForeground`/`setBackground`/`setBold`/`setUnderline`
write both the arena term-state and the app state while off, `clear` memsets
the whole grid and schedules a redraw, `moveTo` moves the cursor,
`showCursor`/`hideCursor` write state + redraw, and `terminalSize` returns
OK(grid size) instead of ERR_UNSUPPORTED.

## Trigger

- `term::setForeground(1,2,3)` before `term::on()` then `term::getForeground()`:
  Linux app returns (1,2,3); macOS app and the console backend return the
  default.
- `term::terminalSize()` while off: OK on Linux app, error 77050007 on macOS
  app.

## Fix

Port `emit_term_active_gate` (or an equivalent) into each GTK term setter and
make GTK `terminalSize` return ERR_UNSUPPORTED while inactive, matching macOS
and plan-01-term §4.2.1.
