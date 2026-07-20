# getForeground

Read the current foreground colour as a `TermColor`

## Synopsis

```
term::getForeground() AS TermColor
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

`term::getForeground` returns the colour that subsequently drawn text will be
written in, as a freshly allocated `TermColor` record with three `Byte` fields
`r`, `g`, and `b` holding the red, green, and blue channels. It takes no
arguments. [[src/builtins/term.rs:builtin_type_fields]]

The value is the module's current foreground attribute, unpacked from the 24-bit
value that `term::setForeground` stored. Immediately after `term::on` — which
resets the foreground to white — it is (255, 255, 255); after a
`term::setForeground` call it is exactly the triple that was set, until the next
`term::setForeground` or the next `term::on`.
[[src/target/shared/code/term.rs:emit_get_color]]

This is the *current attribute*, not the colour of anything on screen. Each cell
of the grid carries the attributes that were current when its glyph was written,
so this call says what the next drawing will use, not what the cell under the
cursor looks like.

Unlike most of the module, `term::getForeground` does not simply do nothing while
TUI mode is off: it returns the **inert default**, white (255, 255, 255). A
program cannot distinguish "off" from "on and set to white" by this call alone —
use `term::isOn` for that. [[src/target/shared/code/term.rs:emit_gate_inactive]]

The call reads state only: it changes no `term::` state, moves no cursor, and
draws nothing. It can still fail, because the returned record has to be
allocated.

## Parameters

`term::getForeground` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `TermColor` | A record whose `r`, `g`, and `b` `Byte` fields are the channels of the current foreground colour, each 0 to 255. White (255, 255, 255) immediately after `term::on`, and white while TUI mode is off. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The returned `TermColor` record cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Set a colour and read it back:

```
IMPORT term

SUB main()
  term::on()
  term::setForeground(0, 128, 255)
  LET c AS TermColor = term::getForeground()
  term::off()
END SUB
```

Save the current colour, draw a highlight, then restore it:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  LET saved AS TermColor = term::getForeground()
  term::setForeground(255, 0, 0)
  io::print("warning")
  term::setForeground(saved.r, saved.g, saved.b)
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term setForeground`
- `mfb man term getBackground`
- `mfb man term getBold`
- `mfb man term isOn`
