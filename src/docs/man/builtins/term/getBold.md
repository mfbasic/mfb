# getBold

Report whether the bold attribute is currently set

## Synopsis

```
term::getBold() AS Boolean
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

`term::getBold` returns `TRUE` when subsequently drawn text will be bold and
`FALSE` when it will not. It takes no arguments.
[[src/builtins/term.rs:arity]]

The value is the module's current bold attribute read directly. Immediately after
`term::on` — which resets bold to off — it is `FALSE`; afterwards it is whatever
the most recent `term::setBold` passed, until the next `term::setBold` or the
next `term::on`. [[src/target/shared/code/term.rs:emit_get_attr]]

This is the *current attribute*, not a property of anything on screen. Each cell
of the grid carries the attributes that were current when its glyph was written,
so this call describes what the next drawing will use.

Unlike most of the module, `term::getBold` does not simply do nothing while TUI
mode is off: it returns the **inert default**, `FALSE`. A program cannot
distinguish "off" from "on with bold disabled" by this call alone — use
`term::isOn` for that. [[src/target/shared/code/term.rs:emit_gate_inactive]]

The call reads state only. It allocates nothing, changes no `term::` state, draws
nothing, and cannot fail.

## Parameters

`term::getBold` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when the bold attribute is set for subsequently drawn text, `FALSE` otherwise — including whenever TUI mode is off. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Set bold and read it back:

```
IMPORT term

SUB main()
  term::on()
  term::setBold(TRUE)
  LET b AS Boolean = term::getBold()
  term::off()
END SUB
```

Toggle the attribute from its current value:

```
IMPORT term

SUB main()
  term::on()
  term::setBold(NOT term::getBold())
  term::off()
END SUB
```

## See also

- `mfb man term setBold`
- `mfb man term getUnderline`
- `mfb man term getForeground`
- `mfb man term isOn`
