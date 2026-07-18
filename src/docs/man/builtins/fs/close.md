# close

Close an open `File` resource and release its operating-system handle

## Synopsis

```
fs::close(file AS File) AS Nothing
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

`fs::close` releases the operating-system file descriptor behind an open `File`,
then returns nothing. Before releasing the descriptor it drains any output held in
the handle's per-handle buffer (see `fs::setBuffered`) so buffered on-disk data is
never stranded; the drain is a no-op on an unbuffered handle, which is the default.
[[src/target/shared/code/fs_helpers_io.rs:lower_fs_close_helper]]

The `File` is marked closed regardless of the outcome of the underlying `close`. On
some platforms a failing `close` (for example `EINTR` or `EIO`) has still released
the descriptor, so leaving the handle usable would let a later call drain and close
the same descriptor number — which by then may name an unrelated open file. Setting
the closed flag first means any later `fs::` call that takes the same `File` is
refused rather than touching a stale or reused descriptor, and a re-close raises an
error instead of repeating the release. [[src/target/shared/code/fs_helpers_io.rs:lower_fs_close_helper]]

Closing is otherwise automatic. Every `File` returned by `fs::open`, `fs::openFile`,
`fs::openFileNoFollow`, `fs::openWithin`, or `fs::createTempFile` is closed by
lexical drop when the `RES` binding that holds it leaves scope, and that drop drains
the buffer the same way. Call `fs::close` only when the descriptor must be released
earlier than scope exit — for example to reopen the same path, to let another process
observe writes, or to bound how many descriptors a long-running program holds open at
once. Closing a `File` and then letting it drop is safe: the drop sees the closed
flag and does nothing. [[src/target/shared/code/fs_helpers_io.rs:lower_fs_close_helper]]

Beyond the pre-close flush, `fs::close` reads and writes no file contents of its own.
It is an error to close a `File` that is already closed, including one closed by a
previous `fs::close` on the same value or by a prior scope-drop. It is likewise an
error to close a handle that `thread::transfer` has moved to another thread: such a
handle is not closed but no longer belongs to this thread, so the call reports that
distinctly. [[src/target/shared/code/error_constants.rs:RESOURCE_MOVED_BIT]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `file` | `File` | The open `File` resource to close, as returned by `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, `fs::openWithin`, or `fs::createTempFile`. Must not already be closed or moved. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing on success. After a successful return the descriptor is released and `file` is marked closed and must not be used again. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `file` has already been closed, whether by an earlier `fs::close` on the same value or by a prior scope-drop. [[src/target/shared/code/fs_helpers_io.rs:lower_fs_close_helper]] |
| `77030009` | `ErrResourceMoved` | `file` was moved to another thread by `thread::transfer` and no longer belongs to this thread, so it cannot be closed here. [[src/target/shared/code/fs_helpers_io.rs:lower_fs_close_helper]] |
| `77030006` | `ErrCloseFailed` | The host operating system reports a failure while releasing the descriptor. The handle is still marked closed and must not be reused. [[src/target/shared/code/fs_helpers_io.rs:lower_fs_close_helper]] |
| `77020002` | `ErrOutput` | The mandatory pre-close drain of the handle's buffered output fails — for example the disk is full or the descriptor is no longer writable. The descriptor is still released. [[src/target/shared/code/fs_helpers_io.rs:lower_fs_close_helper]] |

## Examples

Open a file and release its handle explicitly:

```
IMPORT fs

RES f = fs::openFile("data.txt")
LET line AS String = fs::readLine(f)
fs::close(f)
```

Write a file, then close it before reopening the same path:

```
IMPORT fs

RES w = fs::open("out.txt", "write")
fs::writeAll(w, "hello")
fs::close(w)
RES r = fs::open("out.txt", "read")
io::print(fs::readAll(r))
fs::close(r)
```

## See also

- `mfb man fs open`
- `mfb man fs openFile`
- `mfb man fs createTempFile`
- `mfb man fs flush`
- `mfb man fs setBuffered`
