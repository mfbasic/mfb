# drawVLine

Draw a vertical box-drawing line down a column of the surface

## Synopsis

```
term::drawVLine(line AS LineStyle, col AS Integer, rowA AS Integer, rowB AS Integer) AS Nothing
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

`term::drawVLine` stamps a vertical run of a box-drawing glyph into the retained
surface: on column `col`, it fills every row from `rowA` to `rowB` with the
vertical form of the chosen `LineStyle`. The glyph is drawn with the colours and
attributes currently in effect (`term::setForeground`/`setBackground`/`setBold`/
`setUnderline`), exactly as `io::write` stamps text, and — like every drawing call
on this surface — it mutates the back buffer only and appears on the next
`term::sync`. [[src/target/shared/code/term.rs:emit_draw_line]]

Coordinates are **zero-based** and measured from the top-left corner: column 0 is
the leftmost column and row 0 is the topmost line. The two row endpoints may be
given in **either order** — `rowA` and `rowB` are normalised so the lower one
starts the run — and the run is **inclusive of both ends**. The span is then
**clamped to the surface**: a negative start becomes 0 and an end past the bottom
edge becomes `rows-1`. If `col` is outside `0 .. columns-1`, or the clamped span
covers no on-grid cell, the call draws nothing rather than clamping the line onto
an edge. No error is raised for an out-of-range request.
[[src/target/shared/code/term.rs:emit_draw_line]]

The `line` argument is a `LineStyle` enum value selecting the weight and pattern:
`LineStyle.Light` (`│`), `LineStyle.Heavy` (`┃`), `LineStyle.LightDash` (`┆`),
`LineStyle.HeavyDash` (`┇`), `LineStyle.LightDot` (`┊`), `LineStyle.HeavyDot`
(`┋`), and `LineStyle.Double` (`║`). `term::drawHLine` draws the matching
horizontal forms. [[src/codegen/builtins/term/mod.rs:LineStyle]]

Drawing a line does not move the shadow cursor and does not change the current
colours or attributes; it overwrites only the cells in the run, so a later draw
over the same cell (for example a crossing horizontal line) wins. The same surface
is rendered on the console backend and in windowed app mode, so the line looks the
same on both. [[src/target/shared/code/term_grid.rs:emit_grid_write]]

The call is gated: while TUI mode is off it does nothing and reports no error.
[[src/target/shared/code/term.rs:emit_gate_inactive]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `line` | `LineStyle` | The box-drawing weight/pattern; its vertical form is used. [[src/codegen/registry/mod.rs:call_param_names]] |
| `col` | `Integer` | Zero-based column the line is drawn on. Outside `0 .. columns-1` the call draws nothing. [[src/codegen/registry/mod.rs:call_param_names]] |
| `rowA` | `Integer` | One end of the row span (inclusive); may be greater or less than `rowB`. Clamped to the surface. [[src/codegen/registry/mod.rs:call_param_names]] |
| `rowB` | `Integer` | The other end of the row span (inclusive). Clamped to the surface. [[src/codegen/registry/mod.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of stamping the run into the back buffer. [[src/codegen/builtins/term/mod.rs:register]] |

## Errors

No errors.

## Examples

Draw a double vertical rule down the left edge of the surface:

```
IMPORT term

SUB main()
  term::on()
  LET size AS TermSize = term::terminalSize()
  term::drawVLine(LineStyle.Double, 0, 0, size.rows - 1)
  term::sync()
  term::off()
END SUB
```

Draw a vertical divider between two panes:

```
IMPORT term

SUB main()
  term::on()
  term::drawVLine(LineStyle.Light, 40, 0, 23)
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term drawHLine`
- `mfb man term types`
- `mfb man term setForeground`
- `mfb man term sync`
