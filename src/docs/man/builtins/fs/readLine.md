# readLine

Read one line of UTF-8 text from an open `File`

## Synopsis

```
fs::readLine(file AS File) AS String
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

`fs::readLine` reads a single line from `file` starting at its current read
position, advances the position to just past the line's terminator, and returns
the line as a `String` with the terminator removed. `file` must be an open `File`
resource — such as one returned by `fs::openFile` or `fs::open` — opened in a mode
that permits reading. [[src/target/shared/code/fs_helpers_io.rs:lower_fs_read_line_helper]]

A line ends at the first line feed (LF, byte `0x0A`) at or after the current
position. Both LF and CRLF terminators are accepted: when the byte immediately
before the LF is a carriage return (CR, byte `0x0D`) it is treated as part of the
terminator and is also stripped from the returned `String`. A bare CR with no
following LF is not a terminator and is returned as an ordinary character. When
the remaining bytes contain no LF, the entire remainder of the file is returned
as the final line and the position is advanced to end of input; the next call
then fails with end-of-input. [[src/target/shared/code/fs_helpers_io.rs:lower_fs_read_line_helper]]

The returned `String` never includes the terminating LF or the CR of a CRLF pair.
An empty line (an LF, or a CRLF, with nothing before it) yields an empty `String`
while still consuming the terminator and advancing the position. The bytes making
up the line are validated as UTF-8 before being returned.

On success the position is left immediately after the consumed terminator (or at
end of input when the last line had no terminator), so repeated calls walk the
file one line at a time. Because end of input is reported as an error rather than
an empty result, use `fs::eof` to test for the end before each call. The function
only reads from and repositions `file`; it does not close it and has no other side
effects.

Reads are served from a transparent per-handle block buffer: internally the file
is read in blocks and lines are handed out from that buffer, so a loop over a
large file runs in linear time rather than re-reading the remainder for every
line. This is invisible — the lines, terminators, EOF point, and errors are
identical to an unbuffered read. A whole-file read (`fs::readAll`,
`fs::readAllBytes`) or a write (`fs::writeAll`) on the same handle transparently
reconciles the buffer first, so mixing them with `fs::readLine` sees the exact
logical position. [[src/target/shared/code/fs_helpers_io.rs:lower_fs_read_line_helper]]

Thread cancellation is cooperative. The current runtime does not asynchronously
interrupt arbitrary host file reads; workers that need prompt cancellation around
blocking file descriptors should check `thread::isCancelled` between
cancellation-point operations.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `file` | `File` | An open `File` resource to read from, positioned at the start of the line to read. Must not have been closed and must have been opened in a mode that permits reading. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The next line of text, in file order, with the trailing LF or CRLF removed. An empty line returns an empty `String`; a final line with no terminator returns the remaining bytes of the file. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `file` has already been closed. [[src/target/shared/code/fs_helpers_io.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77020003` | `ErrEof` | `file` is already at end of input, so no bytes remain to form a line. [[src/target/shared/code/fs_helpers_io.rs:ERR_EOF_CODE]] |
| `77020001` | `ErrRead` | Repositioning the file or reading its bytes fails, including when `file` was not opened for reading and when the host read fails partway through. [[src/target/shared/code/fs_helpers_io.rs:ERR_READ_CODE]] |
| `77020004` | `ErrEncoding` | The bytes of the line are not valid UTF-8. [[src/target/shared/code/fs_helpers_io.rs:ERR_ENCODING_CODE]] |
| `77010001` | `ErrOutOfMemory` | The read block, the line accumulator, or the `String` holding the returned line cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Read the first line of a file:

```
IMPORT fs

RES f = fs::openFile("data.txt")
LET line AS String = fs::readLine(f)
' f is closed by lexical drop when this scope ends
```

Read every line until end of input:

```
IMPORT fs
IMPORT io

RES f = fs::openFile("data.txt")
WHILE NOT fs::eof(f)
  io::print(fs::readLine(f))
END WHILE
```

## See also

- `mfb man fs readAll`
- `mfb man fs eof`
- `mfb man fs openFile`
- `mfb man fs readText`
- `mfb man fs close`
