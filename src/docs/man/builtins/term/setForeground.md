# setForeground

Set the foreground colour used for subsequently drawn text

## Synopsis

```
term::setForeground(r AS Byte, g AS Byte, b AS Byte) AS Nothing
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

`term::setForeground` sets the 24-bit RGB colour that subsequent text drawn
through the `term::` surface will be written in. The three channels — red, green,
blue — are each a `Byte` from 0 to 255, so (0, 0, 0) is black, (255, 255, 255) is
white, and (255, 0, 0) is pure red. Exactly three arguments are required.
[[src/builtins/term.rs:arity]] [[src/builtins/term.rs:param_types]]

The colour is packed into the module's current-attribute state and **no escape
sequence is emitted**. Like every other drawing operation on this retained
surface, the effect becomes visible only when `term::sync` presents the frame.
[[src/target/shared/code/term.rs:emit_set_color]]

Colour is per cell, not global. Each cell of the grid records the foreground,
background, bold, and underline that were current when its glyph was written, so
changing the foreground affects only text drawn *after* the call — text already in
the back buffer keeps the colour it was drawn with, and is not restyled.
[[src/target/shared/code/term_grid.rs:emit_grid_write]]

The setting persists until the next `term::setForeground` or the next
`term::on`, which resets the foreground to white (255, 255, 255). The background
colour and the bold and underline attributes are independent and are left
untouched; the current value can be read back with `term::getForeground`.

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
| `Nothing` | Returns nothing. The call is made for its side effect of setting the current foreground colour. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Draw red text and present the frame:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::setForeground(255, 0, 0)
  io::print("hello in red")
  term::sync()
  term::off()
END SUB
```

Two colours in one frame — the first line keeps its colour:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::setForeground(0, 255, 0)
  io::print("green")
  term::setForeground(0, 128, 255)
  io::print("blue")
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term getForeground`
- `mfb man term setBackground`
- `mfb man term setBold`
- `mfb man term sync`
