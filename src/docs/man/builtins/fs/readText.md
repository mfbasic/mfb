# readText

Read an entire UTF-8 text file into a `String`

## Synopsis

```
fs::readText(path AS String) AS String
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

`fs::readText` opens the file named by `path` for reading, reads its complete
contents in one call, closes the file, validates that the bytes are well-formed
UTF-8, and returns them as a `String`. The whole file is read at once — there is
no streaming and no partial result. No newline translation or other decoding is
performed beyond the UTF-8 validity check, so the returned `String` holds the
file's bytes exactly as stored on disk, interpreted as UTF-8.
[[src/target/shared/code/fs/atomic.rs:lower_fs_read_text_path_helper]]

Internally the function opens the file read-only, seeks to the end and back to
determine the length, allocates the result `String`, reads the bytes in a loop,
and closes the descriptor. The file is always closed before the function returns,
on both the success and the post-open failure paths. The byte length of the
returned `String` equals the byte length of the file at the moment it is read, so
an empty file yields an empty `String`. A partial read caused by the file
shrinking mid-read (an unexpected end of file) is a hard error, not a truncated
result. [[src/target/shared/code/fs/atomic.rs:lower_fs_read_text_path_helper]]

The final path component is followed when it is a symlink, so reading through a
symlink reads the target file. `path` is interpreted as UTF-8 bytes and passed to
the host filesystem; it may be absolute or relative to the current working
directory, and may contain Unicode characters when the host filesystem accepts
those names. The string must not be empty and must not contain an embedded NUL
byte, because the host `open` call requires a NUL-terminated path. Apart from
opening and closing the file descriptor, the call has no side effects. To read
arbitrary binary data without the UTF-8 requirement, use `fs::readBytes`.
[[src/target/shared/code/fs/atomic.rs:lower_fs_read_text_path_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the file to read, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. [[src/builtins/fs.rs:READ_TEXT]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The complete contents of the file as a UTF-8 `String`, in file order. An empty file returns an empty `String`. [[src/target/shared/code/fs/atomic.rs:lower_fs_read_text_path_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot be turned into a valid NUL-terminated host path. [[src/target/shared/code/fs/atomic.rs:lower_fs_read_text_path_helper]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of `path` or the `String` holding the file contents cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77030001` | `ErrPathNotFound` | No file exists at `path` (host `ENOENT`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies access to `path` (host `EACCES`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030002` | `ErrInvalidPath` | `path` is unusable as a path: a non-directory used as a directory component, an over-long path, an invalid byte sequence, or a symlink loop resolving the final component (host `ENOTDIR`, `ENAMETOOLONG`, `EILSEQ`, or `ELOOP`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The file cannot be opened for any other host reason not classified above. [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77020001` | `ErrRead` | Determining the file's length (seek) or reading its bytes fails partway through, before the full contents have been read. [[src/target/shared/code/fs/atomic.rs:ERR_READ_CODE]] |
| `77020004` | `ErrEncoding` | The bytes read from the file are not valid UTF-8. [[src/target/shared/code/fs/atomic.rs:ERR_ENCODING_CODE]] |

## Examples

Read a text file into a `String`:

```
IMPORT fs

SUB main()
  LET value AS String = fs::readText("data.txt")
END SUB
```

Write a file and read it back:

```
IMPORT fs
IMPORT io

SUB main()
  fs::writeText("greeting.txt", "hello")
  LET text AS String = fs::readText("greeting.txt")
  io::print(text)
END SUB
```

## See also

- `mfb man fs readBytes`
- `mfb man fs readAll`
- `mfb man fs writeText`
- `mfb man fs appendText`
