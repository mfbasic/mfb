# createDirectory

Create a single directory whose parent already exists

## Synopsis

```
fs::createDirectory(path AS String) AS Nothing
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

`fs::createDirectory` creates the single directory named by `path` with one host
`mkdir` operation. On success the directory exists and the function returns
`Nothing`. [[src/target/macos_aarch64/code.rs:emit_fs_path_operation]][[src/builtins/fs.rs:call_return_type_name]]

Only the final component is created; every parent component must already exist.
`fs::createDirectory` does not create intermediate directories, so a `path` whose
parent is missing fails rather than building the chain. Use `fs::createDirectories`
to create a directory together with any missing parents, like `mkdir -p`.
[[src/target/shared/code/fs/paths.rs:lower_fs_path_operation_helper]]

The new directory is requested with permission bits `0755` (`rwxr-xr-x`), which the
host masks with the process umask in the usual way, so the directory's actual mode
is `0755` with the umask bits cleared. [[src/target/macos_aarch64/code.rs:emit_fs_path_operation]]

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory, and may contain Unicode
characters when the host filesystem accepts those names. Internally a
NUL-terminated copy of `path` is allocated for the host call, so `path` must be
non-empty and must not contain an embedded NUL byte.
[[src/target/shared/code/fs/paths.rs:lower_fs_path_operation_helper]]

`fs::createDirectory` never overwrites or reuses an existing entry: if anything
already exists at `path`, including an existing directory, the call fails with
`ErrAlreadyExists` rather than succeeding quietly. When the host refuses the
operation, the failure `errno` is mapped to the matching error below and the
filesystem is left unchanged. `errno` values are per-OS; the same symbolic error is
produced on each platform. [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the directory to create, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. Only the final component is created; every parent component must already exist as a directory. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Nothing is returned on success, after the directory named by `path` has been created. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot become a valid NUL-terminated host path. [[src/target/shared/code/fs/paths.rs:lower_fs_path_operation_helper]] |
| `77010001` | `ErrOutOfMemory` | The internal NUL-terminated copy of `path` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77030001` | `ErrPathNotFound` | A parent component of `path` does not exist (host `ENOENT`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies permission to create the directory (host `EACCES`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | An entry already exists at `path`, including an existing directory (host `EEXIST`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030005` | `ErrDirectoryNotEmpty` | The host reports a non-empty directory conflict for the operation (host `ENOTEMPTY`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030002` | `ErrInvalidPath` | `path` is unusable as a path: a non-directory used as a directory component, an over-long path, an invalid byte sequence, or a symlink loop (host `ENOTDIR`, `ENAMETOOLONG`, `EILSEQ`, or `ELOOP`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The host refuses the operation for any other reason. [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |

## Examples

Create a single output directory whose parent already exists:

```
IMPORT fs

SUB main()
  fs::createDirectory("target/example")
END SUB
```

Guard against re-creating a directory that already exists:

```
IMPORT fs

SUB main()
  IF NOT fs::directoryExists("target/cache") THEN
    fs::createDirectory("target/cache")
  END IF
END SUB
```

## See also

- `mfb man fs createDirectories`
- `mfb man fs deleteDirectory`
- `mfb man fs directoryExists`
- `mfb man fs exists`
