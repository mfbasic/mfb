# createDirectories

Create a directory together with any missing parent directories

## Synopsis

```
fs::createDirectories(path AS String) AS Nothing
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

`fs::createDirectories` creates the directory named by `path` along with any
missing parent directories, like `mkdir -p`, and returns `Nothing` on success.
[[src/builtins/fs.rs:call_return_type_name]]

`path` is scanned left to right and each `/`-separated prefix is created in turn
before the final component is created. A leading `/` is skipped so the filesystem
root is not treated as a component to create. For every prefix, and for the final
component, one host `mkdir` operation is attempted; a component that already
exists (host `EEXIST`, errno `17`) is accepted and the scan continues. As a
result, existing intermediate directories and a final `path` that already exists
as a directory all succeed quietly rather than being treated as errors, which
makes `fs::createDirectories` idempotent: re-running it on a path that is already
present succeeds without changing anything.
[[src/target/shared/code/fs_helpers_paths.rs:lower_fs_create_directories_helper]]

Unlike `fs::createDirectory`, which creates only the final component and fails
when a parent is missing, `fs::createDirectories` builds the entire chain of
missing parents. Each directory is requested with permission bits `0755`
(`rwxr-xr-x`), which the host masks with the process umask in the usual way, so
each directory's actual mode is `0755` with the umask bits cleared.
[[src/target/macos_aarch64/code.rs:emit_fs_path_operation]]

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory, and may contain Unicode
characters when the host filesystem accepts those names. Internally a
NUL-terminated copy of `path` is allocated for the host calls, and the `/`
separators in that copy are temporarily overwritten with NUL bytes to create each
prefix and restored afterward, so `path` must be non-empty and must not contain an
embedded NUL byte.
[[src/target/shared/code/fs_helpers_paths.rs:lower_fs_create_directories_helper]]

When the host refuses to create a prefix or the final component for any reason
other than `EEXIST`, the operation stops at that point and the failure `errno` is
mapped to the matching error below. Only `ENOENT` and `EACCES` are given specific
errors; every other refusal is reported as `ErrOutput`. `errno` values are per-OS;
the same symbolic error is produced on each platform.
[[src/target/shared/code/fs_helpers_paths.rs:lower_fs_create_directories_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the directory to create, including any parents that must be created first, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes. Every `/`-separated component is created in order, and components that already exist as directories are accepted. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Nothing is returned on success, after the directory named by `path` and all of its previously missing parent directories have been created. A `path` that already exists as a directory also succeeds and returns `Nothing`. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot become a valid NUL-terminated host path. [[src/target/shared/code/fs_helpers_paths.rs:lower_fs_create_directories_helper]] |
| `77010001` | `ErrOutOfMemory` | The internal NUL-terminated copy of `path` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050004` | `ErrNotFound` | The host cannot resolve a component while creating a prefix or the final directory (host `ENOENT`, errno `2`). [[src/target/shared/code/fs_helpers_paths.rs:lower_fs_create_directories_helper]] |
| `77030003` | `ErrAccessDenied` | The host denies permission to create a directory (host `EACCES`, errno `13`). [[src/target/shared/code/fs_helpers_paths.rs:lower_fs_create_directories_helper]] |
| `77020002` | `ErrOutput` | The host refuses the operation for any other reason. [[src/target/shared/code/fs_helpers_paths.rs:lower_fs_create_directories_helper]] |

## Examples

Create a nested directory together with its missing parents:

```
IMPORT fs

fs::createDirectories("target/example/nested")
```

Re-running is safe because existing directories are accepted:

```
IMPORT fs

fs::createDirectories("target/example/nested")
fs::createDirectories("target/example/nested")
```

## See also

- `mfb man fs createDirectory`
- `mfb man fs deleteDirectory`
- `mfb man fs directoryExists`
- `mfb man fs exists`
