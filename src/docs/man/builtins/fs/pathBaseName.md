# pathBaseName

Return the final component of a path

## Synopsis

```
fs::pathBaseName(path AS String) AS String
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

`fs::pathBaseName` returns the final component of `path` — the part after the
last `/` separator — as a `String`. The operation is purely syntactic: it
inspects the bytes of `path` and never consults the filesystem, resolves `.` or
`..` segments, follows symbolic links, or checks whether any path exists.
[[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_base_name]]

Trailing `/` separators are trimmed before the final component is located, so
`"target/output/"` and `"target/output"` both yield `"output"`. Trimming stops
once a single character remains, so it never consumes the whole string. After
trimming, the remaining bytes are scanned backward for the last `/` separator and
everything following it becomes the result, so the returned `String` carries no
leading separator. [[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_base_name]]

When `path` contains no separator, it is returned unchanged. When `path` is `"/"`
itself, or trims down to a lone `/` because it consists only of separators (for
example `"//"` or `"///"`), `"/"` is returned. An empty `path` returns an empty
`String`. [[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_base_name]]

The scan is byte-oriented (the separator is the single byte `47`), so UTF-8 file
names are preserved unchanged and any embedded bytes are treated literally. A new
`String` holding the final-component bytes is allocated for the result. The
function reads no external state and has no side effects other than allocating the
returned `String`. [[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_base_name]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The path whose final component is wanted, interpreted as raw UTF-8 bytes. Trailing `/` separators are ignored. May be empty. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The final component of `path`, with no leading separator. Returns `path` unchanged when it has no separator, `"/"` when `path` is `"/"` or consists only of separators, and an empty `String` when `path` is empty. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The result `String` for the final component cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

A directory and a file name yield the file name:

```
IMPORT fs
IMPORT io

io::print(fs::pathBaseName("target/output.txt"))
```

The final component of an absolute path:

```
IMPORT fs
IMPORT io

io::print(fs::pathBaseName("/usr/local/bin"))
```

A trailing separator is ignored before the component is located:

```
IMPORT fs
IMPORT io

io::print(fs::pathBaseName("/usr/local/bin/"))
```

A path with no separator is returned unchanged:

```
IMPORT fs
IMPORT io

io::print(fs::pathBaseName("output.txt"))
```

The root path yields itself:

```
IMPORT fs
IMPORT io

io::print(fs::pathBaseName("/"))
```

## See also

- `mfb man fs pathDirName`
- `mfb man fs pathJoin`
- `mfb man fs pathNormalize`
- `mfb man fs pathExtension`
- `mfb man fs canonicalPath`
