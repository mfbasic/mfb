# isWithin

Test whether one path is contained within another

## Synopsis

```
fs::isWithin(base AS String, child AS String) AS Boolean
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

`fs::isWithin` canonicalizes both `base` and `child` with the host `realpath`
resolution, then reports whether `child` names the same location as `base` or a
location nested below it. It returns `TRUE` when the canonical `child` path
equals the canonical `base` path, or when it begins with the canonical `base`
path followed by a path separator; it returns `FALSE` otherwise.
[[src/target/shared/code/fs_helpers_paths.rs:lower_fs_is_within_helper]]

Canonicalization collapses `.` and `..` components, removes redundant
separators, and follows every symbolic link in both paths, resolving each
relative argument against the current working directory. The comparison
therefore reflects the real on-disk locations of the two paths rather than the
literal text of either argument, so symlink indirection and `..` traversal
cannot be used to make a path appear contained when it is not, nor to hide
genuine containment.

The containment test is path-boundary aware: it matches only at separator
boundaries, so `base` contains `base/nested/file.txt` and equals `base`, but a
sibling such as `base2` is reported as not within `base` even though its
canonical text shares the `base` prefix. When the canonical `base` is the
filesystem root (`/`), every canonical `child` is within it.
[[src/target/shared/code/fs_helpers_paths.rs:lower_fs_is_within_helper]]

Because canonicalization walks the real directory tree, every component of both
`base` and `child`, including the final one, must exist on the filesystem. Each
argument is interpreted as raw UTF-8 bytes and passed to the host; an argument
may contain Unicode characters when the host accepts such names, but it must not
be empty and must not contain an embedded NUL byte. The function reads
filesystem metadata only; it does not open, create, or modify any file and has
no other side effects.

This check is inherently subject to a time-of-check/time-of-use race: a
component of either path can be swapped for a symbolic link after `isWithin`
returns but before a later `fs::open` acts on the result. When the goal is to
open a caller-supplied name that cannot escape a trusted root, use
`fs::openWithin`, which enforces containment atomically at open time
(bug-259 / OS-03). [[src/target/shared/code/fs_helpers_io.rs:lower_fs_open_within_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `base` | `String` | The containing path (also accepted as `path`). Interpreted as UTF-8 bytes; may be absolute or relative to the current working directory. Every named component, including the last, must exist. Must be non-empty and free of embedded NUL bytes. [[src/builtins/fs.rs:call_param_names]] |
| `child` | `String` | The path tested for containment (also accepted as `parent`). Interpreted as UTF-8 bytes; may be absolute or relative to the current working directory. Every named component, including the last, must exist. Must be non-empty and free of embedded NUL bytes. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when the canonical `child` path equals the canonical `base` path or lies below it at a separator boundary; `FALSE` when `child` is a sibling, an ancestor, or otherwise outside `base`. [[src/target/shared/code/fs_helpers_paths.rs:lower_fs_is_within_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `base` or `child` is empty or contains an embedded NUL byte, so it cannot be turned into a valid NUL-terminated host path. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77010001` | `ErrOutOfMemory` | An internal NUL-terminated copy of an argument or a `realpath` resolution buffer cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050004` | `ErrNotFound` | `base` or `child`, or a required component of either, does not exist (host `ENOENT`). [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies access while canonicalizing `base` or `child` (host `EACCES`). [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | The host reports an existing-object conflict while canonicalizing `base` or `child` (host `EEXIST`). [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |
| `77020002` | `ErrOutput` | Canonicalization of `base` or `child` fails for any other host reason not classified above, such as a non-directory used as a directory component or a symlink loop. [[src/target/shared/code/fs_helpers.rs:emit_errno_error_mapping]] |

## Examples

Guard against escaping a root directory:

```
IMPORT fs

LET root AS String = fs::canonicalPath("uploads")
IF fs::isWithin(root, candidate) THEN
  io::print("inside")
END IF
```

A nested file is within its base directory:

```
IMPORT fs

fs::createDirectories("base/nested")
fs::writeText("base/nested/file.txt", "hi")
LET inside AS Boolean = fs::isWithin("base", "base/nested/file.txt")
```

A path is within itself, but a sibling is not:

```
IMPORT fs

LET same AS Boolean = fs::isWithin("base", "base")
LET sibling AS Boolean = fs::isWithin("base", "base2")
```

## See also

- `mfb man fs openWithin`
- `mfb man fs canonicalPath`
- `mfb man fs pathNormalize`
- `mfb man fs exists`
- `mfb man fs pathJoin`
