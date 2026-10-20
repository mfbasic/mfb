# Filesystem host errors (fs)

Every `fs` member that hands a path to the host reports a refusal through one
mapping, so the same condition raises the same `errorCode` whichever member met it
and whichever host it ran on. There are no per-platform differences in the codes
an `fs` member raises. The per-member Errors tables are owned by `./mfb man fs`;
this topic is the model behind them.
[[src/codegen/builtins/fs/gen_shared.rs:emit_fs_host_error_mapping]]

## The mapping

The host's code is POSIX `errno` on Linux and macOS and `GetLastError` on Windows.

| condition | POSIX | Windows | raises |
| --- | --- | --- | --- |
| the entry, or a directory above it, is missing | `ENOENT` | `ERROR_FILE_NOT_FOUND`, `ERROR_PATH_NOT_FOUND` (classified) | `ErrPathNotFound` |
| permission refused | `EACCES`, `EPERM` | `ERROR_ACCESS_DENIED` (classified where a file is required) | `ErrAccessDenied` |
| a file used as a directory | `ENOTDIR` | `ERROR_PATH_NOT_FOUND` (classified), `ERROR_DIRECTORY` | `ErrInvalidPath` |
| a directory where a file is required | `EISDIR`, `EPERM` from `unlink` (classified), a read-only open that succeeds (checked) | `ERROR_ACCESS_DENIED` (classified) | `ErrInvalidPath` |
| a name too long | `ENAMETOOLONG` | `ERROR_PATH_NOT_FOUND` (classified), `ERROR_FILENAME_EXCED_RANGE` | `ErrInvalidPath` |
| an unusable name | `EILSEQ` | `ERROR_INVALID_NAME` | `ErrInvalidPath` |
| a loop of symbolic links | `ELOOP` | `ERROR_CANT_RESOLVE_FILENAME` | `ErrInvalidPath` (`ErrAccessDenied` for a no-follow open) |
| an entry already there, on a create | `EEXIST` | `ERROR_FILE_EXISTS`, `ERROR_ALREADY_EXISTS` | `ErrAlreadyExists` |
| a non-empty directory, on `rmdir` | `ENOTEMPTY`, `EEXIST` | `ERROR_DIR_NOT_EMPTY` | `ErrResourceBusy` |
| anything else | | | `ErrReadFailed` for a read or lookup, `ErrWriteFailed` for a write |

The mapping emits only the arms the failed call can reach, chosen by the operation
(`FsHostOp`): "already exists" only for a create, "not empty" only for `rmdir`,
`EISDIR` only where a file is required. The fallback follows the direction of the
call — `readText`, `readBytes`, `listDirectory`, `canonicalPath`, `isWithin` and
`setCurrentDirectory` fall back to `ErrReadFailed`; the writers, `createTempFile`,
`createDirectory`, `createDirectories`, `deleteFile` and `deleteDirectory` to
`ErrWriteFailed`; the `open` family picks by the requested mode at run time
(`ErrReadFailed` for `"r"`). A failed open for reading never raises
`ErrWriteFailed`.
[[src/codegen/builtins/fs/gen_shared.rs:FsHostOp]]
[[src/codegen/builtins/fs/gen_shared.rs:FsFallback]]

## Classification

Where the host's code names two conditions, the mapping asks the shared helper
`_mfb_rt_fs_path_class` about the path the call was given, on the failure path
only. It answers *directory* (the path names an existing directory), *unusable*
(a component is longer than 255 — bytes on POSIX, UTF-16 units on Windows — or the
nearest existing ancestor is not a directory), or *other*.
[[src/codegen/builtins/fs/gen_classify.rs:lower_fs_path_class_helper]]

* Windows `ERROR_PATH_NOT_FOUND` is *unusable* → `ErrInvalidPath`, else
  `ErrPathNotFound`. Windows reports a missing parent, a file used as a directory
  and an over-long path all with this one code.
* Windows `ERROR_ACCESS_DENIED`, and POSIX `EPERM` from `unlink` (macOS's answer
  for a directory), is *directory* → `ErrInvalidPath`, else `ErrAccessDenied`,
  for the calls that require a file.
* A POSIX read-only `open` of a directory succeeds where Windows refuses it. The
  `open` family `stat`s the path after a successful read-only open and refuses a
  directory there; `readText` and `readBytes` classify on their read-failure paths
  instead (the read fails `EISDIR`, or on Linux ext4 the `lseek(SEEK_END)` size is
  one no allocation can satisfy).
[[src/codegen/builtins/fs/gen_open.rs:emit_posix_directory_open_refusal]]
[[src/codegen/builtins/fs/gen_atomic_write/read_path.rs:lower_fs_read_text_path_helper]]

The helper is emitted once per program, when a member that calls it is used.
[[src/codegen/builtins/fs/gen_classify.rs:CLASSIFYING_MEMBERS]]

## See Also

* ./mfb man fs — the per-member Errors tables
* ./mfb spec diagnostics error-codes — the code values and messages
