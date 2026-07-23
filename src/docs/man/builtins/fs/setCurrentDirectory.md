# setCurrentDirectory

Change the process's current working directory

## Synopsis

```
fs::setCurrentDirectory(path AS String) AS Nothing
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

`fs::setCurrentDirectory` changes the current working directory of the running
process to the directory named by `path`, using a single host change-directory
operation. On success the working directory has been changed and the function
returns `Nothing`. [[src/target/shared/code/mod.rs:FsPathOperation]]

The change affects the whole process, so every relative path passed to later
`fs` functions — including `fs::open`, `fs::readText`, `fs::canonicalPath`, and
`fs::listDirectory` — resolves against the new directory rather than the old
one. The new value can be read back with `fs::currentDirectory`.

The working directory is process-global, not per-thread: a change made on one
thread is observed by every other thread, and there is no thread-scoped current
directory. Relative-path `fs` operations are therefore not isolated between
concurrently running threads; a program that needs per-thread path resolution
must build absolute paths itself rather than relying on
`fs::setCurrentDirectory`.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may
be absolute or relative to the current working directory; a relative path, such
as `"tests"` or `".."`, is resolved against the existing working directory
before the change takes effect. The path may contain Unicode characters,
including emoji, when the host filesystem accepts those names. The string must
not be empty and must not contain an embedded NUL byte, because the host call
requires a NUL-terminated path; the helper allocates an internal
NUL-terminated copy of the path for the call and rejects an empty or
NUL-containing string before making it.
[[src/target/shared/code/fs/paths.rs:lower_fs_path_operation_helper]]

The named entry must exist and must be a directory the process is allowed to
enter; every component leading to it must itself be a traversable directory.
When the host refuses the operation for any reason the failure is mapped to the
matching error below and the working directory is left unchanged.
[[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the directory to become the new working directory. Interpreted as UTF-8 bytes; may be absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. The entry must exist and be a directory the process can enter. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing on success, after the process's working directory has been changed to the directory named by `path`. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot become a valid NUL-terminated host path. [[src/target/shared/code/fs/paths.rs:lower_fs_path_operation_helper]] |
| `77010001` | `ErrOutOfMemory` | The internal NUL-terminated copy of `path` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77030001` | `ErrPathNotFound` | `path`, or a component of it, does not exist (host `ENOENT`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies permission to enter `path` or a component leading to it (host `EACCES`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | The host reports an existing-entry conflict for the operation (host `EEXIST`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030005` | `ErrDirectoryNotEmpty` | The host reports a non-empty-directory conflict for the operation (host `ENOTEMPTY`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030002` | `ErrInvalidPath` | `path` is unusable as a directory: a non-directory used as a directory component, an over-long path, an invalid byte sequence, or a symlink loop (host `ENOTDIR`, `ENAMETOOLONG`, `EILSEQ`, or `ELOOP`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The host refuses the operation for any other reason. [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |

## Examples

Move into a subdirectory and back up to the parent:

```
IMPORT fs

SUB main()
  fs::setCurrentDirectory("tests")
  fs::setCurrentDirectory("..")
END SUB
```

Confirm the move by reading the working directory back:

```
IMPORT fs
IMPORT io

SUB main()
  fs::setCurrentDirectory("target")
  LET here AS String = fs::currentDirectory()
  io::print(here)
END SUB
```

## See also

- `mfb man fs currentDirectory`
- `mfb man fs canonicalPath`
- `mfb man fs directoryExists`
- `mfb man fs tempDirectory`
