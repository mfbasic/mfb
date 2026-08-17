# receive

Read one newline-terminated line of text from a child's output.

## Synopsis

```
process::receive(p AS Process) AS String
process::receive(p AS Process, from AS Stream) AS String
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

`process::receive` reads one line from a child's output stream and returns it as a
`String`, **including** the trailing newline. It reads until it sees a `'\n'`,
never over-reading past the line boundary, so successive calls return successive
lines. Without a `from` argument it reads the child's standard output; pass a
`Stream` value to choose standard output or standard error explicitly.
[[src/codegen/builtins/process/func_receive.rs:lower_process_receive_helper_posix]]
[[src/codegen/builtins/process/mod.rs:STREAM_TYPE]]

The call blocks until a full line is available or the stream ends. At end of stream
it **drains before reporting closed**: any bytes accumulated since the last newline
are returned as a final (newline-less) line, and only a subsequent read that finds
end of stream with nothing buffered raises `ErrResourceClosed`. A consumer therefore
loops, reading lines until `ErrResourceClosed` marks the end of the output.
[[src/codegen/builtins/errorcode/mod.rs:ErrResourceClosed]]

The returned line is validated as UTF-8; output that is not valid UTF-8 raises
`ErrEncoding`. Use `process::receiveBytes` for binary output or output whose
encoding is unknown. Very long lines are capped at 1 MiB: a line reaching that
length is returned as-is without waiting for a newline.
[[src/codegen/builtins/errorcode/mod.rs:ErrEncoding]]

## Overloads

**`process::receive(p AS Process) AS String`**

Reads one line from the child's standard output.

**`process::receive(p AS Process, from AS Stream) AS String`**

Reads one line from the selected stream — `Stream.StdOut` or `Stream.StdErr`.
[[src/codegen/builtins/process/func_receive.rs:lower_process_receive_helper_posix]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`. [[src/codegen/builtins/process/func_receive.rs:register]] |
| `from` | `Stream` | Optional. Which output stream to read: `Stream.StdOut` (the default) or `Stream.StdErr`. [[src/codegen/builtins/process/func_receive.rs:register]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | One line of the child's output, including its trailing newline (or, at end of stream, the final unterminated line). End of output is reported as `ErrResourceClosed`, never as an empty string. [[src/codegen/builtins/process/func_receive.rs:lower_process_receive_helper_posix]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | End of the selected stream was reached with nothing left to return, or `p` has already been dropped or detached. [[src/codegen/builtins/errorcode/mod.rs:ErrResourceClosed]] |
| `77020004` | `ErrEncoding` | The bytes read are not valid UTF-8. [[src/codegen/builtins/errorcode/mod.rs:ErrEncoding]] |
| `77010001` | `ErrOutOfMemory` | The line accumulator or the returned `String` could not be allocated. [[src/codegen/builtins/errorcode/mod.rs:ErrOutOfMemory]] |

## Examples

Read one line of a child's standard output:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  io::print(process::receive(child))
  RETURN 0
END FUNC
```

Read a diagnostic line from the child's standard error:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("echo oops 1>&2")
  io::print(process::receive(sh, Stream.StdErr))
  RETURN 0
END FUNC
```

## See also

- `mfb man process receiveBytes`
- `mfb man process poll`
- `mfb man process send`
- `mfb man process close`
- `mfb man process types`
