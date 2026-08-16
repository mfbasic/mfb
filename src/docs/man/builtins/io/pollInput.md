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
[[src/codegen/builtins/io/mod.rs:register]]

## Description

`io::pollInput` reports whether a following read of standard input can proceed
without blocking. It returns `TRUE` when input is ready and `FALSE` when the wait
elapses first, and it **consumes nothing** — the bytes are still there for
`io::readLine`, `io::readChar`, `io::readByte`, or `io::input`.
[[src/codegen/builtins/io/native/stdin.rs:lower_io_poll_input_helper]]

`timeoutMs` bounds the wait, in milliseconds, following the language timeout
convention (see `mfb spec language builtin-functions` → "Timeout convention").
When it is **omitted, `pollInput` blocks** until standard input becomes ready and
then returns `TRUE` (omit = unbounded). `0` is a non-blocking check that returns
immediately with the current readiness (the old omitted behavior — pass `0` for
it). A positive value waits up to that long. A negative `timeoutMs` is rejected
with `ErrInvalidArgument`. Because the host `poll` takes a C `int`, a value above
`2147483647` is clamped to that, which is roughly 24 days.
[[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]]

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
[[src/codegen/builtins/io/native/stdin.rs:lower_io_poll_input_helper]]

On a terminal in the default canonical mode, the line discipline holds typed
characters until Return, so readiness is reported per line rather than per key.
Enter `term::on`'s single-key mode, or use `io::readChar`/`io::readByte`, when a
poll should see individual keypresses.

## Overloads

**`io::pollInput() AS Boolean`**

Blocks until standard input becomes ready, then returns `TRUE` (omitted
`timeoutMs` = unbounded wait). For the old immediate check, pass `0`.
[[src/codegen/builtins/io/mod.rs:register]]

**`io::pollInput(timeoutMs AS Integer) AS Boolean`**

`0` returns immediately with the current readiness; a positive value waits up to
that many milliseconds; a negative value is rejected with `ErrInvalidArgument`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `timeoutMs` | `Integer` | Optional. Omit to block until standard input is ready; `0` is an immediate non-blocking check; a positive value waits up to that many milliseconds, clamped to `2147483647`. Must not be negative. [[src/codegen/builtins/io/mod.rs:register]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when standard input is ready to read — including when it has reached end of input — before the timeout elapses; `FALSE` when the wait elapses with nothing available. [[src/codegen/builtins/io/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative. [[src/builtins/errorcode.rs:ErrInvalidArgument]] |
| `77020005` | `ErrInput` | The poll of standard input fails for a reason other than an interrupting signal, which is retried instead. [[src/builtins/errorcode.rs:ErrInputFailed]] |

## Examples

Read a line only when one is already pending (pass `0` for the immediate check —
omitting the timeout would instead block until input is ready):

```
IMPORT io

SUB main()
  IF io::pollInput(0) THEN
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

Block until input arrives, then take one byte (omit the timeout for an unbounded
wait):

```
IMPORT io

SUB main()
  IF io::pollInput() THEN
    io::print(toString(io::readByte()))
  END IF
END SUB
```

## See also

- `mfb man io readChar`
- `mfb man io readByte`
- `mfb man io readLine`
- `mfb man io input`
- `mfb spec language builtin-functions` — the timeout convention
