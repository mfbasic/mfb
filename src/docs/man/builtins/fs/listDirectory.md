# listDirectory

List the direct child names of a directory

## Synopsis

```
fs::listDirectory(path AS String) AS List OF String
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

`fs::listDirectory` opens the directory named by `path`, reads every entry it
contains directly, and returns those entry names as a `List OF String`. The list
holds the entry names only, not full paths, and the special `"."` (current
directory) and `".."` (parent directory) entries are always filtered out, so
they never appear in the result.
[[src/target/shared/code/fs/paths.rs:lower_fs_list_directory_helper]]

Only the immediate children of the directory are listed; `fs::listDirectory`
does not descend into subdirectories. Every kind of entry is included regardless
of type, so the result mixes regular files, subdirectories, symlinks, and any
other filesystem objects, each represented by its name with no trailing slash or
type marker.
[[src/target/shared/code/fs/paths.rs:lower_fs_list_directory_helper]]

The names are sorted in ascending byte-wise order, comparing their raw UTF-8
bytes (an ordinary lexicographic ordering for ASCII names), so the result is
deterministic and stable across runs and across hosts. An empty directory, or a
directory that contains only `"."` and `".."`, yields an empty `List`.
[[src/target/shared/code/fs/paths.rs:SORT_STRING_LIST_SYMBOL]]

Internally the directory is scanned in two passes: the first pass opens, reads,
and closes it to count the entries and their name bytes so the result `List` can
be sized, and the second pass opens, reads, and closes it again to fill the list
before sorting. If a concurrent writer grows the directory between the two
scans, the extra entries are truncated to the sized capacity rather than
overflowing the arena block, and the header is trimmed to what the second pass
actually wrote. The final path component is followed when it is a symlink, so
listing through a symlink that points at a directory lists the target
directory's entries.
[[src/target/shared/code/fs/paths.rs:lower_fs_list_directory_helper]]

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may
be absolute or relative to the current working directory and may contain Unicode
characters, including emoji, when the host filesystem accepts those names. The
string must not be empty and must not contain an embedded NUL byte, because the
host call requires a NUL-terminated path; an internal NUL-terminated copy of the
path is allocated for the call. Apart from opening and closing the directory,
the call only reads the filesystem and has no side effects.
[[src/target/shared/code/fs/paths.rs:lower_fs_list_directory_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The filesystem path of the directory to list, as UTF-8 bytes; absolute or relative to the current working directory. Must be non-empty and free of embedded NUL bytes, and must name an existing, readable directory. [[src/builtins/fs.rs:LIST_DIRECTORY]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF String` | A `List` containing the name of each direct child of the directory, excluding `"."` and `".."`, sorted in ascending byte-wise (UTF-8 lexicographic) order. An empty directory returns an empty `List`. [[src/target/shared/code/fs/paths.rs:lower_fs_list_directory_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `path` is empty or contains an embedded NUL byte, so it cannot be turned into a valid NUL-terminated host path. [[src/target/shared/code/fs/paths.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77010001` | `ErrOutOfMemory` | The internal NUL-terminated copy of `path`, or the `List` and its backing storage for the entry names, cannot be allocated. [[src/target/shared/code/fs/paths.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050004` | `ErrNotFound` | No entry exists at `path` (host `ENOENT`, errno 2). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies permission to open or read the directory (host `EACCES`, errno 13). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77050005` | `ErrAlreadyExists` | The host reports an existing-entry conflict while opening the directory (host `EEXIST`, errno 17). [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The directory cannot be opened or read for any other host reason, including when `path` names a regular file or other non-directory entry rather than a directory. [[src/target/shared/code/fs/mod.rs:emit_errno_error_mapping]] |

## Examples

Print every entry in a directory in sorted order:

```
IMPORT fs
IMPORT io
IMPORT collections

SUB main()
  LET names AS List OF String = fs::listDirectory("target")
  FOR i = 0 TO len(names) - 1
    io::print(collections::get(names, i))
  NEXT
END SUB
```

An empty directory yields an empty `List`:

```
IMPORT fs
IMPORT io

SUB main()
  fs::createDirectory("target/empty")
  LET names AS List OF String = fs::listDirectory("target/empty")
  io::print(toString(len(names)))
END SUB
```

## See also

- `mfb man fs directoryExists`
- `mfb man fs createDirectory`
- `mfb man fs exists`
- `mfb man fs canonicalPath`
