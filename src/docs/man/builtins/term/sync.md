# sync

Present the composed frame — the only call that puts drawing on screen

## Synopsis

```
term::sync() AS Nothing
```

## Package

term

## Imports

```
IMPORT term
```

`term` is a built-in package, so no manifest dependency is required.
[[src/builtins/term.rs:is_term_call]]

## Description

The `term::` surface is **retained and double-buffered**. While TUI mode is on,
every drawing call — `io::print` and `io::write`, and `term::clear`,
`term::moveTo`, the colour and attribute setters, `term::showCursor` and
`term::hideCursor` — mutates an in-memory cell grid and the current-attribute
state, and touches the terminal not at all. `term::sync` is the one and only
operation that presents a frame. **A program that draws without calling
`term::sync` shows nothing** — this is the single most common mistake when
writing against this surface.
[[src/target/shared/code/term_grid.rs:emit_grid_write]]

The present works by diffing. Each cell of the back buffer is compared against
the front buffer holding what was last shown, and only the cells that differ are
emitted — as a minimal stream of cursor moves, colour and attribute changes, and
glyphs, coalesced so an unchanged attribute is not re-sent. The whole stream is
built into a scratch buffer and issued as a **single batched write**, not a write
per cell, and each emitted cell is copied back to front so the next present diffs
from the frame actually on screen. The result is that repainting every frame
produces output proportional to what changed, and shows no flicker.
[[src/target/shared/code/term_grid.rs:emit_grid_present]]

That write is looped to completion. A short write advances the cursor and
re-issues until every byte lands, which matters here more than elsewhere: the
front buffer already claims those cells were painted, so a partially written
frame that was not finished would leave the screen wrong until something else
happened to mark the cells dirty again.

**A terminal resize is handled here.** On entry `term::sync` re-reads the
terminal size; if it changed, it allocates a new grid, copies the top-left overlap
so existing content survives, clamps the cursor into the new bounds, and forces a
full repaint of the next frame. If the re-read or the allocation fails, the old
grid is kept and the frame is presented into it unchanged.
[[src/target/shared/code/term_grid.rs:emit_grid_resize]]

After the changed cells, the present emits a trailing sequence that resets
attributes, moves the terminal cursor to the shadow cursor's position, and shows
or hides it according to `term::showCursor`/`term::hideCursor`. The first present
after `term::on` (or after a resize) is a full repaint because the grid is marked
dirty.

`term::sync` is gated: while TUI mode is off it is a clean no-op, so calling it
before `term::on` or after `term::off` is harmless. `term::off` performs a final
present of its own, so the last frame a program draws is always shown even
without an explicit `term::sync`. In app mode the call requests a single
coalesced redraw of the terminal view.
[[src/target/macos_aarch64/app/app_io.rs:emit_app_term_helper]]

## Parameters

`term::sync` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of presenting the frame. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors. A failure of the underlying terminal write is not reported as an
MFBASIC error. [[src/target/shared/code/term_grid.rs:emit_grid_present]]

## Examples

Compose a frame, then present it once:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::clear()
  term::moveTo(0, 0)
  io::print("hello")
  term::sync()
  term::off()
END SUB
```

The canonical render loop — draw the whole frame, present, then read input:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  MUT running AS Boolean = TRUE
  WHILE running
    term::clear()
    term::moveTo(0, 0)
    io::print("press q to quit")
    term::sync()
    IF io::pollInput(50) THEN
      running = io::readChar() <> "q"
    END IF
  END WHILE
  term::off()
END SUB
```

## See also

- `mfb man term on`
- `mfb man term off`
- `mfb man term clear`
- `mfb man term terminalSize`
