# moveTo

Move the shadow cursor to a row and column of the surface

## Synopsis

```
term::moveTo(row AS Integer, column AS Integer) AS Nothing
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

`term::moveTo` sets the position at which the next text drawn through the
`term::` surface — including `io::print` and `io::write` — will start. Coordinates
are **zero-based** and measured from the top-left corner: row 0 is the topmost
line, column 0 is the leftmost column, and (0, 0) is the home position. The first
argument is always the row (vertical) and the second the column (horizontal).
[[src/target/shared/code/term.rs:emit_move_to]]

**Both coordinates are clamped at both ends**, on every backend. A negative value
becomes 0, and a value at or past the edge becomes the last valid cell — `rows-1`
for the row, `columns-1` for the column, using the current surface dimensions that
`term::terminalSize` reports. The cursor can therefore never be placed outside the
grid, and no error is raised for an out-of-range request.
[[src/target/shared/code/term.rs:emit_move_to]]

The move is retained, like everything else on this surface: it updates the shadow
cursor in the grid header and emits nothing to the terminal. The position is
honoured by the next glyph written and by the frame `term::sync` presents.
Moving the cursor draws nothing, erases nothing, and leaves the colours and
attributes alone. [[src/target/shared/code/term_grid.rs:emit_grid_write]]

Drawing advances the cursor on its own: each glyph moves it one column right,
wrapping to column 0 of the next row at the right edge and scrolling the surface
up by one row at the bottom. A line feed in the drawn text moves to column 0 of
the next row, a carriage return moves to column 0 of the same row, and
`io::print`'s trailing newline advances a row as well. `term::clear` homes the
cursor to (0, 0). [[src/target/shared/code/term_grid.rs:emit_scroll_back]]

The call is gated: while TUI mode is off it does nothing and reports no error.
[[src/target/shared/code/term.rs:emit_gate_inactive]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `row` | `Integer` | Zero-based row, counting from 0 at the top. Clamped to `0` at the low end and to `rows-1` at the high end. [[src/builtins/term.rs:call_param_names]] |
| `column` | `Integer` | Zero-based column, counting from 0 at the left. Clamped to `0` at the low end and to `columns-1` at the high end. [[src/builtins/term.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of moving the shadow cursor. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Draw at the top-left corner:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::clear()
  term::moveTo(0, 0)
  io::print("top-left")
  term::sync()
  term::off()
END SUB
```

Draw near the middle of the surface:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  LET size AS TermSize = term::terminalSize()
  term::moveTo(size.rows / 2, size.columns / 2)
  io::write("middle")
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term terminalSize`
- `mfb man term clear`
- `mfb man term sync`
- `mfb man term showCursor`
