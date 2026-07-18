# directoryExists

Test whether a path names an existing directory

## Synopsis

```
fs::directoryExists(path AS String) AS Boolean
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

`fs::directoryExists` stats `path` and reports whether it resolves to a
directory. It returns `TRUE` only when `path` exists and the resolved entry is a
directory; it returns `FALSE` for a missing path, a regular file, or any other
non-directory entry (symlink to a missing target, socket, FIFO, or device node).
The check masks the entry's mode with the file-type bits (`61440`) and compares
against the directory type (`16384`), so only directories qualify.
[[src/target/shared/code/mod.rs:1757]][[src/target/shared/code/error_constants.rs:FS_MODE_DIRECTORY]]

The final path component is followed when it is a symlink, because the host
`stat` call is used rather than `lstat`: a symlink pointing at a directory
reports `TRUE`, and a symlink whose target is missing or non-directory reports
`FALSE`. [[src/target/shared/code/fs_helpers_paths.rs:lower_fs_kind_exists_helper]]

A failed `stat` — for example a missing path or an unreadable parent directory —
is reported as `FALSE` rather than raised as an error. The only failure the call
itself raises is an allocation failure while preparing the NUL-terminated copy of
`path`. [[src/target/shared/code/fs_helpers_paths.rs:lower_fs_kind_exists_helper]]

`path` is interpreted as UTF-8 bytes and passed to the host filesystem; it may be
absolute or relative to the current working directory, and may contain Unicode
characters (including emoji) when the host filesystem accepts those names. The
call reads filesystem state only and has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path to test, as UTF-8 bytes; absolute or relative to the current working directory. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `path` exists and resolves to a directory; `FALSE` otherwise (missing path, regular file, or any other non-directory entry). [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The internal NUL-terminated copy of `path` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Test for a directory before listing it:

```
IMPORT fs
IMPORT io

IF fs::directoryExists("data") THEN
  io::print("found")
END IF
```

Unicode paths are accepted:

```
IMPORT fs

LET present AS Boolean = fs::directoryExists("é日😀")
```

## See also

- `mfb man fs fileExists`
- `mfb man fs exists`
