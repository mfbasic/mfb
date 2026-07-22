# setBuffered

Enable or disable opt-in standard-output buffering for this thread

## Synopsis

```
io::setBuffered(enabled AS Boolean) AS Nothing
```

## Package

io

## Imports

```
IMPORT io
```

`io` is a built-in package, so no manifest dependency is required.
[[src/builtins/io.rs:is_io_call]]

## Description

`io::setBuffered` turns standard-output buffering on or off for the calling
thread and returns nothing. Buffering is **off by default**, so without this call
every `io::write` and `io::print` reaches the operating system immediately.
[[src/target/shared/code/io_helpers.rs:lower_io_set_buffered_helper]]

Passing `TRUE` only sets the enabled flag; the 4 KiB buffer itself is allocated
lazily on the first buffered write. From then on output is accumulated and issued
in blocks, collapsing a write-heavy loop from one host write per call to roughly
one per full buffer. A chunk larger than the whole buffer is written directly
after the buffer is drained, so ordering is never disturbed, and if the buffer
cannot be allocated the write falls back to going out directly — buffering is an
optimization, never a correctness dependency.
[[src/target/shared/code/io_helpers.rs:lower_stdout_drain]]

Passing `FALSE` **drains any pending bytes first** and then clears the flag, so
switching buffering off never strands output. That drain is best-effort: this call
returns `Nothing` and does not report a write failure, which instead surfaces from
the next `io::flush` or buffered write.
[[src/target/shared/code/io_helpers.rs:lower_io_set_buffered_helper]]

While buffering is on, held output is also drained when the buffer fills, on
`io::flush`, before any standard-input read — so a buffered prompt always appears
before the program blocks — and at program exit.

The setting is per thread: each thread has its own buffer and its own enabled
flag, and one thread's choice is invisible to another. Standard error is never
buffered, so this call affects standard output only. In app mode the buffer is
inert and this call does nothing.
[[src/target/shared/code/io_helpers.rs:lower_io_set_buffered_helper]]

Because buffered output lives in memory until drained, a hard crash (`SIGSEGV`,
`SIGKILL`, an abort) can lose bytes that were written but not yet flushed. Leave
buffering off when partial-output-on-crash visibility matters.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `enabled` | `Boolean` | `TRUE` to enable standard-output buffering for this thread; `FALSE` to drain any pending output and disable it. [[src/builtins/io.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of changing the buffering flag (and, when disabling, draining pending output). [[src/builtins/io.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Buffer a write-heavy loop and flush once at the end:

```
IMPORT io

SUB main()
  io::setBuffered(TRUE)
  MUT i AS Integer = 0
  WHILE i < 100000
    io::print(toString(i))
    i = i + 1
  END WHILE
  io::flush()
END SUB
```

Restore the previous mode after a bulk section:

```
IMPORT io

SUB emitReport()
END SUB

SUB main()
  LET wasBuffered AS Boolean = io::isBuffered()
  io::setBuffered(TRUE)
  emitReport()
  io::setBuffered(wasBuffered)   ' drains the report when switching back off
END SUB
```

## See also

- `mfb man io isBuffered`
- `mfb man io flush`
- `mfb man io print`
- `mfb man io write`
