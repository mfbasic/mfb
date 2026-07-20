# writeAll

Write all of a `String` to an open `File` as UTF-8 text

## Synopsis

```
fs::writeAll(file AS File, value AS String) AS Nothing
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

`fs::writeAll` writes the complete contents of `value` to `file` as UTF-8 text,
starting at the file's current write position, and returns nothing. The bytes are
taken directly from the `String`'s packed byte data; because a `String` already
holds well-formed UTF-8, no re-encoding, decoding, or newline translation is
performed. An empty `String` writes no bytes and leaves the file unchanged.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_write_all_helper]]

The write is retried until every byte has been written or the host reports an
output failure, so a short host write that transfers only part of the buffer is
resumed from the same cursor rather than treated as complete. The file position
advances by the number of bytes written, so consecutive calls write one after
another within the open handle, and a following `fs::writeAll` or
`fs::writeAllBytes` continues from where this call left off.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_write_all_helper]]

`file` must be an open `File` resource — such as one returned by `fs::openFile`
or `fs::open` — opened in a mode that permits writing (`"write"`, `"readWrite"`,
or `"append"`). If the handle was previously read with `fs::readLine`, its
buffered read-ahead is first reconciled so the write lands at the true
file-descriptor position rather than the block read-ahead. When per-`File` write
buffering is enabled, the bytes are appended into the handle's buffer instead of
being written straight through; otherwise they go directly to the descriptor. The
function only writes to and repositions `file`; it does not close it and has no
other side effects. Whether the data is forced to disk is governed by the open
handle, not by this call, which does not flush on its own. To write a whole file
by path in a single call rather than through an open handle, use `fs::writeText`.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_write_all_helper]]

Thread cancellation is cooperative: the runtime does not asynchronously interrupt
a blocking host file write, so a worker that needs prompt cancellation around a
blocking descriptor should check `thread::isCancelled` between operations.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `file` | `File` | An open `File` resource to write to, positioned at the point where the text should be written. Must not have been closed and must have been opened in a mode that permits writing (`"write"`, `"readWrite"`, or `"append"`). [[src/builtins/fs.rs:WRITE_ALL]] |
| `value` | `String` | The text to write, taken verbatim as the `String`'s UTF-8 bytes, in order. An empty `String` writes nothing. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing on success, after every byte of `value` has been written to `file`. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `file` has already been closed. [[src/target/shared/code/fs_helpers_io.rs:lower_fs_write_all_helper]] |
| `77020002` | `ErrOutput` | `file` was not opened for writing, or the host write fails partway through before all of `value` has been written. [[src/target/shared/code/error_constants.rs:ERR_OUTPUT_CODE]] |

## Examples

Write text to an open file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("target/output.txt", "write")
  fs::writeAll(f, "Hello")
  ' f is closed by lexical drop when this scope ends
END SUB
```

Write a header line, then the rest of the body:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("target/report.txt", "write")
  fs::writeAll(f, "title\n")
  fs::writeAll(f, "body")
END SUB
```

## See also

- `mfb man fs writeAllBytes`
- `mfb man fs writeText`
- `mfb man fs openFile`
- `mfb man fs readAll`
- `mfb man fs close`
