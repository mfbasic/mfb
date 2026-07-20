# types

the term package record types

## Synopsis

```
term::TermColor
term::TermSize
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

## See also

- `mfb man term`
- `mfb man term getForeground`
- `mfb man term setForeground`
- `mfb man term terminalSize`
- `mfb man term moveTo`
