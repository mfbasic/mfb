# bug-1128: Linux `--app` `term::clear`/`hideCursor`/`showCursor` present a frame without `term::sync`

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: LOW
Class: Runtime (app backend parity)

Status: Open
Regression Test: —

`mfb man term` and `mfb man term sync` say `term::sync` is the one operation that
presents a frame. On the Linux GTK app backend (read, not run)
`src/target/linux_gtk/app_io.rs:emit_app_term_clear` and
`emit_app_term_set_cursor` each schedule `TERM_REDRAW_IDLE_SYMBOL` through
`g_idle_add` — the same redraw `emit_app_term_sync` uses — so a clear or a cursor
toggle shows a partly drawn frame. macOS and Windows `--app` do not. The `clear`
and cursor pages currently describe the Linux behaviour.

**The correct behavior a fix produces:** on every backend nothing is presented
until `term::sync`; the three pages then drop their Linux repaint sentence.

References:

- Found by the plan-125 B recheck (`term` package), finding 1 (source only).
