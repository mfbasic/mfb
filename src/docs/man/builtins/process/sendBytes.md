# sendBytes

Write raw bytes to a child's standard input, with no newline added.

## Synopsis

```
process::sendBytes(p AS Process, data AS List OF Byte) AS Nothing
process::sendBytes(p AS Process, data AS List OF Byte, timeoutMs AS Integer) AS Nothing
```

## Package

`process`

## Imports

```
IMPORT process
```

`process` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/process/mod.rs:register]]

## Description

`process::sendBytes` writes the raw bytes of `data` to the child's standard input,
in list order, with **no** trailing newline and no re-encoding. It is the binary
counterpart of `process::send` (which sends a `String` and appends `'\n'`); use
`sendBytes` for binary input or when you control line framing yourself.
[[src/codegen/builtins/process/native/unix.rs:lower_process_send_helper]]

The whole list is written before the call returns: it loops over the underlying
writes, resuming a short or interrupted write rather than treating it as complete.
An empty list writes nothing and returns immediately. Without a `timeoutMs` the
call blocks while the child's input pipe is full, waiting for room.
[[src/codegen/builtins/process/native/unix.rs:lower_process_send_helper]]

If the child has closed or is no longer reading its standard input — a broken pipe —
the write fails and `sendBytes` raises `ErrResourceClosed`, the same error raised
when the input was already closed with `process::close` or the handle was dropped or
detached. [[src/builtins/errorcode.rs:ErrResourceClosed]]

`timeoutMs` bounds how long the call may wait for pipe space, in milliseconds;
on expiry it raises `ErrTimeout`. On Windows the timeout is best-effort: anonymous
pipes have no write-readiness poll, so a write to a draining reader returns at once
but a write that fills the pipe is not preempted at the deadline.
[[src/codegen/builtins/process/native/windows.rs:lower_process_send_helper]]

## Overloads

**`process::sendBytes(p AS Process, data AS List OF Byte) AS Nothing`**

Writes the raw bytes, blocking until the whole list has been handed to the child's
input pipe.

**`process::sendBytes(p AS Process, data AS List OF Byte, timeoutMs AS Integer) AS Nothing`**

The same, but raises `ErrTimeout` if the write cannot complete within `timeoutMs`
milliseconds (best-effort on Windows). [[src/codegen/builtins/process/native/unix.rs:emit_poll_wait]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`. [[src/codegen/builtins/process/func_send_bytes.rs:register]] |
| `data` | `List OF Byte` | The bytes to write, in list order, with no newline appended. An empty list writes nothing. [[src/codegen/builtins/process/native/unix.rs:lower_process_send_helper]] |
| `timeoutMs` | `Integer` | Optional. The maximum time to wait for room in the child's input pipe, in milliseconds; on expiry the call raises `ErrTimeout`. Best-effort on Windows. [[src/codegen/builtins/process/func_send_bytes.rs:register]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `sendBytes` returns no value. On a successful return every byte of `data` has been handed to the child's input pipe. [[src/codegen/builtins/process/mod.rs:PROCESS_TYPE]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | The child's input pipe is gone — a broken pipe, an input already closed with `process::close`, or a handle that was dropped or detached. [[src/builtins/errorcode.rs:ErrResourceClosed]] |
| `77050008` | `ErrTimeout` | (timeout overload) `timeoutMs` elapsed before the bytes could be fully written. [[src/builtins/errorcode.rs:ErrTimeout]] |

## Examples

Write raw bytes to a filter and read the result:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["cat"])
  LET data AS List OF Byte = [104, 105, 10]
  process::sendBytes(child, data)
  process::close(child)
  io::print(process::receive(child))
  RETURN 0
END FUNC
```

## See also

- `mfb man process send`
- `mfb man process receiveBytes`
- `mfb man process close`
- `mfb man process poll`
- `mfb man process types`
