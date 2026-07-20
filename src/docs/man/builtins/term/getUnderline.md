# getUnderline

Report whether the underline attribute is currently set

## Synopsis

```
term::getUnderline() AS Boolean
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

`term::getUnderline` returns `TRUE` when subsequently drawn text will be
underlined and `FALSE` when it will not. It takes no arguments.
[[src/builtins/term.rs:arity]]

The value is the module's current underline attribute read directly. Immediately
after `term::on` — which resets underline to off — it is `FALSE`; afterwards it is
whatever the most recent `term::setUnderline` passed, until the next
`term::setUnderline` or the next `term::on`.
[[src/target/shared/code/term.rs:emit_get_attr]]

This is the *current attribute*, not a property of anything on screen. Each cell
of the grid carries the attributes that were current when its glyph was written,
so this call describes what the next drawing will use.

Unlike most of the module, `term::getUnderline` does not simply do nothing while
TUI mode is off: it returns the **inert default**, `FALSE`. A program cannot
distinguish "off" from "on with underline disabled" by this call alone — use
`term::isOn` for that. [[src/target/shared/code/term.rs:emit_gate_inactive]]

The call reads state only. It allocates nothing, changes no `term::` state, draws
nothing, and cannot fail.

## Parameters

`term::getUnderline` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when the underline attribute is set for subsequently drawn text, `FALSE` otherwise — including whenever TUI mode is off. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Set underline and read it back:

```
IMPORT term

SUB main()
  term::on()
  term::setUnderline(TRUE)
  LET u AS Boolean = term::getUnderline()
  term::off()
END SUB
```

Branch on the current state:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  IF term::getUnderline() THEN
    io::print("underlined")
  ELSE
    io::print("plain")
  END IF
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term setUnderline`
- `mfb man term getBold`
- `mfb man term getBackground`
- `mfb man term isOn`
