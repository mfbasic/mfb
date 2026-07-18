# deleteFile

Remove a single file (or symlink) from the filesystem

## Synopsis

```
fs::deleteFile(path AS String) AS Nothing
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

`fs::deleteFile` removes the filesystem entry named by `path` with a single host
`unlink` operation. On success the entry is gone and the function returns
`Nothing`. [[src/target/linux_x86_64/code.rs:emit_fs_path_operation]][[src/builtins/fs.rs:call_return_type_name]]

When the final component of `path` is a symbolic link, the link itself is removed
rather than the file it points to, because `unlink` does not follow a trailing
symlink. The function removes exactly one non-directory entry; it does not recurse
and it does not remove directories. Use `fs::deleteDirectory` to remove a
directory. [[src/target/macos_aarch64/code.rs:emit_fs_path_operation]]

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory, and may contain Unicode
characters when the host filesystem accepts those names. Internally a
NUL-terminated copy of `path` is allocated for the host call, so `path` must be
non-empty and must not contain an embedded NUL byte.
[[src/target/shared/code/fs_helpers_paths.rs:lower_fs_path_operation_helper]]

When the host refuses the removal, the failure `errno` is mapped to the matching
error below and `path` is left unchanged. Attempting to remove a directory is
reported as a host failure (for example `ErrInvalidPath` or `ErrDirectoryNotEmpty`)
rather than as a directory-specific error, since `unlink` does not operate on
directories. `errno` values are per-OS; the same symbolic error is produced on
each platform. [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the entry to remove, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. A trailing symlink is removed rather than followed. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Nothing is returned on success, after the entry named by `path` has been removed. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot become a valid NUL-terminated host path. [[src/target/shared/code/fs_helpers_paths.rs:lower_fs_path_operation_helper]] |
| `77010001` | `ErrOutOfMemory` | The internal NUL-terminated copy of `path` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77030001` | `ErrPathNotFound` | No entry exists at `path` (host `ENOENT`). [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies permission to remove the entry (host `EACCES`). [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |
| `77030002` | `ErrInvalidPath` | `path` is unusable as a path: a non-directory used as a directory component, an over-long path, an invalid byte sequence, or a symlink loop (host `ENOTDIR`, `ENAMETOOLONG`, `EILSEQ`, or `ELOOP`). [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | The host reports a conflicting existing target (host `EEXIST`). [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |
| `77030005` | `ErrDirectoryNotEmpty` | The host reports the entry is a non-empty directory (host `ENOTEMPTY`). [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The host refuses the removal for any other reason, such as `path` naming a directory or an otherwise non-removable entry. [[src/target/shared/code/fs_helpers.rs:emit_fs_path_errno_error_mapping]] |

## Examples

Remove a generated output file:

```
IMPORT fs

fs::deleteFile("target/output.txt")
```

Write a file and then remove it:

```
IMPORT fs

fs::writeText("scratch.txt", "temporary")
fs::deleteFile("scratch.txt")
```

## See also

- `mfb man fs deleteDirectory`
- `mfb man fs exists`
- `mfb man fs fileExists`
- `mfb man fs writeTextAtomic`
