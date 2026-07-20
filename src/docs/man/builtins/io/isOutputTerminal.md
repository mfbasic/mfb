# isOutputTerminal

Report whether standard output is an interactive terminal

## Synopsis

```
io::isOutputTerminal() AS Boolean
```

## Package

io

## Imports

```
IMPORT io
```

`io` is a built-in package, so no manifest dependency is required.
[[src/builtins/io.rs:is_io_call]]

## Description

`io::isOutputTerminal` returns `TRUE` when standard output is connected to a
terminal and `FALSE` when it is redirected to a file, a pipe, or any other
non-terminal destination. It takes no arguments. [[src/builtins/io.rs:arity]]

The answer comes from an `isatty` probe of file descriptor 1: a result greater
than zero yields `TRUE`, anything else — including an error return — yields
`FALSE`. Because a failure is folded into `FALSE`, the call never raises.
[[src/target/shared/code/io_helpers.rs:lower_io_is_terminal_helper]]

The probe inspects state only: it writes nothing and changes nothing. Use it to
decide whether emitting ANSI colour, progress bars, or cursor tricks is
appropriate, and to fall back to plain text when output is being captured.

Note that the answer says nothing about `io::setBuffered`: buffering is an
MFBASIC-level setting the program controls, not something inferred from whether
standard output is a terminal.

In app mode (`mfb build --app`) the program has no real standard streams — output
is rendered by the application transcript window, which is treated as an
interactive console — so this call returns `TRUE` without probing a descriptor.
[[src/target/shared/code/mod.rs:lower_runtime_helper]]

## Parameters

`io::isOutputTerminal` takes no parameters. [[src/builtins/io.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when standard output is an interactive terminal; `FALSE` when it is a file, a pipe, or any other non-terminal destination. Always `TRUE` in app mode. [[src/builtins/io.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Colour the output only when a terminal is attached:

```
IMPORT io

SUB main()
  IF io::isOutputTerminal() THEN
    io::print("\u{1b}[32mStatus: OK\u{1b}[0m")
  ELSE
    io::print("Status: OK")
  END IF
END SUB
```

## See also

- `mfb man io isInputTerminal`
- `mfb man io isErrorTerminal`
- `mfb man io print`
- `mfb man term on`
