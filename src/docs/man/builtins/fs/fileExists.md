# fileExists

Test whether a path names an existing regular file

## Synopsis

```
fs::fileExists(path AS String) AS Boolean
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

`fs::fileExists` stats `path` and reports whether it resolves to a regular file.
It returns `TRUE` only when `path` exists and the resolved entry is a regular
file; it returns `FALSE` for a missing path, a directory, or any other
non-regular entry (symlink to a missing target, socket, FIFO, or device node).
The check masks the entry's mode with the file-type bits (`61440`) and compares
against the regular-file type (`32768`), so only regular files qualify.
[[src/target/shared/code/fs/paths.rs:lower_fs_kind_exists_helper]][[src/target/shared/code/error_constants.rs:FS_MODE_REGULAR]]

The final path component is followed when it is a symlink, because the host
`stat` call is used rather than `lstat`: a symlink pointing at a regular file
reports `TRUE`, and a symlink whose target is missing or non-regular reports
`FALSE`. [[src/target/linux_common/code.rs:emit_path_stat]]

A failed `stat` — for example a missing path or an unreadable parent directory —
is reported as `FALSE` rather than raised as an error. The only failure the call
itself raises is an allocation failure while preparing the NUL-terminated copy of
`path`. [[src/target/shared/code/fs/paths.rs:lower_fs_kind_exists_helper]]

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
| `Boolean` | `TRUE` when `path` exists and resolves to a regular file; `FALSE` otherwise (missing path, directory, or any other non-regular entry). [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The internal NUL-terminated copy of `path` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Test for a regular file before reading it:

```
IMPORT fs
IMPORT io

SUB main()
  IF fs::fileExists("data.txt") THEN
    io::print("found")
  END IF
END SUB
```

Unicode paths are accepted:

```
IMPORT fs

SUB main()
  LET present AS Boolean = fs::fileExists("é日😀.txt")
END SUB
```

## See also

- `mfb man fs directoryExists`
- `mfb man fs exists`
