# bug-1044: on GTK, `pollMouse` never reports the right or middle button

Last updated: 2026-10-11
Effort: small (<1h)
Severity: MEDIUM
Class: Other

Status: Fixed (2026-10-11)
Regression Test: none automated — it takes a real window and a clicked pointer. Manual: `tools/pointer-probe/mutter-pointer.py` on box 2226, below.

On Linux (GTK4), `term::pollMouse` and `canvas::pollMouse` reported presses and
releases of the left button only. A right or a middle click produced no event at all,
although the handlers map all three buttons.

**The single correct behavior a fix produces:** in a real GTK window with the mouse
enabled, a right click is polled as `Down` then `Up` with `button` `Right`, and a middle
click with `Middle`.

References:

- `src/target/linux_gtk/mouse.rs:emit_attach_controllers` — creates the click gesture.
- `src/target/linux_gtk/mouse.rs:emit_button_code` — reads
  `gtk_gesture_single_get_current_button` and maps 1, 2, 3 to left, middle, right.
- GTK4: `GtkGestureSingle:button` — "Mouse button number to listen to, or 0 to listen
  for any button", default `GDK_BUTTON_PRIMARY`.

## Cause

`gtk_gesture_click_new()` returns a `GtkGestureSingle` whose `button` property is 1: it
listens to the primary button and ignores the rest. Nothing set it to 0. The handlers'
three-way button mapping was written, and was unreachable for two of the three.

## Repro

The bug-1043 run, on a build with bug-1043 fixed: `home`, twelve `rel 70 45`, then
`down` / `up` for the left, right and middle buttons.

- Before: `1 down left`, `1 up left`, `14 other none`. Nothing for right or middle —
  while the same run's `pointer=` stats show buttons 4 and 2 held in turn, from
  plan-172-J's own gesture on the drawing area, which does listen to every button.
- After: `1 down left`, `1 down middle`, `1 down right`, `1 up left`, `1 up middle`,
  `1 up right`, and the moves.

## Fix

`emit_attach_controllers` calls `gtk_gesture_single_set_button(gesture, 0)` on the click
gesture, and a mouse program imports that symbol.

## Verification

The repro above, after: all six button events reported.
