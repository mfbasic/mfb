# currentDirectory

Return the process's current working directory

## Synopsis

```
fs::currentDirectory() AS String
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

`fs::currentDirectory` returns the absolute current working directory of the
running process as a UTF-8 `String`.

The path is queried from the operating system with the host `getcwd` call on
every invocation rather than cached, so the result reflects the process's
working directory at the moment of the call. The returned path is absolute and
is given in the host's native spelling. Internally the path is read into a
fixed 4096-byte arena buffer, its length is measured up to the terminating NUL,
and those bytes are copied into an arena-backed `String`; the terminating NUL is
not included in the returned value.
[[src/codegen/builtins/fs/native/paths.rs:lower_fs_current_directory_helper]]

The working directory is the base against which any relative path passed to
other `fs` functions is resolved, so this value names the directory used by
`fs::canonicalPath`, `fs::open`, `fs::readText`, and the rest of the package
when they are given a path that is not absolute. The current directory can be
changed with `fs::setCurrentDirectory`.

The function takes no arguments, reads process state only, and has no filesystem
side effects: it does not create, open, or modify any file.
[[src/codegen/builtins/fs/mod.rs:register]]

## Parameters

This function takes no parameters. [[src/codegen/builtins/fs/mod.rs:register]]

## Return value

| Type | Description |
| --- | --- |
| `String` | The absolute path of the process's current working directory, decoded as a UTF-8 `String` in the host's native spelling, with the terminating NUL stripped. [[src/codegen/builtins/fs/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020001` | `ErrRead` | The host `getcwd` call fails, for example when the working directory has been removed, when access to a parent component is denied, or when the path does not fit in the internal 4096-byte buffer. [[src/builtins/errorcode.rs:ErrReadFailed]] |
| `77010001` | `ErrOutOfMemory` | The internal buffer used to query the path, or the returned `String` resource, cannot be allocated. [[src/builtins/errorcode.rs:ErrOutOfMemory]] |

## Examples

Read and print the current working directory:

```
IMPORT fs
IMPORT io

SUB main()
  LET cwd AS String = fs::currentDirectory()
  io::print(cwd)
END SUB
```

Resolve a relative path against the working directory:

```
IMPORT fs
IMPORT io

SUB main()
  LET cwd AS String = fs::currentDirectory()
  LET full AS String = fs::pathJoin([cwd, "output.txt"])
  io::print(full)
END SUB
```

## See also

- `mfb man fs setCurrentDirectory`
- `mfb man fs canonicalPath`
- `mfb man fs tempDirectory`
- `mfb man fs pathJoin`
