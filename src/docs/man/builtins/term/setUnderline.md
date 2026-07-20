# setUnderline

Turn the underline attribute on or off for subsequently drawn text

## Synopsis

```
term::setUnderline(enabled AS Boolean) AS Nothing
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

`term::setUnderline` sets whether text drawn through the `term::` surface from now
on is underlined. It takes exactly one `Boolean`: `TRUE` enables the attribute,
`FALSE` disables it.
[[src/builtins/term.rs:arity]] [[src/builtins/term.rs:param_types]]

The flag is stored in the module's current-attribute state and **no escape
sequence is emitted**. Like every other drawing operation on this retained
surface, the change becomes visible only when `term::sync` presents the frame.
[[src/target/shared/code/term.rs:emit_set_attr]]

Underlining is per cell, not global. Each cell of the grid records the foreground,
background, bold, and underline that were current when its glyph was written, so
this call affects text drawn *after* it; text already in the back buffer keeps the
attributes it was drawn with and is not restyled.
[[src/target/shared/code/term_grid.rs:emit_grid_write]]

The setting persists until the next `term::setUnderline` or the next `term::on`,
which resets underline to off. It is independent of the foreground and background
colours and of bold, so changing it leaves those alone, and the current value can
be read back with `term::getUnderline`. Setting the same value twice is harmless —
the state is a flag, not a toggle.

The call is gated: while TUI mode is off it does nothing and reports no error.
[[src/target/shared/code/term.rs:emit_gate_inactive]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `enabled` | `Boolean` | `TRUE` to draw subsequent text underlined, `FALSE` to draw it without an underline. [[src/builtins/term.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of setting the current underline attribute. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Underline one label, then continue normally:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::setUnderline(TRUE)
  io::print("Name")
  term::setUnderline(FALSE)
  io::print("Ada Lovelace")
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term getUnderline`
- `mfb man term setBold`
- `mfb man term setForeground`
- `mfb man term sync`
