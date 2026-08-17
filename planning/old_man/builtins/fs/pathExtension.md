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
[[src/codegen/builtins/fs/mod.rs:register]]

## Description

`fs::pathExtension` returns the extension of `path`'s final component, including
the leading `.`, as a `String`. The operation is purely syntactic: it inspects
the bytes of `path` and never consults the filesystem, resolves `.` or `..`
segments, follows symbolic links, or checks whether any path exists.
[[src/codegen/builtins/fs/native/paths_builder.rs:lower_fs_path_extension]]

Trailing `/` separators are trimmed before the final component is located, so
`"target/output.txt"` and `"target/output.txt/"` both yield `".txt"`. Within that
component the bytes are scanned backward from the end and the scan stops at the
last `.`; the result spans from that `.` through the end of the component, so only
the final extension is returned and `"archive.tar.gz"` yields `".gz"`.
[[src/codegen/builtins/fs/native/paths_builder.rs:lower_fs_path_extension]]

The scan never crosses a `/`, so a `.` in an earlier component is ignored:
`"lib.d/output"` yields an empty `String`. When the final component contains no
`.`, an empty `String` is returned. When the only `.` is the first byte of the
component, that component is treated as a dotfile name and the whole name is
returned, so `".bashrc"` yields `".bashrc"`. An empty `path`, or a `path`
consisting only of `/` separators, returns an empty `String`.
[[src/codegen/builtins/fs/native/paths_builder.rs:lower_fs_path_extension]]

The scan is byte-oriented (the separator is the single byte `47` and the dot is
the single byte `46`), so UTF-8 file names are preserved unchanged and any
embedded bytes are treated literally. A new `String` holding the extension bytes
is allocated for the result. The function reads no external state and has no side
effects other than allocating the returned `String`.
[[src/codegen/builtins/fs/native/paths_builder.rs:lower_fs_path_extension]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The path whose extension is wanted, interpreted as raw UTF-8 bytes. Trailing `/` separators are ignored before the final component is located. May be empty. [[src/codegen/builtins/fs/mod.rs:register]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The extension of `path`'s final component, including the leading `.`. Returns the whole component when it is a dotfile name (its first byte is the only `.`), and an empty `String` when the final component has no `.`, when `path` is empty, or when `path` consists only of `/` separators. [[src/codegen/builtins/fs/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The result `String` for the extension cannot be allocated. [[src/codegen/builtins/errorcode/mod.rs:ErrOutOfMemory]] |

## Examples

A file name with an extension yields the extension:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathExtension("target/output.txt"))
END SUB
```

Only the final extension is returned:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathExtension("archive.tar.gz"))
END SUB
```

A component with no `.` yields an empty `String`:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathExtension("README"))
END SUB
```

A `.` in an earlier component is ignored:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathExtension("lib.d/output"))
END SUB
```

A dotfile name is returned whole:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathExtension(".bashrc"))
END SUB
```

## See also

- `mfb man fs pathBaseName`
- `mfb man fs pathDirName`
- `mfb man fs pathJoin`
- `mfb man fs pathNormalize`
- `mfb man fs canonicalPath`
