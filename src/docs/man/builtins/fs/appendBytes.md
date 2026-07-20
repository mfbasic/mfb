# appendBytes

Append a `List OF Byte` to the end of a file, preserving its existing contents

## Synopsis

```
fs::appendBytes(path AS String, bytes AS List OF Byte) AS Nothing
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

`fs::appendBytes` opens the file named by `path` in append mode, creating it with
no contents if it does not already exist, writes the complete contents of `bytes`
after whatever the file already held, flushes the file to disk, closes it, and
returns nothing. Any existing contents are preserved and the new bytes are added
after them; to replace a file's contents instead of extending them, use
`fs::writeBytes`.
[[src/target/shared/code/fs_helpers_atomic.rs:lower_fs_write_bytes_path_helper]]

The file is opened with the append flag set, so every write is positioned at the
current end of the file. The byte payload is written directly from the byte
list's packed data region. The write is retried until every byte has been written
or the host reports an output failure, so a short host write that transfers only
part of the buffer is resumed rather than treated as complete, and an interrupted
(`EINTR`) write is retried from the same cursor before any byte has moved. An
empty byte list leaves the file's length unchanged, creating it as an empty file
if it did not exist. Bytes are written exactly as held in the list, with no
encoding, decoding, or newline translation, so the function is suitable for
binary data as well as text.
[[src/target/shared/code/fs_helpers_atomic.rs:lower_fs_write_bytes_path_helper]]

When the file is created it is given mode `384` (octal `0600`), owner read/write
only, before the process umask is applied — not the world-readable `0666`. An
existing file keeps its current mode. The file is created and opened only after
`path` has been validated, and the final path component is followed when it is a
symlink, so appending through a symlink appends to the target file.
[[src/target/shared/code/fs_helpers_atomic.rs:lower_fs_write_bytes_path_helper]]

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory and may contain Unicode
characters when the host filesystem accepts those names. The string must not be
empty and must not contain an embedded NUL byte, because the host `open` call
requires a NUL-terminated path.
[[src/target/shared/code/fs_helpers_atomic.rs:lower_fs_write_bytes_path_helper]]

The file is closed before the function returns on both the success and the
write-failure paths. The append is not atomic: a reader observing the file while
the write is in progress may see only part of the appended bytes, and a failure
partway through leaves the file extended by only the bytes written so far.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the file to append to, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. The file is created if it does not exist. [[src/builtins/fs.rs:call_param_names]] |
| `bytes` | `List OF Byte` | The bytes to append, in order, taken verbatim from the list's data region after the file's existing contents. An empty list leaves the file's length unchanged. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing on success, after every byte has been written, flushed, and the file has been closed. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot be turned into a valid NUL-terminated host path. [[src/target/shared/code/fs_helpers_atomic.rs:lower_fs_write_bytes_path_helper]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of `path` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050004` | `ErrNotFound` | The file cannot be opened because a component of `path` does not exist, such as a missing parent directory (host `ENOENT`). [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies permission to create or open the file (host `EACCES`). [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | The host reports the target already exists in a form that conflicts with the open (host `EEXIST`). [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The file cannot be opened for any other host reason, including `path` referring to a directory or another non-writable target, and when writing, flushing, or closing the file fails partway through. [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |

## Examples

Append a single newline byte to a log file:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = [10]
  fs::appendBytes("target/log.bin", bytes)
END SUB
```

Append the contents of one file to the end of another:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = fs::readBytes("source.bin")
  fs::appendBytes("combined.bin", bytes)
END SUB
```

## See also

- `mfb man fs appendText`
- `mfb man fs writeBytes`
- `mfb man fs writeBytesAtomic`
- `mfb man fs readBytes`
