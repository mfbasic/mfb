# fillRect

Fill a rectangular region with a block or shade glyph

## Synopsis

```
term::fillRect(fill AS FillStyle, x1 AS Integer, y1 AS Integer, x2 AS Integer, y2 AS Integer) AS Nothing
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

`term::fillRect` fills every cell of a rectangular region with a block or shade
glyph chosen by the `FillStyle` enum, using the colours and attributes currently
in effect. The two points `(x1, y1)` and `(x2, y2)` are **opposite corners** — `x`
is the column and `y` is the row, both **zero-based** from the top-left — and may
be given in any order. It is the region-filling counterpart to `term::clear`
(which blanks the whole surface): use it to paint a panel background, highlight a
band, or draw solid/█ and shaded/░▒▓ areas. The fill is shown on the next
`term::sync`. [[src/target/shared/code/term.rs:emit_fill_rect]]

The region is **clamped to the surface**, so a rectangle that runs off an edge
fills only the on-screen part, and one entirely off the surface fills nothing. No
error is raised for an out-of-range request. Filling does not move the shadow
cursor. [[src/target/shared/code/term.rs:emit_fill_rect]]

`FillStyle` selects the glyph: `Filled` (█, solid), `Light` (░), `Medium` (▒),
`Dark` (▓), and the two quadrant patterns `Checker` (▚) and `CheckerAlt` (▞). The
shade variants read as translucent overlays at a glance; the solid block is opaque.
The same surface renders identically on the console and in windowed app mode.
[[src/builtins/term.rs:FILL_STYLE_TYPE]]

The call is gated: while TUI mode is off it does nothing and reports no error.
[[src/target/shared/code/term.rs:emit_gate_inactive]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `fill` | `FillStyle` | The block or shade glyph stamped into every cell of the region. [[src/builtins/term.rs:call_param_names]] |
| `x1` | `Integer` | Column of the first corner (zero-based). Clamped to the surface. [[src/builtins/term.rs:call_param_names]] |
| `y1` | `Integer` | Row of the first corner (zero-based). Clamped to the surface. [[src/builtins/term.rs:call_param_names]] |
| `x2` | `Integer` | Column of the opposite corner; may be less or greater than `x1`. [[src/builtins/term.rs:call_param_names]] |
| `y2` | `Integer` | Row of the opposite corner; may be less or greater than `y1`. [[src/builtins/term.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of filling the region. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Paint a solid panel, then a lighter band inside it:

```
IMPORT term

SUB main()
  term::on()
  term::setBackground(0, 0, 40)
  term::fillRect(FillStyle.Filled, 2, 1, 30, 12)
  term::fillRect(FillStyle.Light, 4, 3, 28, 5)
  term::sync()
  term::off()
END SUB
```

Fill the whole surface as a background wash:

```
IMPORT term

SUB main()
  term::on()
  LET size AS TermSize = term::terminalSize()
  term::fillRect(FillStyle.Medium, 0, 0, size.columns - 1, size.rows - 1)
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term drawBox`
- `mfb man term clear`
- `mfb man term types`
- `mfb man term sync`
