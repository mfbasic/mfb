# pathExtension

Return the extension of a path's final component

## Synopsis

```
fs::pathExtension(path AS String) AS String
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

`fs::pathExtension` returns the extension of `path`'s final component, including
the leading `.`, as a `String`. The operation is purely syntactic: it inspects
the bytes of `path` and never consults the filesystem, resolves `.` or `..`
segments, follows symbolic links, or checks whether any path exists.
[[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_extension]]

Trailing `/` separators are trimmed before the final component is located, so
`"target/output.txt"` and `"target/output.txt/"` both yield `".txt"`. Within that
component the bytes are scanned backward from the end and the scan stops at the
last `.`; the result spans from that `.` through the end of the component, so only
the final extension is returned and `"archive.tar.gz"` yields `".gz"`.
[[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_extension]]

The scan never crosses a `/`, so a `.` in an earlier component is ignored:
`"lib.d/output"` yields an empty `String`. When the final component contains no
`.`, an empty `String` is returned. When the only `.` is the first byte of the
component, that component is treated as a dotfile name and the whole name is
returned, so `".bashrc"` yields `".bashrc"`. An empty `path`, or a `path`
consisting only of `/` separators, returns an empty `String`.
[[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_extension]]

The scan is byte-oriented (the separator is the single byte `47` and the dot is
the single byte `46`), so UTF-8 file names are preserved unchanged and any
embedded bytes are treated literally. A new `String` holding the extension bytes
is allocated for the result. The function reads no external state and has no side
effects other than allocating the returned `String`.
[[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_extension]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The path whose extension is wanted, interpreted as raw UTF-8 bytes. Trailing `/` separators are ignored before the final component is located. May be empty. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The extension of `path`'s final component, including the leading `.`. Returns the whole component when it is a dotfile name (its first byte is the only `.`), and an empty `String` when the final component has no `.`, when `path` is empty, or when `path` consists only of `/` separators. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The result `String` for the extension cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

A file name with an extension yields the extension:

```
IMPORT fs
IMPORT io

io::print(fs::pathExtension("target/output.txt"))
```

Only the final extension is returned:

```
IMPORT fs
IMPORT io

io::print(fs::pathExtension("archive.tar.gz"))
```

A component with no `.` yields an empty `String`:

```
IMPORT fs
IMPORT io

io::print(fs::pathExtension("README"))
```

A `.` in an earlier component is ignored:

```
IMPORT fs
IMPORT io

io::print(fs::pathExtension("lib.d/output"))
```

A dotfile name is returned whole:

```
IMPORT fs
IMPORT io

io::print(fs::pathExtension(".bashrc"))
```

## See also

- `mfb man fs pathBaseName`
- `mfb man fs pathDirName`
- `mfb man fs pathJoin`
- `mfb man fs pathNormalize`
- `mfb man fs canonicalPath`
