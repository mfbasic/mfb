# term

Full-screen terminal TUI surface: cursor, colors, attributes, and clearing

## Synopsis

```
IMPORT term
term::on()
term::moveTo(row, column)
term::setForeground(r, g, b)
term::clear()
term::sync()
term::off()
```

## Description

The `term` package gives a program a structured, full-screen terminal surface
for text user interfaces: it moves the cursor, sets the foreground and
background colors and the bold and underline attributes, clears the screen,
shows or hides the cursor, and reports the surface size. The same surface is
rendered on the console backend (using the terminal's alternate screen and ANSI
sequences) and in windowed app mode (`mfb build --app`), so a program draws the
same way on both.

`term::on` is the gate for the whole module. It switches the terminal into TUI
mode and resets all `term::` state to its defaults (white foreground, black
background, bold and underline off, cursor visible, screen cleared, cursor at
the home position). Every other `term::` call except `term::isOn` is a no-op
while TUI mode is off, so a program must call `term::on` before any cursor,
color, attribute, or clear call takes effect, and `term::off` later leaves TUI
mode and restores the user's previous screen. `term::isOn` reports whether TUI
mode is currently on and works whether or not it is. [[src/builtins/term.rs:ON]]

While TUI mode is on the surface is **retained** and **double-buffered**: drawing
calls (including `io::print`/`io::write`) mutate an in-memory cell grid rather
than the terminal, and nothing appears until the program calls `term::sync`, the
one operation that presents a frame. The console backend presents by writing only
the cells that changed since the previous frame, so a program that repaints every
frame shows no flicker and emits output proportional to what actually changed; in
app mode `term::sync` coalesces the frame into a single redraw. `term::off`
performs a final `term::sync` before restoring the screen, so the last frame is
always shown. A program that draws without a following `term::sync` displays
nothing - the canonical shape is to compose a whole frame, call `term::sync`
once, then read input. [[src/builtins/term.rs:SYNC]]

Coordinates are zero-based and measured from the top-left corner of the surface:
row 0 is the topmost line and column 0 is the leftmost column, so (0, 0) is the
home position. The first coordinate is always the row (vertical) and the second
the column (horizontal). Negative coordinates are clamped to 0; in app mode they
are also clamped at the high end to the last valid cell. Colors are 24-bit RGB
triples of three `Byte` channels (red, green, blue), each 0 to 255. Color and
attribute changes take effect immediately for subsequently drawn text and do not
alter text already on the screen; each setting is independent, so changing one
leaves the others untouched, and the matching get function reads the current
value back. [[src/builtins/term.rs:MOVE_TO]]

The package defines two built-in record types. `TermColor` has three `Byte`
fields `r`, `g`, and `b` holding the red, green, and blue channels of a color,
and is returned by `term::getForeground` and `term::getBackground`. `TermSize`
has two `Integer` fields `columns` (the width of the surface in character cells)
and `rows` (its height), and is returned by `term::terminalSize`; the surface
size can change between calls (for example when the terminal window is resized),
so a program that depends on it should query it again rather than caching the
result. [[src/builtins/term.rs:builtin_type_fields]]

## Errors

No errors.
