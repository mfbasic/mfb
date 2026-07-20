# clear

Blank the whole back buffer and home the cursor

## Synopsis

```
term::clear() AS Nothing
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

`term::clear` blanks every cell of the `term::` back buffer and moves the shadow
cursor to the home position (row 0, column 0). It takes no arguments.
[[src/target/shared/code/term.rs:emit_clear_grid]]

Two details are easy to get wrong and worth stating plainly.

**The clear is a zero-fill, not a fill with the current background.** Every cell
is zeroed: a blank glyph, foreground 0, background 0. The cleared surface is
therefore black regardless of what `term::setBackground` was last set to. To get
a coloured background, set the colour and then draw over the region — the colour
is stamped into the cells that drawn text occupies, not into cells the clear
leaves behind.

**The clear does move the cursor.** It homes the shadow cursor to (0, 0), so a
following `term::moveTo(0, 0)` is redundant.

Like the rest of the surface, `term::clear` is retained: it mutates the back
buffer and emits nothing to the terminal. The blanked screen appears when the
program calls `term::sync`. It also leaves the *current* attributes alone — the
foreground, background, bold, underline, and cursor-visibility settings that
subsequent drawing will use are untouched; only the cells are.
[[src/target/shared/code/term_grid.rs:emit_grid_write]]

The call is gated: while TUI mode is off it does nothing. `term::on` already hands
back a cleared surface, so an explicit `term::clear` is for blanking again between
frames — which is exactly what the canonical render loop does.
[[src/target/shared/code/term.rs:emit_gate_inactive]]

## Parameters

`term::clear` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of blanking the back buffer and homing the cursor. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Blank the surface and draw from the top of each frame:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::clear()          ' also homes the cursor to (0, 0)
  io::print("a fresh screen")
  term::sync()
  term::off()
END SUB
```

Clear at the top of a render loop:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  LET rows AS List OF String = ["first", "second"]
  FOR EACH row IN rows
    term::clear()
    io::print(row)
    term::sync()
  NEXT
  term::off()
END SUB
```

## See also

- `mfb man term sync`
- `mfb man term moveTo`
- `mfb man term on`
- `mfb man term setBackground`
