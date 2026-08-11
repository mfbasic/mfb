# process

Spawn and manage child processes: run a program, stream its standard I/O,
wait for it, and signal or detach it.

## Synopsis

```
IMPORT process
RES child = process::spawn(["echo", "hello"])
LET line = process::receive(child)
LET code = process::waitFor(child)
RES sh = process::shell("ls | wc -l")
process::signal(sh, Signal.Terminate)
```

## Description

The `process` package runs and controls child processes. Its one resource,
`Process`, is an opaque, owned, non-copyable handle to a spawned child — a native
resource sharing the runtime's canonical resource record (resource tag `10`).
Like every resource handle it cannot be copied, stored as a collection element,
or carried in a record; it is closed automatically by lexical drop when its
binding leaves scope. [[src/codegen/builtins/process/mod.rs:PROCESS_TYPE]]
[[src/target/shared/code/error_constants.rs:RESOURCE_TAG_PROCESS]]

A child is created two ways. `process::spawn` runs a program directly from an
argument list — `args[0]` is the executable, resolved on `PATH`, and no shell is
involved, so no quoting, globbing, or redirection is interpreted. `process::shell`
instead runs a command line through the platform shell (`/bin/sh -c` on Unix), so
pipes, redirection, and shell syntax work. A four-argument `spawn` overload adds a
working directory, an environment `Map OF String TO String`, and a replace-vs-merge
flag. [[src/codegen/builtins/process/native/unix.rs:lower_process_spawn_helper]]
[[src/codegen/builtins/process/native/unix.rs:lower_process_shell_helper]]

Ownership of a live child is deliberate. Letting a `Process` drop at scope exit
**force-kills and reaps** it (`SIGKILL` + `waitpid` on Unix), so no runaway child
or zombie is left behind and the drop never blocks. `process::close` is *not* a
handle-consuming close: it closes only the child's standard input (signalling
end-of-input to a filter) and leaves the child running and the handle usable.
`process::detach` relinquishes ownership the other way — it closes the parent-side
pipes, arranges for the child to be auto-reaped, and marks the handle closed so
the child keeps running independently after the program exits.
[[src/codegen/builtins/process/mod.rs:resource_close_function]]
[[src/codegen/builtins/process/native/unix.rs:lower_process_detach_helper]]

Streaming I/O connects to the child's three standard streams over pipes.
`process::send` writes a `String` (appending a newline) to the child's standard
input; `process::sendBytes` writes raw bytes with no newline. `process::receive`
reads one newline-terminated line as a `String`; `process::receiveBytes` reads one
available chunk of raw bytes. Both readers take an optional `Stream` argument
selecting standard output (the default) or standard error, and `process::poll`
reports whether the selected stream is readable within a timeout. A read that
reaches end of stream with nothing buffered raises `ErrResourceClosed`, so a
consumer loops until that error is raised. [[src/codegen/builtins/process/mod.rs:STREAM_TYPE]]
[[src/codegen/builtins/process/native/unix.rs:lower_process_receive_helper]]

The `Signal` enum is a four-bucket cross-platform vocabulary (`None`, `Kill`,
`Terminate`, `Error`) used both to *deliver* a signal with `process::signal` and
to *observe* how a terminated child died with `process::didSignal`; the exact
platform mapping is tabulated in `mfb man process types`.
[[src/codegen/builtins/process/mod.rs:SIGNAL_TYPE]]
[[src/codegen/builtins/process/native/unix.rs:lower_process_signal_helper]]

The lifecycle queries read cached state: `process::pid` returns the child pid,
`process::isRunning` polls without blocking, `process::waitFor` blocks for exit
and returns the exit code (`-1` on a signal death on Unix). `waitFor` and
`isRunning` cache the exit status the first time they observe it, so `waitFor` is
idempotent and `didSignal` can report the death cause after the fact.
[[src/codegen/builtins/process/native/unix.rs:lower_process_waitfor_helper]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | raised by `spawn`, `shell`, `receive`, and `receiveBytes` when an internal allocation fails — the `argv`/environment C strings, the read buffer, or the returned `String`/`List OF Byte` [[src/builtins/errorcode.rs:ErrOutOfMemory]] |
| `77020004` | `ErrEncoding` | raised by `receive` when the line read from the child is not valid UTF-8 [[src/builtins/errorcode.rs:ErrEncoding]] |
| `77030004` | `ErrResourceClosed` | raised by any function taking a `Process` when the handle has already been dropped or detached; and by the I/O functions at end of stream — a `receive`/`receiveBytes` past end of output, or a `send`/`sendBytes` to a child whose input pipe is gone [[src/builtins/errorcode.rs:ErrResourceClosed]] |
| `77050002` | `ErrInvalidArgument` | raised by `spawn` when the `args` list is empty (there is no program to run) [[src/builtins/errorcode.rs:ErrInvalidArgument]] |
| `77050008` | `ErrTimeout` | raised by the `send`/`sendBytes` timeout overload when the child's input pipe stays full past the deadline [[src/builtins/errorcode.rs:ErrTimeout]] |
| `77080001` | `ErrSpawnFailed` | raised by `spawn` and `shell` when the child cannot be created — `fork`/`pipe` failed, or the program was not found or could not be `exec`'d [[src/builtins/errorcode.rs:ErrSpawnFailed]] |
