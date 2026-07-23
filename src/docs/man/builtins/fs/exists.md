# exists

Test whether any filesystem entry exists at a path

## Synopsis

```
fs::exists(path AS String) AS Boolean
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

`fs::exists` checks `path` and reports whether any filesystem entry is present
there, regardless of its type. It returns `TRUE` when an entry exists — a regular
file, a directory, a symlink to an existing target, a socket, a FIFO, or a device
node — and `FALSE` when nothing exists at `path`. The check is implemented with
the host `access` call using the existence mode (`F_OK`, `0`); `access` returning
`0` maps to `TRUE` and any nonzero result maps to `FALSE`.
[[src/target/shared/code/fs/paths.rs:lower_fs_exists_helper]][[src/target/linux_common/code.rs:emit_path_exists]]

The final path component is followed when it is a symlink, because `access`
dereferences the last component: a symlink pointing at an existing target reports
`TRUE`, and a symlink whose target is missing reports `FALSE`.
[[src/target/macos_aarch64/code.rs:emit_path_exists]]

A failed check — for example a missing path or an unreadable parent directory — is
reported as `FALSE` rather than raised as an error. The only failure the call
itself raises is an allocation failure while preparing the NUL-terminated copy of
`path`. [[src/target/shared/code/fs/paths.rs:lower_fs_exists_helper]]

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
| `Boolean` | `TRUE` when any entry exists at `path`; `FALSE` otherwise (missing path, or a symlink whose target is missing). [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The internal NUL-terminated copy of `path` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Test for any entry at a path before acting on it:

```
IMPORT fs
IMPORT io

SUB main()
  IF fs::exists("data.txt") THEN
    io::print("found")
  END IF
END SUB
```

Unicode paths are accepted:

```
IMPORT fs

SUB main()
  LET present AS Boolean = fs::exists("é日😀.txt")
END SUB
```

## See also

- `mfb man fs fileExists`
- `mfb man fs directoryExists`
