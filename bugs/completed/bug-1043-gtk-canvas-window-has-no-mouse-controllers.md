# bug-1043: on GTK, a canvas program's window has no mouse controllers, so `canvas::pollMouse` reports nothing

Last updated: 2026-10-11
Effort: small (<1h)
Severity: HIGH
Class: Other

Status: Fixed (2026-10-11)
Regression Test: none automated — it takes a real window and a moved pointer. Manual: `tools/pointer-probe/mutter-pointer.py` on box 2226, below.

On Linux (GTK4), a program that calls `app::setMode` — every canvas program does —
never received a mouse event from `canvas::pollMouse` in a real window. Presses,
releases and motion all reported `MouseKind.None`, forever, with the mouse enabled and
the pointer over the canvas.

**The single correct behavior a fix produces:** a canvas program in a real GTK window,
with `canvas::enableMouse(TRUE)`, polls a `Down` and an `Up` for a left click and
`Move` events for motion over the canvas.

References:

- `src/target/linux_gtk/bootstrap.rs:emit_reconcile_build_helper` — builds the window
  for a program whose static default mode is `None`.
- `src/target/linux_gtk/bootstrap.rs:emit_activate_handler` — builds it for a
  `Console`-default program, and was the only caller of
  `mouse::emit_attach_controllers`.
- `src/target/linux_gtk/mouse.rs:emit_attach_controllers`.
- Found while proving plan-172-J's pointer publication in a real GNOME window.

## Cause

The three mouse controllers are attached to the WINDOW (plan-94-D), so they are
attached where the window is built — and the window is built in two places. A program
that never mentions `app::setMode` starts in `Console` and its window is built by the
activate handler, which attached them. A program that does mention it starts in `None`
with no window; its window is built later, by the mode reconcile
(`_mfb_gtkapp_reconcile_build`), which attached the key controller (plan-98-A Phase 4
added that, for exactly this reason: "Without this the reconcile-built window had no
key handler at all") and nothing else.

A canvas program can only reach `Mode.Canvas` through `app::setMode`, so it always
takes the second path. The cell surface was proven in a real window (bug-982) through
the first.

## Repro

Box 2226's GNOME Wayland session, a `--debug` linux-aarch64 build of a program that
goes fullscreen, enables the mouse, and appends every polled event's kind and button to
a file, with the pointer driven by `tools/pointer-probe/mutter-pointer.py`: `home`,
twelve `rel 70 45`, then a left, a right and a middle click.

- At `6a473b1c7` (before the fix): the file is empty. The same run's `pointer=` stats
  show the pointer inside the canvas the whole time (plan-172-J's own controllers, on
  the drawing area), so GTK was delivering the events — to nothing.
- After the fix: `1 down left`, `1 up left`, `14 other none` (the moves).

## Fix

`emit_reconcile_build_helper` takes `uses_mouse` and calls
`mouse::emit_attach_controllers` after the key controller, as the activate handler
does.

## Verification

The repro above, after: left press and release and fourteen moves reported. (Right and
middle were still missing in that run: bug-1044.)
