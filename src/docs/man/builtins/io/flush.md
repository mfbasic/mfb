# flush

Drain the per-thread standard-output buffer

## Synopsis

```
io::flush() AS Nothing
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

`io::flush` writes out any bytes currently held in this thread's MFBASIC
standard-output buffer and returns nothing. It takes no arguments.
[[src/builtins/io.rs:arity]]

The call is **drain-only**. It issues the pending bytes with a `write` loop and
reports whether that write succeeded; it deliberately does *not* `fsync` or
otherwise ask the host to sync standard output. A sync's result depends on the
kind of descriptor standard output happens to be — `EBADF` only for a genuinely
closed descriptor, a benign `EINVAL` on pipes and character devices, success on a
regular file — which would make `io::flush` succeed or fail based on the runtime
environment rather than on what the program actually wrote. The buffer drain's
`write` is the one portable failure signal, identical on every platform and libc.
[[src/target/shared/code/io_helpers.rs:lower_io_flush_helper]]

It follows that `io::flush` is a **no-op when buffering is off** — the default.
Without `io::setBuffered(TRUE)` there is no MFBASIC buffer to drain, every
`io::write` and `io::print` has already reached the operating system, and this
call succeeds having done nothing. It is likewise a no-op when buffering is on
but nothing is pending. [[src/target/shared/code/io_helpers.rs:lower_stdout_drain]]

The drain loops until the buffer is empty: a short write advances the cursor and
re-issues, and an `EINTR` interruption retries. If a write genuinely fails, the
still-unflushed bytes are slid back to the base of the buffer and kept, so a later
`io::flush` resumes from exactly where this one stopped instead of re-sending the
prefix already written — and this call raises `ErrOutput`.
[[src/target/shared/code/io_helpers.rs:lower_stdout_drain]]

An explicit flush is rarely required even under buffering: the buffer is also
drained when it fills, before every standard-input read (so a buffered prompt
appears before the program blocks), on `io::setBuffered(FALSE)`, and at program
exit. Reach for `io::flush` at a checkpoint where output must be visible to an
external reader before the program continues.

Standard error is never buffered and is written immediately, so it has no
corresponding flush. In app mode transcript writes are synchronous, so this call
succeeds immediately. [[src/target/shared/code/mod.rs:lower_runtime_helper]]

## Parameters

`io::flush` takes no parameters. [[src/builtins/io.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of draining the standard-output buffer. [[src/builtins/io.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020002` | `ErrOutput` | The write of the buffered bytes fails. The unflushed remainder is retained so the flush can be retried. [[src/target/shared/code/error_constants.rs:ERR_OUTPUT_CODE]] |

## Examples

Make buffered output visible at a checkpoint:

```
IMPORT io

SUB longRunningWork()
END SUB

SUB main()
  io::setBuffered(TRUE)
  io::print("phase one complete")
  io::flush()                ' the line reaches the terminal before the long work
  longRunningWork()
END SUB
```

## See also

- `mfb man io setBuffered`
- `mfb man io isBuffered`
- `mfb man io write`
- `mfb man io print`
