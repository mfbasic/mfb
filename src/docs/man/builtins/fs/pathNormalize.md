# pathNormalize

Normalize a path string syntactically

## Synopsis

```
fs::pathNormalize(path AS String) AS String
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

`fs::pathNormalize` returns a normalized form of `path` as a `String` without ever
consulting the filesystem. The normalization is purely syntactic: repeated `/`
separators are collapsed to a single separator, every `.` component is removed, and
each `..` component removes the preceding normal component when one is available to
remove. [[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_normalize]]

A leading `/` is preserved, so an absolute path stays absolute and a `..`
immediately after the root has nothing to remove and is dropped. In a relative path
a leading `..` (or a run of them) has no earlier component to cancel, so each such
`..` is kept in place. When normalization would leave nothing at all — for example
the inputs `""`, `"."`, or `"a/.."` — the result is `"."` so that the returned path
always names something.
[[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_normalize]]

The operation is byte-oriented over the path syntax: only the `/` separator and the
`.` and `..` spellings are interpreted, while all other bytes are copied through
unchanged. UTF-8 file names are therefore preserved exactly, and the function never
resolves symbolic links, accesses any file, or checks whether any path exists. To
resolve a path against the real directory tree instead, use `fs::canonicalPath`. The
normalized output is never longer than the input, and the function has no side
effects other than allocating the returned `String`.
[[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_normalize]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The path to normalize, interpreted as raw UTF-8 bytes with `/` as the component separator. May be absolute or relative, and may be empty. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The lexically normalized path: redundant separators collapsed, `.` removed, and `..` applied against preceding normal components. A leading `/` is retained for absolute paths, unresolvable leading `..` components in relative paths are kept, and `"."` is returned whenever normalization leaves the path empty. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The result `String` for the normalized path cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Redundant separators and `.` components are removed:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathNormalize("target//a/./file.txt"))
END SUB
```

A `..` component cancels the preceding component:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathNormalize("/usr/local/../bin"))
END SUB
```

Normalizing to nothing yields `"."`:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathNormalize("a/b/../.."))
END SUB
```

## See also

- `mfb man fs canonicalPath`
- `mfb man fs pathJoin`
- `mfb man fs pathDirName`
- `mfb man fs pathBaseName`
- `mfb man fs isWithin`
