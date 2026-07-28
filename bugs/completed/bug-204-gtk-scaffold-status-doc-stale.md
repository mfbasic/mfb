# bug-204: linux_gtk SCAFFOLD STATUS doc is materially wrong on safety-relevant points

Last updated: 2026-07-14
Effort: small (<1h)
Severity: MEDIUM
Class: docs

Status: Fixed (2026-07-15) — the linux_gtk SCAFFOLD STATUS block now describes the implemented main-thread contract: io::print/write marshal via g_idle_add(APPEND_IDLE), printError is prefix-distinguished, and the finish path parks the worker in pause() for the GUI case (only _exits headless). Documentation only.
Regression Test: n/a (documentation)

The `src/target/linux_gtk/mod.rs` module-level SCAFFOLD STATUS block describes
behavior that contradicts the current implementation on thread-safety-relevant
points, so a reader could "fix" a non-existent hole or distrust the working
marshal.

The doc claims `io::print`/`io::write` "append to the GtkTextBuffer directly from
the worker thread; the main-thread marshal (g_idle_add / condvar) ... is not yet
wired" and that "the finish path hard-exits via `_exit` instead of keeping the
window open." In fact `emit_app_io_write_helper` marshals every transcript write
via a malloc'd chunk + `g_idle_add(APPEND_IDLE)` on the main loop, and
`emit_finish_helper` parks the worker in `pause()` for the GUI case and only
`_exit`s headless.

## Root Cause

`src/target/linux_gtk/mod.rs:16-24` SCAFFOLD STATUS block never updated after the
marshal + pause()-park finish path landed.

## Non-goals

- No behavior change; documentation only.

## Fix Design

Rewrite the SCAFFOLD STATUS block to describe the implemented main-thread marshal
(`g_idle_add`) and the `pause()`-park finish path (stderr is prefix-distinguished,
not raw-appended).
