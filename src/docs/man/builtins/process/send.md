# send

Write a line of text to a child's standard input, appending a newline.

## Synopsis

```
process::send(p AS Process, text AS String) AS Nothing
process::send(p AS Process, text AS String, timeoutMs AS Integer) AS Nothing
```

## Package

`process`

## Imports

```
IMPORT process
```

`process` is a built-in package, so no manifest dependency is required.
[[src/builtins/process.rs:is_process_call]]

## Description

`process::send` writes the UTF-8 bytes of `text` to the child's standard input and
then appends a single newline (`'\n'`), so each call delivers one complete line to
a line-oriented child. To write raw bytes with no trailing newline, use
`process::sendBytes`. [[src/target/shared/code/process/unix.rs:lower_process_send_helper]]

The whole payload is written before the call returns: it loops over the underlying
writes, advancing past whatever each accepted and retrying an interrupted write, so
a short write is resumed rather than mistaken for completion. Without a `timeoutMs`
the call blocks while the child's input pipe is full, waiting for the child to
consume enough to make room. [[src/target/shared/code/process/unix.rs:lower_process_send_helper]]

If the child has closed or is no longer reading its standard input — a broken pipe —
the write fails and `send` raises `ErrResourceClosed`, the same error raised when
the input was already closed with `process::close` or the handle was dropped or
detached. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]]

`timeoutMs` bounds how long the call may wait for pipe space, in milliseconds;
when the deadline passes with the payload not fully written it raises `ErrTimeout`.
On Windows the timeout is best-effort: anonymous pipes have no write-readiness poll,
so a write to a draining reader returns immediately (the common case) but a write
that fills the pipe is not preempted at the deadline.
[[src/target/shared/code/process/windows.rs:lower_process_send_helper]]

## Overloads

**`process::send(p AS Process, text AS String) AS Nothing`**

Writes `text` plus a newline, blocking until the whole line has been handed to the
child's input pipe.

**`process::send(p AS Process, text AS String, timeoutMs AS Integer) AS Nothing`**

The same, but raises `ErrTimeout` if the write cannot complete within `timeoutMs`
milliseconds (best-effort on Windows). [[src/target/shared/code/process/unix.rs:emit_poll_wait]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`. [[src/builtins/process.rs:P_SEND]] |
| `text` | `String` | The line to write. Its UTF-8 bytes are sent, followed by a single newline. [[src/target/shared/code/process/unix.rs:lower_process_send_helper]] |
| `timeoutMs` | `Integer` | Optional. The maximum time to wait for room in the child's input pipe, in milliseconds; on expiry the call raises `ErrTimeout`. Best-effort on Windows. [[src/builtins/process.rs:P_SEND_T]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `send` returns no value. On a successful return the whole line (text plus newline) has been handed to the child's input pipe. [[src/builtins/process.rs:PROCESS_TYPE]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | The child's input pipe is gone — a broken pipe, an input already closed with `process::close`, or a handle that was dropped or detached. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77050008` | `ErrTimeout` | (timeout overload) `timeoutMs` elapsed before the line could be fully written. [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]] |

## Examples

Send two lines to a filter and read its sorted output:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sorter = process::spawn(["sort"])
  process::send(sorter, "banana")
  process::send(sorter, "apple")
  process::close(sorter)
  io::print(process::receive(sorter))
  RETURN 0
END FUNC
```

Bound the write with a one-second timeout:

```
IMPORT process

FUNC main AS Integer
  RES child = process::spawn(["cat"])
  process::send(child, "hello", 1000)
  RETURN 0
END FUNC
```

## See also

- `mfb man process sendBytes`
- `mfb man process receive`
- `mfb man process close`
- `mfb man process poll`
- `mfb man process types`
