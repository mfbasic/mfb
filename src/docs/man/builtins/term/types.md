# types

the term package record and enum types

## Synopsis

```
term::TermColor
term::TermSize
term::LineStyle
term::FillStyle
```

## Package

term

## Imports

```
IMPORT term
```

`term` is a built-in package, so `IMPORT term` needs no manifest
dependency. [[src/builtins/term.rs:is_term_call]]

## Description

The `term` package defines two record types, `TermColor` and `TermSize`. Both are
built-in record types, recognized once `IMPORT term` is in scope; either spelling
resolves, but the conventional one is bare
(`LET fg AS TermColor = term::getForeground()`) rather than package-qualified. Both
are flat, copyable value records of scalar fields: they hold no resource and no
hidden state, so they copy freely, drop with no heap frees, and are
thread-sendable. Neither is constructed by the program — each is produced by the
`term::` query that returns it and then read with ordinary field
access. [[src/builtins/term.rs:builtin_type_fields]]

`TermColor` is a 24-bit RGB color, three `Byte` channels of 0 to 255. It is
returned by `term::getForeground` and `term::getBackground`, which read back the
color currently in effect for subsequently drawn text. The matching setters take
the three channels as separate `Byte` arguments rather than a record, so a color
read back from a getter is re-applied field by field:
`term::setForeground(c.r, c.g, c.b)`. [[src/builtins/term.rs:TERM_COLOR_TYPE]]

`TermSize` is the size of the drawing surface in character cells, returned by
`term::terminalSize`. The surface size can change between calls — for example when
the user resizes the terminal window — so a program that depends on it should query
it again each frame rather than caching the
result. [[src/builtins/term.rs:TERM_SIZE_TYPE]]

`LineStyle` is an enum selecting the box-drawing weight and pattern for
`term::drawHLine`, `term::drawVLine`, and `term::drawBox`. Its members are
addressed as `LineStyle.Light`, `LineStyle.Heavy`, and so on. Each variant has a
horizontal form (used by `drawHLine`) and a vertical form (used by `drawVLine`);
the two functions pick the right form for their orientation, and `drawBox` uses
both plus the matching corners. [[src/builtins/term.rs:LINE_STYLE_TYPE]]

`FillStyle` is an enum selecting the block or shade glyph for `term::fillRect`,
addressed as `FillStyle.Filled`, `FillStyle.Light`, and so on. [[src/builtins/term.rs:FILL_STYLE_TYPE]]

Coordinates elsewhere in the package are zero-based from the top-left corner, so
on a surface of `columns` by `rows` the valid cells are columns `0 .. columns - 1`
and rows `0 .. rows - 1`. [[src/builtins/term.rs:MOVE_TO]]

## Types

### term::TermColor

A 24-bit RGB color. Returned by `term::getForeground` and `term::getBackground`. [[src/builtins/term.rs:TERM_COLOR_TYPE]]

| Field | Type | Description |
| --- | --- | --- |
| `r` | `Byte` | Red channel, `0 .. 255`. |
| `g` | `Byte` | Green channel, `0 .. 255`. |
| `b` | `Byte` | Blue channel, `0 .. 255`. |

### term::TermSize

The size of the terminal surface in character cells. Returned by `term::terminalSize`. [[src/builtins/term.rs:TERM_SIZE_TYPE]]

| Field | Type | Description |
| --- | --- | --- |
| `columns` | `Integer` | Width of the surface in character cells; the valid column indices are `0 .. columns - 1`. |
| `rows` | `Integer` | Height of the surface in character cells; the valid row indices are `0 .. rows - 1`. |

### term::LineStyle

The box-drawing weight/pattern for `term::drawHLine` and `term::drawVLine`. Each
variant has a horizontal and a vertical form. [[src/builtins/term.rs:LINE_STYLE_TYPE]]

| Variant | Horizontal | Vertical | Description |
| --- | --- | --- | --- |
| `Light` | `─` | `│` | Thin single line. |
| `Heavy` | `━` | `┃` | Thick single line. |
| `LightDash` | `┄` | `┆` | Thin triple-dash line. |
| `HeavyDash` | `┅` | `┇` | Thick triple-dash line. |
| `LightDot` | `┈` | `┊` | Thin quadruple-dot line. |
| `HeavyDot` | `┉` | `┋` | Thick quadruple-dot line. |
| `Double` | `═` | `║` | Double line. |

### term::FillStyle

The block or shade glyph `term::fillRect` stamps into every cell of a region. [[src/builtins/term.rs:FILL_STYLE_TYPE]]

| Variant | Glyph | Description |
| --- | --- | --- |
| `Filled` | `█` | Solid full block. |
| `Light` | `░` | Light shade. |
| `Medium` | `▒` | Medium shade. |
| `Dark` | `▓` | Dark shade. |
| `Checker` | `▚` | Upper-left + lower-right quadrants. |
| `CheckerAlt` | `▞` | Upper-right + lower-left quadrants. |

## See also

- `mfb man term`
- `mfb man term getForeground`
- `mfb man term setForeground`
- `mfb man term terminalSize`
- `mfb man term moveTo`
