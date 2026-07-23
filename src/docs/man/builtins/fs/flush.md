# flush

Drain an open `File`'s output buffer to its file descriptor

## Synopsis

```
fs::flush(file AS File) AS Nothing
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

`fs::flush` drains any output currently held in `file`'s per-handle buffer,
issuing the pending bytes to the underlying file descriptor at once, then returns
nothing. It matters only when the handle has buffering enabled with
`fs::setBuffered(file, TRUE)`; on an unbuffered handle — the default — nothing is
ever held back, so `fs::flush` is a no-op. It is also a no-op when a buffered
handle has no pending bytes.
[[src/target/shared/code/fs/io.rs:lower_fs_flush_helper]]

Internally the drain issues a `write(fd, buffer, filled)` loop until the buffer is
empty and then resets the fill count to zero; a short write advances the cursor
and continues, and an `EINTR` interruption re-issues the write with the unchanged
cursor. If a write fails, the buffer is left intact so a later `fs::flush` can
retry, and the call raises `ErrOutput`.
[[src/target/shared/code/fs/io.rs:lower_fs_file_drain]]

Use `fs::flush` at a checkpoint where buffered data must reach the file before the
program continues — for example before another process reads the file, or before a
long pause. Closing the handle with `fs::close`, or letting its `RES` binding
leave scope, also drains the buffer, so an explicit flush is only needed
mid-stream; the final bytes are never lost to a clean close.
[[src/target/shared/code/fs/io.rs:lower_fs_close_helper]]

Buffering and flushing are per handle: `fs::flush(file)` drains only `file`'s
buffer and affects no other open `File`. Each `File` carries its own buffer and
its own enabled flag.
[[src/target/shared/code/error_constants.rs:FILE_OFFSET_BUF_ENABLED]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `file` | `File` | An open `File` resource whose output buffer should be drained. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of draining the handle's output buffer. [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020002` | `ErrOutput` | The underlying write of the buffered bytes fails — for example the disk is full or the descriptor is no longer writable. The buffer is left intact so the flush can be retried. [[src/target/shared/code/fs/io.rs:lower_fs_flush_helper]] |

## Examples

Force buffered data to disk at a checkpoint, then keep writing:

```
IMPORT fs

SUB main()
  LET header AS String = "id,name\n"
  LET body AS String = "1,alice\n"
  RES out = fs::openFile("report.txt", "write")
  fs::setBuffered(out, TRUE)
  fs::writeAll(out, header)
  fs::flush(out)             ' header reaches disk before the body is written
  fs::writeAll(out, body)
END SUB
```

## See also

- `mfb man fs setBuffered`
- `mfb man fs isBuffered`
- `mfb man fs writeAll`
- `mfb man fs close`
