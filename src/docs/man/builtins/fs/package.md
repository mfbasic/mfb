# fs

Filesystem path, file, and directory operations

## Synopsis

```
IMPORT fs
fs::readText(path)
fs::writeText(path, value)
fs::exists(path)
fs::listDirectory(path)
fs::openFile(path)
```

## Description

The `fs` package provides filesystem access: one-shot whole-file reads and
writes, an open `File` handle for streaming I/O, purely syntactic path-string
manipulation, directory creation and listing, and existence tests. `fs` is a
built-in package: `IMPORT fs` needs no manifest dependency. [[src/builtins/fs.rs:is_fs_call]]

Paths are UTF-8 `String` values, interpreted as bytes and passed to the host
filesystem, so they may carry Unicode characters where the host accepts such
names. A path may be absolute or relative to the process current working
directory; relative paths resolve against `fs::currentDirectory()`. Every path
argument must be non-empty and free of embedded NUL bytes, because the host call
requires a NUL-terminated path. The path-syntax functions — `fs::pathJoin`,
`fs::pathNormalize`, `fs::pathDirName`, `fs::pathBaseName`, and
`fs::pathExtension` — are byte-oriented and never touch the filesystem, while
`fs::canonicalPath` and `fs::isWithin` consult the disk to resolve `.`, `..`,
and symlinks. Where a path names a symlink, the final component is followed (so
reads and writes act on the target) except in `fs::openFileNoFollow`, which
refuses a symlinked final component, and `fs::deleteFile`, which removes the link
itself. [[src/builtins/fs.rs:PATH_JOIN]]

Whole-file functions operate directly on a path. `fs::readText` and
`fs::readBytes` read the entire file in one call; `fs::writeText` and
`fs::writeBytes` replace its contents; `fs::appendText` and `fs::appendBytes` add
to it; and the `fs::writeTextAtomic` and `fs::writeBytesAtomic` variants stage
the new contents in a temporary file and swap it in with an OS rename so readers
never observe a partial write. Text functions require and produce well-formed
UTF-8; byte functions transfer a `List OF Byte` verbatim, with no encoding or
newline translation, and so suit binary data. [[src/builtins/fs.rs:call_return_type_name]]

Handle functions work through the opaque `File` resource type. `fs::open`,
`fs::openFile`, `fs::openFileNoFollow`, and `fs::createTempFile` return a `File`;
`fs::readLine`, `fs::readAll`, `fs::readAllBytes`, `fs::writeAll`,
`fs::writeAllBytes`, and `fs::eof` act on one. Portable open modes are
`"read"`/`"r"`, `"write"`/`"w"`, `"readWrite"`/`"rw"`, and `"append"`/`"a"`. A
`File` is an owned, non-copyable handle closed automatically by lexical drop when
its binding leaves scope; call `fs::close` only to release it earlier. Using a
`File` after it is closed fails. [[src/builtins/fs.rs:resource_close_function]]

Each `File` handle can independently opt in to output buffering. It is off by
default, so `fs::writeAll`/`fs::writeAllBytes` reach the OS immediately;
`fs::setBuffered(file, TRUE)` instead holds incremental writes in a per-handle
buffer that is drained on `fs::flush(file)`, when it fills, and — mandatorily — on
close (`fs::close` or scope drop), so buffered on-disk data is never stranded.
`fs::setBuffered(file, FALSE)` drains and disables it, and `fs::isBuffered(file)`
reports the current mode. Only incremental handle writes are buffered; whole-file
and atomic writes already issue one write and ignore the setting. A hard crash may
lose buffered bytes not yet flushed — flush or close for durability.

Directory functions create (`fs::createDirectory`, `fs::createDirectories`),
remove (`fs::deleteDirectory`), and inspect (`fs::listDirectory`) directories,
read or change the working directory (`fs::currentDirectory`,
`fs::setCurrentDirectory`), and report the host temporary directory
(`fs::tempDirectory`), which is also the default location `fs::createTempFile`
uses when called without one. `fs::listDirectory` returns entry names only,
excluding `.` and `..`, sorted in ascending byte-wise order for deterministic
results. The existence predicates `fs::exists`, `fs::fileExists`, and
`fs::directoryExists` return a `Boolean` and report a missing or unreadable path
as `FALSE` rather than raising; only an internal allocation failure can raise
from them. [[src/builtins/fs.rs:LIST_DIRECTORY]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | raised by the path and open functions when a path is empty or contains an embedded NUL byte, or when an open mode is not one of the portable modes [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77010001` | `ErrOutOfMemory` | raised by any function when an internal allocation fails, such as the NUL-terminated copy of a path, a `File` record, or the buffer or collection holding a result [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77030001` | `ErrPathNotFound` | raised by reading functions and by `fs::open` in a read mode when no entry exists at the path (host ENOENT) [[src/target/shared/code/error_constants.rs:ERR_PATH_NOT_FOUND_CODE]] |
| `77050004` | `ErrNotFound` | raised by writing, directory, and working-directory functions, and by `fs::isWithin`, when a path component cannot be resolved, such as a missing parent directory [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] |
| `77030003` | `ErrAccessDenied` | raised by any function when the host denies permission to the path (host EACCES) [[src/target/shared/code/error_constants.rs:ERR_ACCESS_DENIED_CODE]] |
| `77030002` | `ErrInvalidPath` | raised when a path is unusable as a path string, including a non-directory used as a directory component, an over-long path, an invalid byte sequence, or a symlink loop (host ENOTDIR, ENAMETOOLONG, EILSEQ, or ELOOP) [[src/target/shared/code/error_constants.rs:ERR_INVALID_PATH_CODE]] |
| `77050005` | `ErrAlreadyExists` | raised by `fs::createDirectory` when an entry already exists at the final path component (host EEXIST) [[src/target/shared/code/error_constants.rs:ERR_ALREADY_EXISTS_CODE]] |
| `77030005` | `ErrDirectoryNotEmpty` | raised by `fs::deleteDirectory` when the named directory still contains entries [[src/target/shared/code/error_constants.rs:ERR_DIRECTORY_NOT_EMPTY_CODE]] |
| `77020001` | `ErrRead` | raised by reading functions when a host read fails partway through, before the full contents have been read [[src/target/shared/code/error_constants.rs:ERR_READ_CODE]] |
| `77020002` | `ErrOutput` | raised by writing and open functions when the target is a directory or another non-writable entry, when opening fails for any other host reason, or when writing, flushing, or closing fails partway through [[src/target/shared/code/error_constants.rs:ERR_OUTPUT_CODE]] |
| `77020003` | `ErrEof` | raised by `fs::readLine` when the `File` is already at end of input before any byte is read [[src/target/shared/code/error_constants.rs:ERR_EOF_CODE]] |
| `77020004` | `ErrEncoding` | raised by `fs::readText` and `fs::readAll` when the bytes read are not valid UTF-8 [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
| `77030004` | `ErrResourceClosed` | raised by the `File`-handle functions when the `File` has already been closed [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77030006` | `ErrCloseFailed` | raised by `fs::close` when the host OS reports a failure while flushing or releasing the handle [[src/target/shared/code/error_constants.rs:ERR_CLOSE_FAILED_CODE]] |
