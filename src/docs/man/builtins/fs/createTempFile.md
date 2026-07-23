# createTempFile

Securely create and open a unique, freshly named temporary file

## Synopsis

```
fs::createTempFile() AS File
fs::createTempFile(directory AS String) AS File
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

`fs::createTempFile` creates a brand-new file with a unique, unpredictable name
and returns an open `File` resource referring to it. The name has the form
`mfb-<uuid>.tmp`, where `<uuid>` is a freshly generated version 4 UUID rendered
in the canonical 8-4-4-4-12 hexadecimal form. The random bytes that seed the
name are drawn from host entropy before the file is opened, so two calls
effectively never collide and the name cannot be guessed by another process.
[[src/target/shared/code/fs/atomic.rs:emit_uuid_v4_to_path]]

The file is opened read/write with exclusive-create semantics and permission
bits `0600` (octal), so the call always yields a freshly created, empty file
readable and writable only by the current user. Exclusive creation means the
call fails rather than reusing or truncating any pre-existing file, which
together with the random name closes the classic temporary-file race and
symlink-redirection attacks. The descriptor is also opened close-on-exec.
[[src/target/shared/code/fs/atomic.rs:temp_file_open_flags]]

Without an argument the file is created inside the host temporary directory, the
same location returned by `fs::tempDirectory`; that directory path is supplied
automatically as the `directory` argument for the zero-argument form. With a
`directory` argument the file is created directly inside that directory. The
argument names the containing directory, not the file — no name component of
your own is added. The directory must already exist and be writable, since the
new file is created there.
[[src/target/shared/nir/lower.rs:382]]

`directory` is interpreted as UTF-8 bytes and passed to the host filesystem. It
may be absolute or relative to the current working directory and may contain
Unicode characters when the host filesystem accepts those names. It must not be
empty and must not contain an embedded NUL byte, because the host `open` call
requires a NUL-terminated path.
[[src/target/shared/code/fs/atomic.rs:lower_fs_create_temp_file_helper]]

The returned `File` is positioned at the start of the empty file and is owned by
the caller. It is closed by lexical drop when the binding that holds it leaves
scope, or explicitly with `fs::close`. The file itself is not deleted on close;
removing it is the caller's responsibility, for example with `fs::deleteFile`.

## Overloads

**`fs::createTempFile() AS File`**

Creates the temporary file in the host temporary directory reported by
`fs::tempDirectory`.

**`fs::createTempFile(directory AS String) AS File`**

Creates the temporary file inside `directory` instead of the host temporary
directory.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `directory` | `String` | The path of an existing, writable directory in which to create the temporary file, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. When omitted, the host temporary directory is used. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `File` | An open, empty `File` resource for a newly created file opened read/write with `0600` permissions and positioned at the start of the file. The resource must eventually be closed, by scope drop or by `fs::close`. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | The directory path is empty or contains an embedded NUL byte, so it cannot be turned into a valid NUL-terminated host path. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77010001` | `ErrOutOfMemory` | The internal buffer for the constructed temporary path, or the `File` resource record, cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050004` | `ErrNotFound` | The file cannot be created because a path component does not exist, such as a missing containing directory (host `ENOENT`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies permission to create the file in the directory (host `EACCES`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | Exclusive creation fails because a file with the generated name already exists (host `EEXIST`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77020002` | `ErrOutput` | Random bytes for the name cannot be obtained, or the host refuses to create the file for any other reason not classified above. [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |

## Examples

Create a temporary file in the host temporary directory and write to it:

```
IMPORT fs

SUB main()
  RES f = fs::createTempFile()
  fs::writeAll(f, "data")
  ' f is closed by lexical drop when this scope ends
END SUB
```

Create a temporary file in a specific directory:

```
IMPORT fs

SUB main()
  RES g = fs::createTempFile("target")
  fs::writeAll(g, "data")
  ' g is closed by lexical drop when this scope ends
END SUB
```

## See also

- `mfb man fs tempDirectory`
- `mfb man fs open`
- `mfb man fs openFile`
- `mfb man fs close`
- `mfb man fs deleteFile`
