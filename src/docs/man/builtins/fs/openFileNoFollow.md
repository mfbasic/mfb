# openFileNoFollow

Open a file, refusing to traverse a symbolic link at any path component

## Synopsis

```
fs::openFileNoFollow(path AS String) AS File
fs::openFileNoFollow(path AS String, mode AS String) AS File
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

`fs::openFileNoFollow` opens the file named by `path` and returns an opaque
`File` resource that later `fs::` calls read from, write to, and close. It
behaves exactly like `fs::openFile` except that it refuses to traverse a
symbolic link at *any* component of `path`, not just the terminal name: if any
component — an intermediate directory or the final name — is a symbolic link,
the open is refused rather than resolving through the link.
[[src/builtins/fs.rs:OPEN_FILE_NO_FOLLOW]]

The whole-path guarantee is enforced by the host in a single operation. On Linux
the path is resolved with `openat2` carrying `RESOLVE_NO_SYMLINKS`; on macOS the
open uses `O_NOFOLLOW_ANY`, which fails if a symlink is encountered at any
component (bug-260 / OS-04). On a Linux kernel too old for `openat2` (`ENOSYS`,
pre-5.6, or a restrictive seccomp filter) it falls back to a plain `open` with
`O_NOFOLLOW`, which refuses only a symlinked *final* component.
[[src/target/shared/code/fs/io.rs:lower_fs_open_helper]]
[[src/target/shared/code/fs/io.rs:open_flag_set]]

This is useful for safely opening a file whose path you control without being
silently redirected through a symlink that may have been swapped in along the
path.

The `mode` argument is optional: when it is omitted the file is opened for
reading, exactly as if `"read"` had been supplied. The implicit `"read"` is
appended before lowering, matching `fs::openFile`.
[[src/target/shared/nir/lower.rs:apply_default_args]]

`mode` selects how the file is opened. The portable mode names are `"read"` or
`"r"`, `"write"` or `"w"`, `"readWrite"` or `"rw"`, and `"append"` or `"a"`.
`"read"` opens an existing file for reading only and creates nothing. `"write"`
opens the file for writing, creating it when it does not exist and truncating it
to empty when it does. `"readWrite"` opens the file for both reading and writing,
creating it when it does not exist but preserving existing contents. `"append"`
opens the file for writing with every write directed to the end of the file,
creating it when it does not exist. The mode string is matched exactly, byte for
byte, and is case sensitive; any other value is rejected before the file is
touched. [[src/target/shared/code/fs/io.rs:lower_fs_open_helper]]

Files created by a `write`, `readWrite`, or `append` open are created with
owner-only `0600` permission bits (subject to the process umask), not
world-readable `0666`, matching `fs::createTempFile` and the atomic writers
(audit-2 OS-01 / bug-184).
[[src/target/shared/code/fs/io.rs:lower_fs_open_helper]]

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory and may contain Unicode
characters when the host filesystem accepts those names. The string must not be
empty and must not contain an embedded NUL byte, because the host `open` call
requires a NUL-terminated path.
[[src/target/shared/code/fs/io.rs:lower_fs_open_helper]]

The returned `File` is closed by lexical drop when the binding that holds it
leaves scope, or explicitly with `fs::close`. The function reads or writes no
file contents itself; it only opens the descriptor and wraps it in the `File`
resource. [[src/target/shared/code/fs/io.rs:lower_fs_open_helper]]

## Overloads

**`fs::openFileNoFollow(path AS String) AS File`**

Opens `path` for reading, refusing a symlink at any component. Equivalent to
calling the two-argument overload with `mode` set to `"read"`; the implicit mode
is appended before lowering.
[[src/target/shared/nir/lower.rs:apply_default_args]]

**`fs::openFileNoFollow(path AS String, mode AS String) AS File`**

Opens `path` using the explicitly named access mode, refusing a symlink at any
component. [[src/builtins/fs.rs:OPEN_FILE_NO_FOLLOW]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the file to open, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. No component of the path may be a symbolic link. [[src/builtins/fs.rs:OPEN_FILE_NO_FOLLOW]] |
| `mode` | `String` | The access mode. Optional; defaults to `"read"` when omitted. One of `"read"`/`"r"` (read existing file), `"write"`/`"w"` (create or truncate for writing), `"readWrite"`/`"rw"` (create-if-absent for reading and writing, preserving contents), or `"append"`/`"a"` (create-if-absent for writing at end of file). Matched exactly and case sensitively. [[src/target/shared/code/fs/io.rs:lower_fs_open_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `File` | An open `File` resource positioned at the start of the file for `read`, `readWrite`, and `write` modes, and with writes directed to the end of the file for `append` mode. The resource must eventually be closed, by scope drop or by `fs::close`. [[src/target/shared/code/fs/io.rs:lower_fs_open_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty, `path` contains an embedded NUL byte, or `mode` is not one of the recognized portable mode names. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of `path` or the `File` resource record cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77030001` | `ErrPathNotFound` | A `read` open finds no file at `path`, or a directory component of `path` does not exist (host `ENOENT`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies access to `path` for the requested mode (host `EACCES`), or a symlink is encountered at any path component so the no-follow open is refused (host `ELOOP`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030002` | `ErrInvalidPath` | `path` is unusable as a path: a non-directory used as a directory component, an over-long path, or an invalid byte sequence (host `ENOTDIR`, `ENAMETOOLONG`, or `EILSEQ`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The file cannot be opened for any other host reason not classified above. [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |

## Examples

Open a file for reading using the default mode, refusing a symlinked path:

```
IMPORT fs

SUB main()
  RES f AS File = fs::openFileNoFollow("data.txt")
  fs::close(f)
END SUB
```

Open a file for writing; the open fails if any component of the path is a symlink:

```
IMPORT fs

SUB main()
  RES w AS File = fs::openFileNoFollow("out.txt", "write")
  fs::writeAll(w, "hello")
  fs::close(w)
END SUB
```

Open a file for appending so each write lands at the end:

```
IMPORT fs

SUB main()
  RES log AS File = fs::openFileNoFollow("app.log", "a")
  fs::writeAll(log, "started\n")
  fs::close(log)
END SUB
```

## See also

- `mfb man fs openFile`
- `mfb man fs open`
- `mfb man fs close`
- `mfb man fs readAll`
- `mfb man fs writeAll`
