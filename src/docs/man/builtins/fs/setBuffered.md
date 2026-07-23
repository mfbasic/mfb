# setBuffered

Enable or disable opt-in output buffering for an open `File`

## Synopsis

```
fs::setBuffered(file AS File, enabled AS Boolean) AS Nothing
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

`fs::setBuffered` turns per-handle output buffering on or off for a single open
`File`, then returns nothing. Buffering is a per-handle flag stored on the `File`
resource itself, so the call affects only `file` and no other open handle; each
`File` carries its own buffer and its own enabled flag.
[[src/target/shared/code/fs/io.rs:lower_fs_set_buffered_helper]]

Buffering is **off by default**: a freshly opened `File` starts with its buffered
flag clear, so every incremental `fs::writeAll` and `fs::writeAllBytes` reaches
the operating system immediately. Calling `fs::setBuffered(file, TRUE)` sets the
flag; from then on incremental writes to `file` are held in a per-handle buffer
and issued in larger blocks, collapsing a loop of small writes into roughly one
host write per full buffer.
[[src/target/shared/code/error_constants.rs:FILE_OFFSET_BUF_ENABLED]]

When buffering is on, held output is drained automatically when the buffer fills,
on an explicit `fs::flush(file)`, and when the handle is closed — whether by
`fs::close` or by lexical scope exit of its `RES` binding. Calling
`fs::setBuffered(file, FALSE)` drains any pending bytes first, on a best-effort
basis, and then clears the flag, so switching buffering off never strands data in
the buffer.
[[src/target/shared/code/fs/io.rs:lower_fs_set_buffered_helper]]

Only incremental `fs::writeAll` / `fs::writeAllBytes` writes are buffered. The
whole-file operations (`fs::writeText`, `fs::writeBytes`, and the append and
atomic variants) already issue their output in a single write and are unaffected
by this setting.

Because buffered output is held in memory until it is drained, a hard crash
(`SIGSEGV`, `SIGKILL`, or an abort) can lose bytes that were written but not yet
flushed. Flush or close a buffered handle to make its data durable, and leave
buffering off (the default) when partial-output-on-crash durability matters. A
buffered handle should also be flushed before it is transferred to another
thread, which resets it to unbuffered.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `file` | `File` | An open `File` resource whose buffering mode is being changed. [[src/builtins/fs.rs:call_param_names]] |
| `enabled` | `Boolean` | `TRUE` to enable output buffering for this handle; `FALSE` to drain any pending output and disable it. [[src/builtins/fs.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of changing the handle's buffering flag (and, when disabling, draining any pending output). [[src/builtins/fs.rs:call_return_type_name]] |

## Errors

No errors. `fs::setBuffered` always returns success; it never raises. When
disabling buffering, the drain of any pending bytes is best-effort and its result
is discarded here — a write failure at that point is not reported by this call but
surfaces from the next `fs::flush`, buffered write, or close of the handle.
[[src/target/shared/code/fs/io.rs:lower_fs_set_buffered_helper]]

## Examples

Buffer a loop of small writes and let scope exit flush and close the handle:

```
IMPORT fs

SUB main()
  LET events AS List OF String = ["started", "ready"]
  RES log = fs::openFile("events.log", "write")
  fs::setBuffered(log, TRUE)
  FOR EACH event IN events
    fs::writeAll(log, event & "\n")
  NEXT
  ' log is flushed and closed automatically at scope exit
END SUB
```

Enable buffering for a bulk write, then flush and disable it before durable work:

```
IMPORT fs

SUB main()
  LET header AS String = "id,name\n"
  LET body AS String = "1,alice\n"
  RES out = fs::openFile("report.txt", "write")
  fs::setBuffered(out, TRUE)
  fs::writeAll(out, header)
  fs::writeAll(out, body)
  fs::setBuffered(out, FALSE)   ' drains the pending header and body, then disables
END SUB
```

## See also

- `mfb man fs isBuffered`
- `mfb man fs flush`
- `mfb man fs writeAll`
- `mfb man fs close`
