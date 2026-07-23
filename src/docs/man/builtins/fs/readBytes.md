# readBytes

Read an entire file into a `List OF Byte`

## Synopsis

```
fs::readBytes(path AS String) AS List OF Byte
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

`fs::readBytes` opens the file named by `path` for reading, reads its complete
contents into a single `List OF Byte`, closes the file, and returns the byte
list. The whole file is read in one call — there is no streaming and no partial
result. Bytes are returned exactly as stored on disk, with no encoding, decoding,
or newline translation, so the function is suitable for binary data as well as
text. [[src/target/shared/code/fs/atomic.rs:lower_fs_read_bytes_path_helper]]

Internally the function opens the file read-only, wraps the descriptor in a fresh
`File` handle, and delegates to the same whole-file reader as `fs::readAllBytes`;
the file is always closed before the function returns, on both the success and the
read-failure paths. The returned list's length equals the byte length of the file
at the moment it is read, so an empty file yields an empty `List OF Byte`.
[[src/target/shared/code/fs/io.rs:lower_fs_read_all_bytes_helper]]

The final path component is followed when it is a symlink, so reading through a
symlink reads the target file. `path` is interpreted as UTF-8 bytes and passed to
the host filesystem; it may be absolute or relative to the current working
directory, and may contain Unicode characters when the host filesystem accepts
those names. The string must not be empty and must not contain an embedded NUL
byte, because the host `open` call requires a NUL-terminated path. Apart from
opening and closing the file descriptor, the call has no side effects.
[[src/target/shared/code/fs/atomic.rs:lower_fs_read_bytes_path_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the file to read, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The complete contents of the file as a `List OF Byte`, in file order. An empty file returns an empty list. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot be turned into a valid NUL-terminated host path. [[src/target/shared/code/fs/atomic.rs:lower_fs_read_bytes_path_helper]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of `path`, the `File` handle record, or the byte collection holding the file contents cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77030001` | `ErrPathNotFound` | No file exists at `path` (host `ENOENT`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies access to `path` (host `EACCES`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030002` | `ErrInvalidPath` | `path` is unusable as a path: a non-directory used as a directory component, an over-long path, an invalid byte sequence, or a symlink loop resolving the final component (host `ENOTDIR`, `ENAMETOOLONG`, `EILSEQ`, or `ELOOP`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The file cannot be opened for any other host reason not classified above. [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77020001` | `ErrRead` | Determining the file's length or reading its bytes fails partway through, before the full contents have been read. [[src/target/shared/code/fs/io.rs:lower_fs_read_all_bytes_helper]] |

## Examples

Read a binary file into a byte list:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = fs::readBytes("data.bin")
END SUB
```

Report the size of a file in bytes:

```
IMPORT fs
IMPORT io

SUB main()
  LET bytes AS List OF Byte = fs::readBytes("image.png")
  io::print("size: " & toString(len(bytes)))
END SUB
```

## See also

- `mfb man fs readText`
- `mfb man fs readAllBytes`
- `mfb man fs writeBytes`
- `mfb man fs appendBytes`
