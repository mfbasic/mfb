# bug-730: the macOS and Windows `--app` term surfaces draw no cursor, so `term::showCursor`/`hideCursor` do nothing there

Last updated: 2026-09-25
Effort: medium
Severity: LOW
Class: Backend parity (a documented call has no visible effect on two backends)

Status: Open
Regression Test: none yet — see Phase 1

`term::showCursor` and `term::hideCursor` set a cursor-visible flag. Two backends
honour it: a terminal program (the present emits `ESC[?25h`/`ESC[?25l` from the
flag) and a Linux `--app` build (`src/target/linux_gtk/term_draw.rs` draws a caret
when it is set). The macOS and Windows `--app` builds store the flag and never read
it — no drawing code draws a cursor at all:

```
$ grep -rn TV_CURSOR_VISIBLE_OFFSET src/target/macos_aarch64/
app/mod.rs:519    the constant
app/term_view.rs:1199   the term::on reset store
app/app_io.rs:1456      the showCursor/hideCursor store
$ grep -n CURSOR_VISIBLE src/target/win_x86_64/app/mod.rs
3711   the term::on reset store
3941   the setter store
```

`emit_app_set_cursor_visible`'s own doc comment says "Cursor glyph rendering is a
later refinement, so no redraw is needed yet".

**The single correct behavior a fix produces:** every `--app` backend draws a
cursor at the position the next drawing would use while the flag is set, and none
while it is clear — as the terminal and Linux GTK backends already do.

Until then `mfb man term showCursor`/`hideCursor` state that the calls have no
visible effect in a macOS or Windows `--app` build.

Found by plan-125-D Phase 3 (the Fable review of `man-page:term/showCursor`).
**Filed, not fixed**, by user instruction during a documentation-only plan ("file
all bugs, make no fixes").

## Reproduction

`mfb build --app` on macOS any program that calls `term::on()`, `term::showCursor()`,
draws text and `term::sync()`s: no cursor is drawn. `term::hideCursor()` changes
nothing either.

## Non-goals

- Cursor shape or blink.

## Blast-radius audit

- bug-540 (Windows app term reduced implementation) — fold the Windows half in if
  that bug is worked first.
- `mfb spec app term-backend`'s per-backend coverage table must record the gap.

## Fix

Phase 1 — an app-mode test that reads the painted cursor state (or a pixel probe
on macOS). Commit:

Phase 2 — draw the cursor in the macOS `TermView` paint and the Windows GDI paint
from the flag. Commit:
