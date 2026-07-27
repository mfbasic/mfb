# drawGlyph

Stamp a single glyph at a position by code point

## Synopsis

```
term::drawGlyph(x AS Integer, y AS Integer, codepoint AS Integer) AS Nothing
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

`term::drawGlyph` stamps a single Unicode scalar — given by its `codepoint` — into
the cell at column `x`, row `y`, using the colours and attributes currently in
effect. Coordinates are **zero-based** from the top-left. It does not move the
shadow cursor. This is the low-level counterpart to `term::drawText`: use it to
place one arbitrary character (a marker, a cursor, a sprite cell) at a known
position. [[src/target/shared/code/term.rs:emit_draw_glyph]]

The cell is **clamped to the surface**: if `(x, y)` is off the grid the call draws
nothing, and no error is raised. Control code points (below U+0020) are **skipped**
— they would corrupt the presented frame — so `codepoint` should be a printable
scalar (for example `9731` for `☃`, or `65` for `A`). The glyph is shown on the
next `term::sync`. [[src/target/shared/code/term.rs:emit_draw_glyph]]

The call is gated: while TUI mode is off it does nothing and reports no error.
[[src/target/shared/code/term.rs:emit_gate_inactive]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `x` | `Integer` | Zero-based column. Off-grid cells draw nothing. [[src/builtins/term.rs:call_param_names]] |
| `y` | `Integer` | Zero-based row. Off-grid cells draw nothing. [[src/builtins/term.rs:call_param_names]] |
| `codepoint` | `Integer` | The Unicode scalar to stamp. Control code points (< 0x20) are skipped. [[src/builtins/term.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of stamping the cell. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Place a marker character at the centre of the surface:

```
IMPORT term

SUB main()
  term::on()
  LET size AS TermSize = term::terminalSize()
  term::drawGlyph(size.columns / 2, size.rows / 2, 9731) ' ☃
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term drawText`
- `mfb man term moveTo`
- `mfb man term sync`
