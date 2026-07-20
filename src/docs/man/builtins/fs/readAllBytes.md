# readAllBytes

Read all remaining bytes from an open `File` into a `List OF Byte`

## Synopsis

```
fs::readAllBytes(file AS File) AS List OF Byte
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

`fs::readAllBytes` reads every remaining byte of `file`, starting at the file's
current read position and continuing to end of input, and returns them as a single
`List OF Byte`. The read position is advanced to end of input, so a subsequent
`fs::eof` reports true. `file` must be an open `File` resource — such as one
returned by `fs::openFile` or `fs::open` — opened in a mode that permits reading.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_read_all_bytes_helper]]

The amount to read is measured up front: the function seeks to record the current
position, seeks to the end to find the file's length, seeks back to the start
position, allocates a `List OF Byte` of exactly that length, and reads the
remainder into it in one or more host reads until the collection is full. No
newline translation, decoding, or UTF-8 validation is performed, so the returned
list holds the file's remaining bytes exactly as stored on disk, making it suitable
for binary data as well as text. When `file` is already at end of input, no bytes
remain and the empty `List OF Byte` is returned.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_read_all_bytes_helper]]

If the file was previously read with `fs::readLine`, the buffered read-ahead is
first reconciled so the measurement and read see the true file-descriptor position
rather than the block read-ahead. The function only reads from and repositions
`file`; it does not close it and has no other side effects. To read the same data
as validated UTF-8 text, use `fs::readAll`. To read a whole file by path in a
single call rather than from an open handle, use `fs::readBytes`.
[[src/target/shared/code/fs_helpers_io.rs:emit_reconcile_read_buffer]]

Thread cancellation is cooperative: the runtime does not asynchronously interrupt a
blocking host file read, so a worker that needs prompt cancellation around a
blocking descriptor should check `thread::isCancelled` between operations.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `file` | `File` | An open `File` resource to read from, positioned at the start of the data to read. Must not have been closed and must have been opened in a mode that permits reading. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The remaining contents of `file`, from the current position to end of input, in file order, as a `List OF Byte`. When `file` is already at end of input, an empty `List OF Byte` is returned. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `file` has already been closed. [[src/target/shared/code/fs_helpers_io.rs:lower_fs_read_all_bytes_helper]] |
| `77020001` | `ErrRead` | Reconciling or repositioning `file` to measure its remaining length fails, the measured end is before the start position, `file` was not opened for reading, or the host read fails partway through before all measured bytes have been read. [[src/target/shared/code/error_constants.rs:ERR_READ_CODE]] |
| `77010001` | `ErrOutOfMemory` | The `List OF Byte` that holds the file's remaining contents cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Read all remaining bytes from an open file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("data.bin")
  LET bytes AS List OF Byte = fs::readAllBytes(f)
  ' f is closed by lexical drop when this scope ends
END SUB
```

Skip the first line, then read the remaining bytes of the file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("data.bin")
  LET header AS String = fs::readLine(f)
  LET body AS List OF Byte = fs::readAllBytes(f)
END SUB
```

## See also

- `mfb man fs readAll`
- `mfb man fs readBytes`
- `mfb man fs readLine`
- `mfb man fs openFile`
- `mfb man fs eof`
