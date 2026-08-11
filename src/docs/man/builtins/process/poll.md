# poll

Test whether a child's output stream is readable within a timeout.

## Synopsis

```
process::poll(p AS Process, ms AS Integer) AS Boolean
process::poll(p AS Process, ms AS Integer, from AS Stream) AS Boolean
```

## Package

`process`

## Imports

```
IMPORT process
```

`process` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/process/mod.rs:is_process_call]]

## Description

`process::poll` reports whether a following read of a child's output stream can
proceed without blocking. It returns `TRUE` when the selected stream is readable —
**including** the case where the child has closed it and the stream is at end of
output, so a draining `process::receive`/`process::receiveBytes` can follow — and
`FALSE` when nothing became readable before the deadline. The stream is inspected
only; no data is consumed, so a `TRUE` result leaves the bytes in place for the next
read. [[src/target/shared/code/process/unix.rs:lower_process_poll_helper]]

`ms` is the wait bound in milliseconds. `0` is a non-blocking check that returns the
stream's current readiness immediately; a positive value waits up to that long; a
timeout that elapses with nothing readable returns `FALSE` (poll reports readiness
as a boolean and never raises `ErrTimeout`).
[[src/target/shared/code/process/unix.rs:lower_process_poll_helper]]

Without a `from` argument `poll` inspects the child's standard output; pass a
`Stream` value to choose standard output or standard error.
[[src/codegen/builtins/process/mod.rs:STREAM_TYPE]]

## Overloads

**`process::poll(p AS Process, ms AS Integer) AS Boolean`**

Waits up to `ms` milliseconds for the child's standard output to become readable.

**`process::poll(p AS Process, ms AS Integer, from AS Stream) AS Boolean`**

The same, for the selected stream — `Stream.StdOut` or `Stream.StdErr`.
[[src/target/shared/code/process/unix.rs:lower_process_poll_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. Borrowed and inspected for readiness only; no data is read. Also accepts the alternate named-argument spelling `process`. [[src/codegen/builtins/process/mod.rs:P_POLL]] |
| `ms` | `Integer` | The maximum time to wait, in milliseconds. `0` is an immediate non-blocking check; a positive value waits up to that long. [[src/target/shared/code/process/unix.rs:lower_process_poll_helper]] |
| `from` | `Stream` | Optional. Which output stream to inspect: `Stream.StdOut` (the default) or `Stream.StdErr`. [[src/codegen/builtins/process/mod.rs:P_POLL_S]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when the selected stream is readable — a following read will not block, including when that read would report end of output; `FALSE` when nothing became readable before the deadline. [[src/target/shared/code/process/unix.rs:lower_process_poll_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `p` has already been dropped or detached. [[src/builtins/errorcode.rs:ErrResourceClosed]] |

## Examples

Read a line only if one is ready within 100 ms:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  IF process::poll(child, 100) THEN
    io::print(process::receive(child))
  END IF
  RETURN 0
END FUNC
```

Check the child's standard error without blocking:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("echo oops 1>&2")
  IF process::poll(sh, 0, Stream.StdErr) THEN
    io::print(process::receive(sh, Stream.StdErr))
  END IF
  RETURN 0
END FUNC
```

## See also

- `mfb man process receive`
- `mfb man process receiveBytes`
- `mfb man process send`
- `mfb man process types`
