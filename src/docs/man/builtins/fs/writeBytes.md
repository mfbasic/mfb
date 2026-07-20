# writeBytes

Write a `List OF Byte` to a file, replacing its contents

## Synopsis

```
fs::writeBytes(path AS String, bytes AS List OF Byte) AS Nothing
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

`fs::writeBytes` opens the file named by `path` for writing, truncating it to
empty if it already exists or creating it if it does not, writes the complete
contents of `bytes`, flushes the file to disk, closes it, and returns nothing.
Any previous contents of an existing file are discarded; to add to a file
instead of replacing it, use `fs::appendBytes`.
[[src/target/shared/code/fs_helpers_atomic.rs:lower_fs_write_bytes_path_helper]]

The byte payload is written directly from the byte list's packed data region.
The write is retried until every byte has been written or the host reports an
output failure, so a short host write that transfers only part of the buffer is
resumed rather than treated as complete, and an interrupted (`EINTR`) write is
retried from the same cursor before any byte has moved. An empty byte list
produces an empty (truncated) file. Bytes are written exactly as held in the
list, with no encoding, decoding, or newline translation, so the function is
suitable for binary data as well as text.
[[src/target/shared/code/fs_helpers_atomic.rs:lower_fs_write_bytes_path_helper]]

The new file is created with mode `384` (octal `0600`), owner read/write only,
before the process umask is applied — not the world-readable `0666`. The file is
created and truncated only after `path` has been validated, and the final path
component is followed when it is a symlink, so writing through a symlink writes
the target file.
[[src/target/shared/code/fs_helpers_atomic.rs:lower_fs_write_bytes_path_helper]]

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory and may contain Unicode
characters when the host filesystem accepts those names. The string must not be
empty and must not contain an embedded NUL byte, because the host `open` call
requires a NUL-terminated path.
[[src/target/shared/code/fs_helpers_atomic.rs:lower_fs_write_bytes_path_helper]]

The file is closed before the function returns on both the success and the
write-failure paths. The write is not atomic: a reader observing the file while
the write is in progress may see a partially written file, and a failure partway
through leaves the file truncated and partially written. For an all-or-nothing
replacement, use `fs::writeBytesAtomic`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the file to write, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. [[src/builtins/fs.rs:call_param_names]] |
| `bytes` | `List OF Byte` | The bytes to write, in order, taken verbatim from the list's data region. An empty list truncates the file to zero length. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing on success, after every byte has been written, flushed, and the file has been closed. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot be turned into a valid NUL-terminated host path. [[src/target/shared/code/fs_helpers_atomic.rs:lower_fs_write_bytes_path_helper]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of `path` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050004` | `ErrNotFound` | The file cannot be created because a component of `path` does not exist, such as a missing parent directory (host `ENOENT`). [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies permission to create or open the file (host `EACCES`). [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | The file cannot be opened because the target already exists in a form that conflicts with creating it (host `EEXIST`). [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The file cannot be opened for any other host reason, including `path` referring to a directory or another non-writable target, and when writing, flushing, or closing the file fails partway through. [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |

## Examples

Write raw bytes to a file:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = [72, 105]
  fs::writeBytes("target/output.bin", bytes)
END SUB
```

Replace a file's contents with bytes read from another file:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = fs::readBytes("source.bin")
  fs::writeBytes("copy.bin", bytes)
END SUB
```

## See also

- `mfb man fs writeBytesAtomic`
- `mfb man fs appendBytes`
- `mfb man fs writeText`
- `mfb man fs readBytes`
