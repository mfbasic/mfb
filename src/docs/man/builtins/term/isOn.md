# isOn

Report whether TUI mode is currently on

## Synopsis

```
term::isOn() AS Boolean
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

`term::isOn` returns `TRUE` while the `term::` surface is active — after
`term::on` and before the matching `term::off` — and `FALSE` otherwise, including
before any `term::on` call. It takes no arguments.
[[src/target/shared/code/term.rs:emit_is_on]]

`term::isOn` and `term::on` are the only two calls in the module that are **not
gated**. Every other `term::` call short-circuits while TUI mode is off: the
setters and `term::clear`, `term::moveTo`, and `term::sync` do nothing,
`term::getForeground`/`getBackground`/`getBold`/`getUnderline` return inert
defaults rather than live state, and `term::terminalSize` raises
`ErrUnsupported`. That is what makes this query useful: it is the way to find out
whether the rest of the surface will actually do anything.
[[src/target/shared/code/term.rs:emit_gate_inactive]]

The result is the module's active flag read directly, so it changes only at
`term::on` and `term::off`. `term::off` while already off leaves the flag alone;
`term::on` while already on re-runs its setup but the flag stays `TRUE`
throughout.

The call reads state only: it touches neither the terminal, the alternate screen,
nor the shadow grid, and it cannot fail.

## Parameters

`term::isOn` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when TUI mode is on, `FALSE` otherwise. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Enter TUI mode only once:

```
IMPORT term

SUB main()
  IF NOT term::isOn() THEN
    term::on()
  END IF
END SUB
```

Draw only when the surface is live:

```
IMPORT term
IMPORT io

SUB main()
  IF term::isOn() THEN
    term::clear()
    term::moveTo(0, 0)
    io::print("status")
    term::sync()
  END IF
END SUB
```

## See also

- `mfb man term on`
- `mfb man term off`
- `mfb man term sync`
