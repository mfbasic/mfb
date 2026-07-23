# writeBytesAtomic

Atomically replace a file with a `List OF Byte`

## Synopsis

```
fs::writeBytesAtomic(path AS String, bytes AS List OF Byte) AS Nothing
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

`fs::writeBytesAtomic` writes the complete contents of `bytes` to a uniquely
named temporary file in the same directory as `path`, flushes that temporary
file to disk, closes it, and then renames it over `path`. A reader observing
`path` during the operation sees either the previous file or the fully written
new file, never a partially written one, so the replacement is all-or-nothing.
The final rename is atomic when the host filesystem supports atomic rename.
[[src/target/shared/code/fs/atomic.rs:lower_fs_atomic_write_helper]]

The replacement is also crash-durable: after the rename the containing directory
is itself flushed to disk, so once this function returns successfully the new
file survives a crash or power loss and never reverts to the previous contents.
The directory flush is best-effort — if the containing directory cannot be
opened or flushed the write is still reported as successful, because the atomic
rename has already completed.
[[src/target/shared/code/fs/atomic.rs:lower_fs_atomic_write_helper]]

The temporary file is created next to `path` with a name derived from `path`'s
final component plus a `.mfb-XXXXXX.tmp` suffix, where the host fills in the `X`
markers to make the name unique. Creating the temporary in the same directory as
`path` keeps both files on the same filesystem so the final rename is a
same-filesystem move rather than a copy.
[[src/target/shared/code/fs/atomic.rs:lower_fs_atomic_write_helper]]

The byte payload is written directly from the byte list's packed data region.
The write is retried until every byte has been written or the host reports an
output failure, so a short host write that transfers only part of the buffer is
resumed rather than treated as complete, and an interrupted (`EINTR`) write is
retried from the same cursor before any byte has moved. An empty byte list
produces an empty file at `path`. Bytes are written exactly as held in the list,
with no encoding, decoding, or newline translation, so the function is suitable
for binary data as well as text.
[[src/target/shared/code/fs/atomic.rs:lower_fs_atomic_write_helper]]

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory and may contain Unicode
characters when the host filesystem accepts those names. The string must not be
empty and must not contain an embedded NUL byte, because the host calls require a
NUL-terminated path. The containing directory of `path` must already exist and be
writable, since the temporary file is created there.
[[src/target/shared/code/fs/atomic.rs:lower_fs_atomic_write_helper]]

When any step before the final rename fails, `path` is left unchanged, and the
leftover temporary file is unlinked before the error is reported so a failed
write never litters the target directory with a stray temp. To replace a file in
place without the temporary-and-rename guarantee, use `fs::writeBytes`; for the
text equivalent of this function, use `fs::writeTextAtomic`.
[[src/target/shared/code/fs/atomic.rs:lower_fs_atomic_write_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the file to replace, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. Its containing directory must exist and be writable. [[src/builtins/fs.rs:call_param_names]] |
| `bytes` | `List OF Byte` | The bytes to write, in order, taken verbatim from the list's data region. An empty list produces an empty file at `path`. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing on success, after every byte has been written, flushed, the temporary file has been closed, and the rename over `path` has completed. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot be turned into a valid NUL-terminated host path. [[src/target/shared/code/fs/atomic.rs:lower_fs_atomic_write_helper]] |
| `77010001` | `ErrOutOfMemory` | An internal NUL-terminated copy of the temporary or final path cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050004` | `ErrNotFound` | The temporary file cannot be created or the rename fails because a path component does not exist, such as a missing containing directory (host `ENOENT`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies permission to create the temporary file or to perform the rename (host `EACCES`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | The rename fails because the target already exists in a form that cannot be replaced (host `EEXIST`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77020002` | `ErrOutput` | Writing, flushing, or closing the temporary file fails, or creating the temporary file or renaming it fails for any other host reason. [[src/target/shared/code/fs/atomic.rs:lower_fs_atomic_write_helper]] |

## Examples

Atomically write raw bytes to a file:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = [72, 105]
  fs::writeBytesAtomic("target/output.bin", bytes)
END SUB
```

Atomically replace a file's contents with bytes read from another file:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = fs::readBytes("source.bin")
  fs::writeBytesAtomic("copy.bin", bytes)
END SUB
```

## See also

- `mfb man fs writeBytes`
- `mfb man fs writeTextAtomic`
- `mfb man fs appendBytes`
- `mfb man fs readBytes`
