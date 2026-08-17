# drawBox

Draw a rectangular box in a box-drawing style

## Synopsis

```
term::drawBox(line AS LineStyle, x1 AS Integer, y1 AS Integer, x2 AS Integer, y2 AS Integer) AS Nothing
```

## Package

term

## Imports

```
IMPORT term
```

`term` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/term/mod.rs:register]]

## Description

`term::drawBox` draws a rectangle into the retained surface in the chosen
`LineStyle`. The two points `(x1, y1)` and `(x2, y2)` are **opposite corners** —
`x` is the column and `y` is the row, both **zero-based** from the top-left — and
they may be given in any order. The box is drawn as the four edges followed by the
four corners: the top and bottom rows are horizontal runs and the left and right
columns are vertical runs, each using this style's own line glyph, and then the
four corner cells are overwritten with the matching corner glyph. Everything is
stamped with the colours and attributes currently in effect and shown on the next
`term::sync`. [[src/target/shared/code/term.rs:emit_draw_box]]

Because the edges use the style's line glyph, a **dashed or dotted** style draws
dashed or dotted edges — but those styles have no dashed corner glyphs, so the
corners fall back to the solid **Light** or **Heavy** corner of the same weight
(`Double` uses the double corners). So `LineStyle.LightDash` draws `┄`/`┆` edges
with `┌┐└┘` corners, and `LineStyle.HeavyDot` draws `┉`/`┋` edges with `┏┓┗┛`
corners. [[src/codegen/builtins/term/mod.rs:LineStyle]]

Each edge and each corner is **clamped to the surface independently**, so a box
that runs off one side still draws the parts that are on-screen (including the
edges along the visible sides), and a box entirely off the surface draws nothing.
No error is raised for an out-of-range request. A one-cell-wide or one-cell-tall
box collapses to a line or a single cell, with the corners drawn last. The same
surface renders identically on the console and in windowed app mode.
[[src/target/shared/code/term.rs:emit_draw_box]]

The call is gated: while TUI mode is off it does nothing and reports no error.
[[src/target/shared/code/term.rs:emit_gate_inactive]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `line` | `LineStyle` | The box-drawing style; the edges use its line glyph and the corners the matching Light/Heavy/Double corner. [[src/codegen/registry/mod.rs:call_param_names]] |
| `x1` | `Integer` | Column of the first corner (zero-based). Clamped to the surface. [[src/codegen/registry/mod.rs:call_param_names]] |
| `y1` | `Integer` | Row of the first corner (zero-based). Clamped to the surface. [[src/codegen/registry/mod.rs:call_param_names]] |
| `x2` | `Integer` | Column of the opposite corner; may be less or greater than `x1`. [[src/codegen/registry/mod.rs:call_param_names]] |
| `y2` | `Integer` | Row of the opposite corner; may be less or greater than `y1`. [[src/codegen/registry/mod.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of stamping the box into the back buffer. [[src/codegen/builtins/term/mod.rs:register]] |

## Errors

No errors.

## Examples

Draw a light box near the top-left corner:

```
IMPORT term

SUB main()
  term::on()
  term::drawBox(LineStyle.Light, 2, 1, 20, 8)
  term::sync()
  term::off()
END SUB
```

Frame the whole surface with a double-line border:

```
IMPORT term

SUB main()
  term::on()
  LET size AS TermSize = term::terminalSize()
  term::drawBox(LineStyle.Double, 0, 0, size.columns - 1, size.rows - 1)
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term drawHLine`
- `mfb man term drawVLine`
- `mfb man term types`
- `mfb man term sync`
