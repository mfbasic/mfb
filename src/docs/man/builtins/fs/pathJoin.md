# pathJoin

Join path components into a single path

## Synopsis

```
fs::pathJoin(parts AS List OF String) AS String
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

`fs::pathJoin` concatenates the path components in `parts` with the POSIX `/`
separator and returns the combined path as a `String`. The components are joined
in list order, inserting exactly one separator where one is needed so that no
duplicate slashes appear between components: a separator is added before a
component only when the result so far is non-empty and does not already end in
`/`. [[src/target/shared/code/fs_helpers_paths.rs:lower_fs_path_join_helper]]

Empty components are skipped entirely; they contribute neither text nor a
separator. If a component is absolute — its first byte is `/` — it discards
everything accumulated before it and the result restarts from that component, so
the last absolute component in the list determines the prefix of the result.
[[src/target/shared/code/fs_helpers_paths.rs:lower_fs_path_join_helper]]

The join is purely syntactic: it operates on the bytes of each component and
never consults the filesystem, resolves `.` or `..` segments, follows symbolic
links, or checks whether any path exists. Each component is interpreted as raw
UTF-8 bytes, so Unicode file names are preserved unchanged, and embedded NUL
bytes are copied verbatim rather than treated as terminators. An empty list, or
a list containing only empty components, yields the empty `String`. The function
reads no external state and has no side effects other than allocating the
returned `String`.
[[src/target/shared/code/fs_helpers_paths.rs:lower_fs_path_join_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `parts` | `List OF String` | The path components to join, in order, interpreted as raw UTF-8 bytes. Empty components are skipped; any component beginning with `/` is treated as absolute and resets the accumulated result. May be an empty list. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The joined path, with components separated by single `/` characters and empty components omitted. When a component is absolute, the result begins at that component. An empty list, or a list containing only empty components, yields the empty `String`. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The result `String` for the joined path cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Join a directory and a file name:

```
IMPORT fs
IMPORT io

SUB main()
  LET path AS String = fs::pathJoin(["target", "output.txt"])
  io::print(path)
END SUB
```

A trailing separator is not duplicated by the join:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathJoin(["target/", "output.txt"]))
END SUB
```

An absolute component discards everything joined before it:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathJoin(["ignored", "/etc", "hosts"]))
END SUB
```

## See also

- `mfb man fs pathNormalize`
- `mfb man fs pathDirName`
- `mfb man fs pathBaseName`
- `mfb man fs canonicalPath`
