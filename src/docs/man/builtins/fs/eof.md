# eof

Test whether an open `File` is at end of input

## Synopsis

```
fs::eof(file AS File) AS Boolean
```

## Package

fs

## Imports

```
IMPORT fs
```

`fs` is a built-in package, so no manifest dependency is required.
[[src/builtins/fs.rs:is_fs_call]]

## Description

`fs::eof` reports whether `file`'s current read position has reached the end of
its contents. It returns `TRUE` when the position is at or beyond the last byte
and `FALSE` while one or more bytes remain to be read. `file` must be an open
`File` resource, such as one returned by `fs::openFile` or `fs::open`.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_eof_helper]]

The test is buffer-aware (plan-14-C): if the transparent per-handle read buffer
still holds unconsumed bytes (its read cursor is before its fill mark), `fs::eof`
returns `FALSE` immediately without querying the host. Otherwise it asks the host
for the file's current position and total length and compares them — the position
is captured, the handle is seeked to end to read the length, then seeked back to
the captured position, so the read position is left exactly where it was. The
function reads no contents and has no side effects: it does not advance the
position, write anything, or close `file`.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_eof_helper]]

Because determining the length requires seeking, `fs::eof` only works on a
seekable handle — a regular file on disk. On a pipe, a socket, or another
non-seekable handle the host cannot report a position or length, and the call
raises an error instead of returning a `Boolean`.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_eof_helper]]

Use `fs::eof` to guard a read loop so that `fs::readLine` and the other reading
functions are only called while input remains. This is the intended pattern
because end of input is reported by those functions as an error rather than as an
empty result: testing `fs::eof` first lets a loop stop cleanly at the end of the
file.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `file` | `File` | An open, seekable `File` resource to test, as returned by `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, or `fs::createTempFile`. Must not have been closed. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `file`'s read position is at or beyond the end of its contents, `FALSE` when one or more bytes remain to be read. An empty file reports `TRUE`. The read position is unchanged either way. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `file` has already been closed, whether by an earlier `fs::close` on the same value or by a prior scope-drop. [[src/target/shared/code/fs_helpers_io.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77020001` | `ErrRead` | The host cannot determine the file's position or length — for example on a pipe, socket, or other non-seekable handle — so a seek fails. [[src/target/shared/code/fs_helpers_io.rs:ERR_READ_CODE]] |

## Examples

Read every line until end of input:

```
IMPORT fs
IMPORT io

SUB main()
  RES f = fs::openFile("data.txt")
  WHILE NOT fs::eof(f)
    io::print(fs::readLine(f))
  END WHILE
  ' f is closed by lexical drop when this scope ends
END SUB
```

## See also

- `mfb man fs readLine`
- `mfb man fs readAll`
- `mfb man fs openFile`
- `mfb man fs open`
- `mfb man fs close`
