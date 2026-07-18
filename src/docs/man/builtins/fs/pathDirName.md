# pathDirName

Return the directory portion of a path

## Synopsis

```
fs::pathDirName(path AS String) AS String
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

`fs::pathDirName` returns the directory portion of `path` — everything up to but
not including the final component — as a `String`. The operation is purely
syntactic: it inspects the bytes of `path` and never consults the filesystem,
resolves `.` or `..` segments, follows symbolic links, or checks whether any path
exists. [[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_dir_name]]

Trailing `/` separators are trimmed before the final component is located, so
`"target/output/"` and `"target/output"` both yield `"target"`. Trimming stops
once a single character remains, so it never consumes the whole string. After
trimming, the remaining bytes are scanned backward for the last `/` separator;
the separator that joins the directory to the final component is dropped, so the
result carries no trailing separator unless it is the root itself.
[[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_dir_name]]

When `path` contains no separator, `"."` is returned. When the last separator
found is at position `0` — the only separator is a leading `/` — or `path` is
`"/"` itself, `"/"` is returned. An empty `path` returns `"."`.
[[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_dir_name]]

The scan is byte-oriented (the separator is the single byte `47`), so UTF-8 file
names are preserved unchanged and any embedded bytes are treated literally. When
the result is `"."` or `"/"` a shared string constant is returned; otherwise a
new `String` holding the directory bytes is allocated. The function reads no
external state and has no side effects other than allocating the returned
`String`. [[src/target/shared/code/builder_fs_paths.rs:lower_fs_path_dir_name]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `String` | The path whose directory portion is wanted, interpreted as raw UTF-8 bytes. Trailing `/` separators are ignored. May be empty. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The directory portion of `path`, with no trailing separator unless it is the root `/`. Returns `"."` when `path` has no separator or is empty, and `"/"` when the only separator is a leading one or `path` is `"/"` itself. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The result `String` for the directory portion cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

A directory and a file name yield the directory:

```
IMPORT fs
IMPORT io

io::print(fs::pathDirName("target/output.txt"))
```

Leading separators are preserved in the result:

```
IMPORT fs
IMPORT io

io::print(fs::pathDirName("/usr/local/bin"))
```

A path with no separator yields `"."`:

```
IMPORT fs
IMPORT io

io::print(fs::pathDirName("output.txt"))
```

The root path yields itself:

```
IMPORT fs
IMPORT io

io::print(fs::pathDirName("/"))
```

## See also

- `mfb man fs pathBaseName`
- `mfb man fs pathJoin`
- `mfb man fs pathNormalize`
- `mfb man fs pathExtension`
- `mfb man fs canonicalPath`
