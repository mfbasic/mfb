# getBackground

Read the current background colour as a `TermColor`

## Synopsis

```
term::getBackground() AS TermColor
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

`term::getBackground` returns the colour drawn behind subsequently written text,
as a freshly allocated `TermColor` record with three `Byte` fields `r`, `g`, and
`b` holding the red, green, and blue channels. It takes no arguments.
[[src/builtins/term.rs:builtin_type_fields]]

The value is the module's current background attribute, unpacked from the 24-bit
value that `term::setBackground` stored. Immediately after `term::on` — which
resets the background to black — it is (0, 0, 0); after a `term::setBackground`
call it is exactly the triple that was set, until the next `term::setBackground`
or the next `term::on`. [[src/target/shared/code/term.rs:emit_get_color]]

This is the *current attribute*, not the colour of anything on screen. Each cell
of the grid carries the attributes that were current when its glyph was written,
so this call says what the next drawing will use. Note in particular that
`term::clear` zero-fills the grid rather than painting it with this colour, so a
cleared surface is black whatever `term::getBackground` reports.
[[src/target/shared/code/term.rs:emit_clear_grid]]

Unlike most of the module, `term::getBackground` does not simply do nothing while
TUI mode is off: it returns the **inert default**, black (0, 0, 0). A program
cannot distinguish "off" from "on and set to black" by this call alone — use
`term::isOn` for that. [[src/target/shared/code/term.rs:emit_gate_inactive]]

The call reads state only: it changes no `term::` state, moves no cursor, and
draws nothing. It can still fail, because the returned record has to be
allocated.

## Parameters

`term::getBackground` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `TermColor` | A record whose `r`, `g`, and `b` `Byte` fields are the channels of the current background colour, each 0 to 255. Black (0, 0, 0) immediately after `term::on`, and black while TUI mode is off. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The returned `TermColor` record cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Set a background colour and read it back:

```
IMPORT term

SUB main()
  term::on()
  term::setBackground(0, 128, 255)
  LET c AS TermColor = term::getBackground()
  term::off()
END SUB
```

Inspect the individual channels:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  LET c AS TermColor = term::getBackground()
  term::off()
  io::print(toString(c.r) & "," & toString(c.g) & "," & toString(c.b))
END SUB
```

## See also

- `mfb man term setBackground`
- `mfb man term getForeground`
- `mfb man term clear`
- `mfb man term isOn`
