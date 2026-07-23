# appendText

Append a `String` to the end of a file as UTF-8 text, preserving its existing contents

## Synopsis

```
fs::appendText(path AS String, value AS String) AS Nothing
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

`fs::appendText` opens the file named by `path` in append mode, creating it with
no contents if it does not already exist, writes the complete contents of `value`
as UTF-8 text after whatever the file already held, flushes the file to disk,
closes it, and returns nothing. Any existing contents are preserved and the new
text is added after them; to replace a file's contents instead of extending them,
use `fs::writeText`.
[[src/target/shared/code/fs/atomic.rs:lower_fs_write_text_path_helper]]

The file is opened with the append flag set, so every write is positioned at the
current end of the file. The text payload is written directly from the `String`'s
packed byte data. A `String` already holds well-formed UTF-8, so the bytes are
written exactly as held, with no re-encoding, decoding, or newline translation,
and no trailing newline is added. The write is retried until every byte has been
written or the host reports an output failure, so a short host write that
transfers only part of the buffer is resumed rather than treated as complete, and
an interrupted (`EINTR`) write is retried from the same cursor before any byte has
moved. An empty `String` leaves the file's length unchanged, creating it as an
empty file if it did not exist.
[[src/target/shared/code/fs/atomic.rs:lower_fs_write_text_path_helper]]

When the file is created it is given mode `384` (octal `0600`), owner read/write
only, before the process umask is applied — not the world-readable `0666`. An
existing file keeps its current mode. The file is created and opened only after
`path` has been validated, and the final path component is followed when it is a
symlink, so appending through a symlink appends to the target file.
[[src/target/shared/code/fs/atomic.rs:lower_fs_write_text_path_helper]]

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory and may contain Unicode
characters when the host filesystem accepts those names. The string must not be
empty and must not contain an embedded NUL byte, because the host `open` call
requires a NUL-terminated path.
[[src/target/shared/code/fs/atomic.rs:lower_fs_write_text_path_helper]]

The file is closed before the function returns on both the success and the
write-failure paths. The append is not atomic: a reader observing the file while
the write is in progress may see only part of the appended text, and a failure
partway through leaves the file extended by only the bytes written so far.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the file to append to, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. The file is created if it does not exist. [[src/builtins/fs.rs:call_param_names]] |
| `value` | `String` | The text to append, taken verbatim as the `String`'s UTF-8 bytes, in order, after the file's existing contents. An empty `String` leaves the file's length unchanged. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing on success, after every byte has been written, flushed, and the file has been closed. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot be turned into a valid NUL-terminated host path. [[src/target/shared/code/fs/atomic.rs:lower_fs_write_text_path_helper]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of `path` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050004` | `ErrNotFound` | The file cannot be opened because a component of `path` does not exist, such as a missing parent directory (host `ENOENT`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies permission to create or open the file (host `EACCES`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | The host reports the target already exists in a form that conflicts with the open (host `EEXIST`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The file cannot be opened for any other host reason, including `path` referring to a directory or another non-writable target, and when writing, flushing, or closing the file fails partway through. [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |

## Examples

Append a line to a log file:

```
IMPORT fs

SUB main()
  fs::appendText("target/output.txt", "line\n")
END SUB
```

Build up a file across several calls:

```
IMPORT fs
IMPORT io

SUB main()
  fs::appendText("notes.txt", "first\n")
  fs::appendText("notes.txt", "second\n")
  LET text AS String = fs::readText("notes.txt")
  io::print(text)
END SUB
```

## See also

- `mfb man fs appendBytes`
- `mfb man fs writeText`
- `mfb man fs writeTextAtomic`
- `mfb man fs readText`
