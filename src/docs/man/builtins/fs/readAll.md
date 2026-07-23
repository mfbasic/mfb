# readAll

Read all remaining text from an open `File` into a `String`

## Synopsis

```
fs::readAll(file AS File) AS String
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

`fs::readAll` reads every remaining byte of `file` as UTF-8 text, starting at the
file's current read position and continuing to end of input, validates that the
bytes are well-formed UTF-8, and returns them as a single `String`. The read
position is advanced to end of input, so a subsequent `fs::eof` reports true.
`file` must be an open `File` resource — such as one returned by `fs::openFile` or
`fs::open` — opened in a mode that permits reading.
[[src/target/shared/code/fs/io.rs:lower_fs_read_all_helper]]

The amount to read is measured up front: the function seeks to record the current
position, seeks to the end to find the file's length, seeks back to the start
position, allocates a `String` of exactly that length, and reads the remainder
into it in one or more host reads until the buffer is full. No newline
translation or other decoding is performed beyond the UTF-8 validity check, so the
returned `String` holds the file's remaining bytes exactly as stored on disk,
interpreted as UTF-8. When `file` is already at end of input, no bytes remain and
the empty `String` is returned.
[[src/target/shared/code/fs/io.rs:lower_fs_read_all_helper]]

If the file was previously read with `fs::readLine`, the buffered read-ahead is
first reconciled so the measurement and read see the true file-descriptor
position rather than the block read-ahead. The function only reads from and
repositions `file`; it does not close it and has no other side effects. To read
the same data without the UTF-8 requirement, use `fs::readAllBytes`. To read a
whole file by path in a single call rather than from an open handle, use
`fs::readText`. [[src/target/shared/code/fs/io.rs:emit_reconcile_read_buffer]]

Thread cancellation is cooperative: the runtime does not asynchronously interrupt
a blocking host file read, so a worker that needs prompt cancellation around a
blocking descriptor should check `thread::isCancelled` between operations.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `file` | `File` | An open `File` resource to read from, positioned at the start of the data to read. Must not have been closed and must have been opened in a mode that permits reading. [[src/builtins/fs.rs:READ_ALL]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The remaining contents of `file`, from the current position to end of input, in file order, as a UTF-8 `String`. When `file` is already at end of input, an empty `String` is returned. [[src/target/shared/code/fs/io.rs:lower_fs_read_all_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `file` has already been closed. [[src/target/shared/code/fs/io.rs:lower_fs_read_all_helper]] |
| `77020001` | `ErrRead` | Repositioning `file` to measure its remaining length fails, the measured end is before the start position, or the host read fails partway through before all measured bytes have been read. [[src/target/shared/code/error_constants.rs:ERR_READ_CODE]] |
| `77010001` | `ErrOutOfMemory` | The `String` that holds the file's remaining contents cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77020004` | `ErrEncoding` | The bytes read from `file` are not valid UTF-8. [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |

## Examples

Read all remaining text from an open file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("data.txt")
  LET value AS String = fs::readAll(f)
  ' f is closed by lexical drop when this scope ends
END SUB
```

Skip the first line, then read the rest of the file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("data.txt")
  LET header AS String = fs::readLine(f)
  LET body AS String = fs::readAll(f)
END SUB
```

## See also

- `mfb man fs readAllBytes`
- `mfb man fs readLine`
- `mfb man fs readText`
- `mfb man fs openFile`
- `mfb man fs eof`
