# tempDirectory

Return the host temporary directory path

## Synopsis

```
fs::tempDirectory() AS String
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

`fs::tempDirectory` returns the path of the host's temporary directory as a
UTF-8 `String`. This is the same location `fs::createTempFile` uses when it is
called without a `directory` argument; that zero-argument form is lowered to
supply `fs::tempDirectory()` as the directory automatically.
[[src/target/shared/nir/lower.rs:363]]

The directory path is queried from the operating system on every call rather
than cached, so the result reflects the host environment at the moment of the
call. The returned `String` holds only the path bytes, with the trailing NUL
that the host query produces stripped off; no trailing path separator is added.
[[src/target/shared/code/fs_helpers_paths.rs:lower_fs_temp_directory_helper]]

The source of the path is platform specific:

- On macOS the per-process Darwin user temporary directory is read with
  `confstr(_CS_DARWIN_USER_TEMP_DIR, ...)`, a user-private location under the
  system temporary area. The reported length is the returned size minus one, to
  drop the terminating NUL. [[src/target/macos_aarch64/code.rs:emit_temp_directory]]
- On Linux the value of the `TMPDIR` environment variable is used when it is set,
  non-empty, and fits within the internal buffer; otherwise the path falls back
  to `/tmp`. [[src/target/linux_common/code.rs:emit_temp_directory]]

The function takes no arguments and has no filesystem side effects: it neither
creates the directory nor verifies that it exists, it only reports the
configured path. Internally it reads into a fixed 4096-byte buffer before
copying the result into an arena-backed `String`.
[[src/target/shared/code/fs_helpers_paths.rs:lower_fs_temp_directory_helper]]

## Parameters

This function takes no parameters. [[src/builtins/fs.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `String` | The path of the host temporary directory, decoded as a UTF-8 `String` with no terminating NUL and no added trailing separator. On macOS this is the Darwin per-process user temporary directory; on Linux it is `TMPDIR` when set and usable, otherwise `/tmp`. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020001` | `ErrRead` | The host fails to report a temporary directory path, for example when the platform query returns a zero-length or empty result. [[src/target/shared/code/error_constants.rs:ERR_READ_CODE]] |
| `77010001` | `ErrOutOfMemory` | The internal buffer used to query the path, or the returned `String` resource, cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Read and print the host temporary directory:

```
IMPORT fs
IMPORT io

SUB main()
  LET dir AS String = fs::tempDirectory()
  io::print(dir)
END SUB
```

Create a temporary file under the host temporary directory:

```
IMPORT fs

SUB main()
  RES f = fs::createTempFile()
  ' f is created under fs::tempDirectory() and closed by lexical drop
END SUB
```

## See also

- `mfb man fs createTempFile`
- `mfb man fs currentDirectory`
- `mfb man fs canonicalPath`
