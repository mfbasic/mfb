# receiveBytes

Read one available chunk of raw bytes from a child's output.

## Synopsis

```
process::receiveBytes(p AS Process) AS List OF Byte
process::receiveBytes(p AS Process, from AS Stream) AS List OF Byte
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

`process::receiveBytes` reads the next available chunk of raw bytes from a child's
output stream and returns it as a `List OF Byte`. It performs one underlying read,
so it returns as soon as any data is available rather than waiting to fill a fixed
size, and the returned list is frequently shorter than the amount the child will
eventually produce. It does no line framing, decoding, or newline translation, so
it is the right call for binary output; use `process::receive` for text lines.
[[src/codegen/builtins/process/func_receive_bytes.rs:lower_process_receivebytes_helper_posix]]

Without a `from` argument it reads the child's standard output; pass a `Stream`
value to choose standard output or standard error. The call blocks until at least
one byte is available or the stream ends. A pipe read returns any buffered bytes
before signalling end of stream, so late output is drained; only a read that finds
end of stream with nothing buffered raises `ErrResourceClosed`. On success the
result always holds at least one byte — end of output is never an empty list.
[[src/builtins/errorcode.rs:ErrResourceClosed]]
[[src/codegen/builtins/process/mod.rs:STREAM_TYPE]]

## Overloads

**`process::receiveBytes(p AS Process) AS List OF Byte`**

Reads the next chunk from the child's standard output.

**`process::receiveBytes(p AS Process, from AS Stream) AS List OF Byte`**

Reads the next chunk from the selected stream — `Stream.StdOut` or `Stream.StdErr`.
[[src/codegen/builtins/process/func_receive_bytes.rs:lower_process_receivebytes_helper_posix]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`. [[src/codegen/builtins/process/func_receive_bytes.rs:register]] |
| `from` | `Stream` | Optional. Which output stream to read: `Stream.StdOut` (the default) or `Stream.StdErr`. [[src/codegen/builtins/process/func_receive_bytes.rs:register]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The next chunk of the child's output, in arrival order, always at least one byte. End of output is reported as `ErrResourceClosed`, never as an empty list. [[src/codegen/builtins/process/func_receive_bytes.rs:lower_process_receivebytes_helper_posix]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | End of the selected stream was reached with nothing buffered, or `p` has already been dropped or detached. [[src/builtins/errorcode.rs:ErrResourceClosed]] |
| `77010001` | `ErrOutOfMemory` | The read buffer or the returned `List OF Byte` could not be allocated. [[src/builtins/errorcode.rs:ErrOutOfMemory]] |

## Examples

Read one chunk of raw output and report its length:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  LET chunk = process::receiveBytes(child)
  io::print(toString(len(chunk)))
  RETURN 0
END FUNC
```

## See also

- `mfb man process receive`
- `mfb man process poll`
- `mfb man process sendBytes`
- `mfb man process close`
- `mfb man process types`
