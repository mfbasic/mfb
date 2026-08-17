# isInputTerminal

Report whether standard input is an interactive terminal

## Synopsis

```
io::isInputTerminal() AS Boolean
```

## Package

io

## Imports

```
IMPORT io
```

`io` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/io/mod.rs:register]]

## Description

`io::isInputTerminal` returns `TRUE` when standard input is connected to a
terminal and `FALSE` when it is redirected from a file, a pipe, or any other
non-terminal source. It takes no arguments. [[src/codegen/builtins/io/mod.rs:register]]

The answer comes from an `isatty` probe of file descriptor 0: a result greater
than zero yields `TRUE`, anything else — including an error return — yields
`FALSE`. Because a failure is folded into `FALSE`, the call never raises.
[[src/codegen/builtins/io/native/terminal.rs:lower_io_is_terminal_helper]]

The probe inspects state only. It does not modify the stream, consume any input,
or block waiting for data, so it is safe to call before deciding whether to
prompt interactively, enable line editing, or read a piped stream straight
through.

In app mode (`mfb build --app`) the program has no real standard streams — input
is served by the application window, which is treated as an interactive console —
so this call returns `TRUE` without probing a descriptor.
[[src/target/shared/code/mod.rs:lower_runtime_helper]]

## Parameters

`io::isInputTerminal` takes no parameters. [[src/codegen/builtins/io/mod.rs:register]]

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when standard input is an interactive terminal; `FALSE` when it is a file, a pipe, or any other non-terminal source. Always `TRUE` in app mode. [[src/codegen/builtins/io/mod.rs:register]] |

## Errors

No errors.

## Examples

Prompt only when a human is attached, otherwise read the piped stream:

```
IMPORT io

SUB main()
  IF io::isInputTerminal() THEN
    io::print(io::input("Name: "))
  ELSE
    io::print(io::readLine())
  END IF
END SUB
```

## See also

- `mfb man io isOutputTerminal`
- `mfb man io isErrorTerminal`
- `mfb man io input`
- `mfb man io pollInput`
