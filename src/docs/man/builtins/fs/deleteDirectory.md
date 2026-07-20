# deleteDirectory

Remove an empty directory from the filesystem

## Synopsis

```
fs::deleteDirectory(path AS String) AS Nothing
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

`fs::deleteDirectory` removes the empty directory named by `path` with a single
host `rmdir` operation. On success the directory is gone and the function returns
`Nothing`. [[src/target/shared/code/types.rs:FsPathOperation]][[src/builtins/fs.rs:call_return_type_name]]

The final component of `path` must name an actual directory, and that directory
must be empty. `fs::deleteDirectory` does not recurse and never removes a file or
a symbolic link; use `fs::deleteFile` to remove a non-directory entry. A directory
that still contains entries is left untouched and the call fails with
`ErrDirectoryNotEmpty`. Only the named directory is removed; parent directories are
left in place. [[src/target/shared/code/fs_helpers_paths.rs:lower_fs_path_operation_helper]]

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory, and may contain Unicode
characters when the host filesystem accepts those names. Internally a
NUL-terminated copy of `path` is allocated for the host call, so `path` must be
non-empty and must not contain an embedded NUL byte.
[[src/target/shared/code/fs_helpers_paths.rs:lower_fs_path_operation_helper]]

When the host refuses the removal, the failure `errno` is mapped to the matching
error below and the filesystem is left unchanged. `errno` values are per-OS; the
same symbolic error is produced on each platform.
[[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the directory to remove, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. The final component must name an existing, empty directory rather than a file or symbolic link. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Nothing is returned on success, after the empty directory named by `path` has been removed. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot become a valid NUL-terminated host path. [[src/target/shared/code/fs_helpers_paths.rs:lower_fs_path_operation_helper]] |
| `77010001` | `ErrOutOfMemory` | The internal NUL-terminated copy of `path` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77030001` | `ErrPathNotFound` | No entry exists at `path` (host `ENOENT`). [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies permission to remove the directory (host `EACCES`). [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | The host reports a conflicting existing target (host `EEXIST`). [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |
| `77030005` | `ErrDirectoryNotEmpty` | `path` names a directory that still contains entries (host `ENOTEMPTY`). [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |
| `77030002` | `ErrInvalidPath` | `path` is unusable as a path: the final component names a non-directory, a non-directory is used as a directory component, an over-long path, an invalid byte sequence, or a symlink loop (host `ENOTDIR`, `ENAMETOOLONG`, `EILSEQ`, or `ELOOP`). [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The host refuses the removal for any other reason. [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |

## Examples

Remove an empty directory:

```
IMPORT fs

SUB main()
  fs::deleteDirectory("target/example")
END SUB
```

Create a directory and then remove it:

```
IMPORT fs

SUB main()
  fs::createDirectory("target/scratch")
  fs::deleteDirectory("target/scratch")
END SUB
```

## See also

- `mfb man fs createDirectory`
- `mfb man fs createDirectories`
- `mfb man fs deleteFile`
- `mfb man fs directoryExists`
