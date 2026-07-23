# openWithin

Open a file resolved beneath a trusted root directory, refusing any escape

## Synopsis

```
fs::openWithin(root AS String, relPath AS String) AS File
fs::openWithin(root AS String, relPath AS String, mode AS String) AS File
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

`fs::openWithin` opens the file named by `relPath` resolved **beneath** the
trusted directory `root`, and returns an opaque `File` resource. Its purpose is
to open a caller-controlled name inside an intended directory with a host-enforced
guarantee that the result cannot escape that directory — closing the
time-of-check/time-of-use race that an `fs::isWithin(root, path)` check followed
by a separate `fs::open(path)` leaves open (bug-259 / OS-03).
[[src/builtins/fs.rs:OPEN_WITHIN]]

Containment is enforced at open time. `root` is canonicalized once with `realpath`
(resolving the trusted root's own symbolic links); `relPath` is rejected if it is
absolute or contains a `..` component; the canonical root and `relPath` are joined;
and the join is opened with the same whole-path no-symlink resolution as
`fs::openFileNoFollow` — on Linux `openat2` carrying `RESOLVE_NO_SYMLINKS`, on
macOS `O_NOFOLLOW_ANY`. Because the canonical root is symlink-free and every
component is re-checked at open time, a component swapped to a symbolic link
*after* canonicalization is **rejected** rather than followed, so the open cannot
be redirected outside `root`.
[[src/target/shared/code/fs/io.rs:lower_fs_open_within_helper]]

A `relPath` is therefore refused when it is absolute, contains a `..` component,
or traverses a symbolic link at any component. `relPath` is always interpreted
relative to `root`, never to the process working directory.
[[src/target/shared/code/fs/io.rs:lower_fs_open_within_helper]]

The `mode` argument is optional: when it is omitted the file is opened for
reading, exactly as if `"read"` had been supplied. The implicit `"read"` is
appended before lowering, matching `fs::openFile`.
[[src/target/shared/nir/lower.rs:apply_default_args]]

`mode` selects how the file is opened. The portable mode names are `"read"` or
`"r"`, `"write"` or `"w"`, `"readWrite"` or `"rw"`, and `"append"` or `"a"`.
`"read"` opens an existing file for reading only and creates nothing. `"write"`
opens the file for writing, creating it when it does not exist and truncating it
to empty when it does. `"readWrite"` opens the file for both reading and writing,
creating it when it does not exist but preserving existing contents. `"append"`
opens the file for writing with every write directed to the end of the file,
creating it when it does not exist. The mode string is matched exactly, byte for
byte, and is case sensitive; any other value is rejected before the file is
touched. [[src/target/shared/code/fs/io.rs:lower_fs_open_within_helper]]

Files created by a `write`, `readWrite`, or `append` open are created with
owner-only `0600` permission bits (subject to the process umask), not
world-readable `0666`, matching `fs::createTempFile` and the atomic writers
(audit-2 OS-01 / bug-184).
[[src/target/shared/code/fs/io.rs:lower_fs_open_within_helper]]

`root` and `relPath` are interpreted as UTF-8 bytes and passed to the host
filesystem. `root` must resolve to an existing directory (it is canonicalized
with `realpath`), and neither string may be empty or contain an embedded NUL
byte. The returned `File` is closed by lexical drop when the binding that holds
it leaves scope, or explicitly with `fs::close`.
[[src/target/shared/code/fs/io.rs:lower_fs_open_within_helper]]

## Overloads

**`fs::openWithin(root AS String, relPath AS String) AS File`**

Opens `relPath` beneath `root` for reading. Equivalent to calling the
three-argument overload with `mode` set to `"read"`; the implicit mode is
appended before lowering.
[[src/target/shared/nir/lower.rs:apply_default_args]]

**`fs::openWithin(root AS String, relPath AS String, mode AS String) AS File`**

Opens `relPath` beneath `root` using the explicitly named access mode.
[[src/builtins/fs.rs:OPEN_WITHIN]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `root` | `String` | The trusted base directory. Canonicalized with `realpath` (its own symlinks are resolved); must resolve to an existing directory and be free of embedded NUL bytes. [[src/builtins/fs.rs:OPEN_WITHIN]] |
| `relPath` | `String` | The path to open, relative to `root`. Rejected if it is empty, absolute, contains a `..` component, or traverses a symbolic link at any component. [[src/target/shared/code/fs/io.rs:lower_fs_open_within_helper]] |
| `mode` | `String` | The access mode. Optional; defaults to `"read"` when omitted. One of `"read"`/`"r"`, `"write"`/`"w"`, `"readWrite"`/`"rw"`, or `"append"`/`"a"`. Matched exactly and case sensitively. [[src/target/shared/code/fs/io.rs:lower_fs_open_within_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `File` | An open `File` resource beneath `root`, positioned at the start of the file for `read`, `readWrite`, and `write` modes, and with writes directed to the end of the file for `append` mode. The resource must eventually be closed, by scope drop or by `fs::close`. [[src/target/shared/code/fs/io.rs:lower_fs_open_within_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `root` or `relPath` is empty or contains an embedded NUL byte, `relPath` is absolute or contains a `..` component, or `mode` is not one of the recognized portable mode names. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copies, the join buffer, or the `File` resource record cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77030001` | `ErrPathNotFound` | `root` does not resolve, or a `read` open finds no file at the join (host `ENOENT`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030003` | `ErrAccessDenied` | The host denies access (host `EACCES`), or a symbolic link is encountered at any component of the join so the no-follow open is refused (host `ELOOP`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77030002` | `ErrInvalidPath` | The join is unusable as a path: a non-directory used as a directory component, an over-long path, or an invalid byte sequence (host `ENOTDIR`, `ENAMETOOLONG`, or `EILSEQ`). [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |
| `77020002` | `ErrOutput` | The file cannot be opened for any other host reason not classified above. [[src/target/shared/code/fs/mod.rs:emit_fs_path_errno_error_mapping]] |

## Examples

Open a caller-supplied name beneath a fixed root, for reading:

```
IMPORT fs

SUB main()
  LET userName AS String = "alice.txt"
  RES f AS File = fs::openWithin("/srv/data", userName)
  fs::close(f)
END SUB
```

A `relPath` that tries to escape the root is refused rather than followed:

```
IMPORT fs
IMPORT errorCode
IMPORT io

SUB main()
  RES f AS File = fs::openWithin("/srv/data", "../../etc/passwd") TRAP(e)
    io::print(toString(e.code = errorCode::ErrInvalidArgument))
    EXIT SUB
  END TRAP
END SUB
```

Write beneath the root; a symlinked component makes the open fail:

```
IMPORT fs

SUB main()
  RES w AS File = fs::openWithin("/srv/data", "reports/today.txt", "write")
  fs::writeAll(w, "hello")
  fs::close(w)
END SUB
```

## See also

- `mfb man fs openFileNoFollow`
- `mfb man fs isWithin`
- `mfb man fs openFile`
- `mfb man fs close`
- `mfb man fs writeAll`
