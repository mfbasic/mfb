# isBuffered

Report whether standard-output buffering is enabled for this thread

## Synopsis

```
io::isBuffered() AS Boolean
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

`io::isBuffered` returns `TRUE` when opt-in standard-output buffering is on for
the calling thread and `FALSE` otherwise. It takes no arguments.
[[src/builtins/io.rs:arity]]

The result is the thread's buffering flag read directly: `TRUE` after
`io::setBuffered(TRUE)`, `FALSE` after `io::setBuffered(FALSE)`. Buffering is off
by default, so a program that never calls `io::setBuffered` always observes
`FALSE`. [[src/target/shared/code/io_stdout.rs:lower_io_is_buffered_helper]]

The flag is per thread — each thread has its own standard-output buffer and its
own enabled state — so this call never reports another thread's mode. Standard
error is never buffered and has no corresponding query.

The call reads state only: it writes nothing, drains nothing, and cannot fail.

In app mode the standard-output buffer is inert, so `io::isBuffered` always
reports `FALSE` there regardless of any `io::setBuffered` call.
[[src/target/shared/code/io_stdout.rs:lower_io_is_buffered_helper]]

## Parameters

`io::isBuffered` takes no parameters. [[src/builtins/io.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when standard-output buffering is enabled for this thread, `FALSE` otherwise — including before it has ever been enabled, and always in app mode. [[src/builtins/io.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Enable buffering only when it is not already on:

```
IMPORT io

SUB main()
  IF NOT io::isBuffered() THEN
    io::setBuffered(TRUE)
  END IF
END SUB
```

Capture the mode so it can be restored later:

```
IMPORT io

SUB emitReport()
END SUB

SUB main()
  LET wasBuffered AS Boolean = io::isBuffered()
  io::setBuffered(TRUE)
  emitReport()
  io::setBuffered(wasBuffered)
END SUB
```

## See also

- `mfb man io setBuffered`
- `mfb man io flush`
- `mfb man io print`
