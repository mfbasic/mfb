# isErrorTerminal

Report whether standard error is an interactive terminal

## Synopsis

```
io::isErrorTerminal() AS Boolean
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

`io::isErrorTerminal` returns `TRUE` when standard error is connected to a
terminal and `FALSE` when it is redirected to a file, a pipe, or any other
non-terminal destination. It takes no arguments. [[src/builtins/io.rs:arity]]

The answer comes from an `isatty` probe of file descriptor 2: a result greater
than zero yields `TRUE`, anything else — including an error return — yields
`FALSE`. Because a failure is folded into `FALSE`, the call never raises.
[[src/target/shared/code/io_helpers.rs:lower_io_is_terminal_helper]]

Standard error is probed independently of standard output, which matters in the
common case where one is redirected and the other is not: a program run as
`prog > out.txt` should still colour its diagnostics, and `prog 2> log.txt`
should not. Ask this question about the stream you are about to write to.

The probe inspects state only: it writes nothing and changes nothing.

In app mode (`mfb build --app`) the program has no real standard streams — error
output is rendered by the application transcript, which is treated as an
interactive console — so this call returns `TRUE` without probing a descriptor.
[[src/target/shared/code/mod.rs:lower_runtime_helper]]

## Parameters

`io::isErrorTerminal` takes no parameters. [[src/builtins/io.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when standard error is an interactive terminal; `FALSE` when it is a file, a pipe, or any other non-terminal destination. Always `TRUE` in app mode. [[src/builtins/io.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Colour diagnostics only when the error stream is a terminal:

```
IMPORT io

SUB main()
  IF io::isErrorTerminal() THEN
    io::printError("\u{1b}[31mError\u{1b}[0m: build failed")
  ELSE
    io::printError("Error: build failed")
  END IF
END SUB
```

## See also

- `mfb man io isOutputTerminal`
- `mfb man io isInputTerminal`
- `mfb man io printError`
- `mfb man io writeError`
