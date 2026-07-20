# setBackground

Set the background colour used for subsequently drawn text

## Synopsis

```
term::setBackground(r AS Byte, g AS Byte, b AS Byte) AS Nothing
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

`term::setBackground` sets the 24-bit RGB colour drawn behind subsequent text on
the `term::` surface. The three channels — red, green, blue — are each a `Byte`
from 0 to 255, so (0, 0, 0) is black and (255, 255, 255) is white. Exactly three
arguments are required.
[[src/builtins/term.rs:arity]] [[src/builtins/term.rs:param_types]]

The colour is packed into the module's current-attribute state and **no escape
sequence is emitted**; the effect becomes visible when `term::sync` presents the
frame. [[src/target/shared/code/term.rs:emit_set_color]]

Background colour is per cell, and it colours only the cells that drawn text
occupies. Each cell records the attributes current when its glyph was written, so
this call affects text drawn *after* it and does not restyle what is already in
the back buffer. In particular, **`term::clear` does not paint the current
background**: it zero-fills the grid, which is black regardless of this setting.
To get a coloured region, set the background and then draw over it — for example
by writing spaces across the cells you want filled.
[[src/target/shared/code/term.rs:emit_clear_grid]]

The setting persists until the next `term::setBackground` or the next `term::on`,
which resets the background to black (0, 0, 0). The foreground colour and the
bold and underline attributes are independent and are left untouched; the current
value can be read back with `term::getBackground`.

The call is gated: while TUI mode is off it does nothing and reports no error.
[[src/target/shared/code/term.rs:emit_gate_inactive]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `r` | `Byte` | Red channel, 0 to 255. [[src/builtins/term.rs:call_param_names]] |
| `g` | `Byte` | Green channel, 0 to 255. [[src/builtins/term.rs:call_param_names]] |
| `b` | `Byte` | Blue channel, 0 to 255. [[src/builtins/term.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of setting the current background colour. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Draw text on a blue background:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::setBackground(0, 0, 255)
  io::print("hello on blue")
  term::sync()
  term::off()
END SUB
```

Fill a banner row by drawing spaces over it:

```
IMPORT term
IMPORT io
IMPORT strings

SUB main()
  term::on()
  LET size AS TermSize = term::terminalSize()
  term::setBackground(0, 0, 128)
  term::moveTo(0, 0)
  io::write(strings::repeat(" ", size.columns))
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term getBackground`
- `mfb man term setForeground`
- `mfb man term clear`
- `mfb man term sync`
