# canonicalPath

Resolve a path to its canonical absolute path

## Synopsis

```
fs::canonicalPath(path AS String) AS String
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

`fs::canonicalPath` resolves `path` to an absolute, canonical path and returns it
as a `String`. Resolution is performed by the host `realpath` call, which collapses
`.` and `..` components, removes redundant separators, and follows every symbolic
link encountered along the way, so the returned path names the real file or
directory with no indirection left in it. A relative `path` is resolved against the
current working directory; an absolute `path` is canonicalized in place.
[[src/target/shared/code/fs/paths.rs:lower_fs_canonical_path_helper]]

Because resolution walks the real directory tree rather than manipulating the
string alone, every component named by `path`, including the final one, must exist
on the filesystem; a missing component raises an error. To normalize a path
lexically without touching the filesystem, use `fs::pathNormalize` instead.
[[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]]

`path` is interpreted as raw UTF-8 bytes and passed to the host filesystem. It may
contain Unicode characters when the host accepts such names, and the byte-oriented
spelling of the name is preserved in the result. The string must not be empty and
must not contain an embedded NUL byte, because the host call requires a
NUL-terminated path; either condition raises `ErrInvalidArgument` before any host
call is made. The result is copied into an arena-backed `String` with the host
resolution buffer sized to hold up to `PATH_MAX` bytes plus the terminating NUL
(`4097`). [[src/target/shared/code/fs/paths.rs:PATH_MAX_PLUS_NUL]]

The function reads filesystem metadata only; it does not open, create, or modify
any file and has no other side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The path to canonicalize, as UTF-8 bytes; absolute or relative to the current working directory. Every named component, including the last, must exist. Must be non-empty and free of embedded NUL bytes. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The absolute, canonical path with all `.` and `..` components removed and all symbolic links resolved, in the host's native spelling. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty, or contains an embedded NUL byte, so it cannot be turned into a valid NUL-terminated host path. [[src/target/shared/code/fs/paths.rs:lower_fs_canonical_path_helper]] |
| `77050004` | `ErrNotFound` | `path` or a required component does not exist (host `ENOENT`, errno `2`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies access while resolving `path` (host `EACCES`, errno `13`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | The host reports an existing-object conflict while resolving `path` (host `EEXIST`, errno `17`). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77020002` | `ErrOutput` | Resolution fails for any other host reason, such as a non-directory used as a directory component or a symlink loop. [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77010001` | `ErrOutOfMemory` | The internal NUL-terminated copy of `path`, the resolution buffer, or the result `String` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Resolve a relative path against the working directory:

```
IMPORT fs
IMPORT io

SUB main()
  LET full AS String = fs::canonicalPath("target/output.txt")
  io::print(full)
END SUB
```

Canonicalize a path containing `.` and `..` components:

```
IMPORT fs
IMPORT io

SUB main()
  fs::createDirectories("a/b")
  LET real AS String = fs::canonicalPath("a/./b/../b")
  io::print(real)
END SUB
```

## See also

- `mfb man fs pathNormalize`
- `mfb man fs isWithin`
- `mfb man fs currentDirectory`
- `mfb man fs pathJoin`
