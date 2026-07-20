# pollInput

Test whether standard input is ready to read, optionally waiting up to a timeout

## Synopsis

```
io::pollInput() AS Boolean
io::pollInput(timeoutMs AS Integer) AS Boolean
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

`io::pollInput` reports whether a following read of standard input can proceed
without blocking. It returns `TRUE` when input is ready and `FALSE` when the wait
elapses first, and it **consumes nothing** — the bytes are still there for
`io::readLine`, `io::readChar`, `io::readByte`, or `io::input`.
[[src/target/shared/code/io_helpers.rs:lower_io_poll_input_helper]]

`timeoutMs` is in milliseconds and is passed straight through to the underlying
`poll`, so it follows that call's convention:

- **negative** — wait indefinitely until input is ready;
- **zero** — check readiness and return immediately;
- **positive** — wait up to that many milliseconds.

When the argument is omitted the compiler supplies `0`, so `io::pollInput()` is a
non-blocking check. [[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]]

Readiness is answered in two stages. Standard input is served from a per-thread
broadcast log, and a byte already staged there for this thread is invisible to a
`poll` of file descriptor 0 — so the log is consulted first, and a staged byte (or
a reached end-of-input offset) reports `TRUE` at once with no system call. Only
when the log holds nothing for this thread does the call `poll` file descriptor 0.
A thread that has not subscribed to standard input simply defers to that `poll`;
unlike the read calls, `io::pollInput` does not raise `ErrInvalidContext`.
[[src/target/shared/code/stdin_broadcast.rs:emit_stdin_poll_ready_check]]

**End of input counts as ready.** A stream at end of input is reported readable,
so `io::pollInput` returns `TRUE` and the following read then raises `ErrEof`.
A `TRUE` result therefore promises that the next read will not block, not that it
will succeed.

A signal delivered while the call is blocked (`SIGWINCH` from a terminal resize,
`SIGCHLD`, the console interrupt handler) is not an error: the `poll` is re-armed
and retried rather than surfacing as `ErrInput`.
[[src/target/shared/code/io_helpers.rs:lower_io_poll_input_helper]]

On a terminal in the default canonical mode, the line discipline holds typed
characters until Return, so readiness is reported per line rather than per key.
Enter `term::on`'s single-key mode, or use `io::readChar`/`io::readByte`, when a
poll should see individual keypresses.

## Overloads

**`io::pollInput() AS Boolean`**

Non-blocking readiness check; equivalent to `io::pollInput(0)`.
[[src/builtins/io.rs:arity]]

**`io::pollInput(timeoutMs AS Integer) AS Boolean`**

Waits up to `timeoutMs` milliseconds, indefinitely when the value is negative and
not at all when it is zero.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `timeoutMs` | `Integer` | Maximum wait in milliseconds: negative waits forever, `0` returns immediately, positive bounds the wait. Defaults to `0` when omitted. [[src/builtins/io.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when standard input is ready to read — including when it has reached end of input — before the timeout elapses; `FALSE` when the wait elapses with nothing available. [[src/builtins/io.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020005` | `ErrInput` | The poll of standard input fails for a reason other than an interrupting signal, which is retried instead. [[src/target/shared/code/error_constants.rs:ERR_INPUT_CODE]] |

## Examples

Read a line only when one is already pending:

```
IMPORT io

SUB main()
  IF io::pollInput() THEN
    io::print(io::readLine())
  END IF
END SUB
```

Wait up to a second for a keypress:

```
IMPORT io

SUB main()
  IF io::pollInput(1000) THEN
    io::print(io::readChar())
  ELSE
    io::print("timeout")
  END IF
END SUB
```

Block until input arrives, then take one byte:

```
IMPORT io

SUB main()
  IF io::pollInput(-1) THEN
    io::print(toString(io::readByte()))
  END IF
END SUB
```

## See also

- `mfb man io readChar`
- `mfb man io readByte`
- `mfb man io readLine`
- `mfb man io input`
