# io

Standard stream input/output and terminal inspection

## Synopsis

```
IMPORT io
io::print(value)
io::write(value)
LET line AS String = io::input("Name: ")
io::printError(value)
IF io::pollInput(1000) THEN io::print(io::readLine())
```

## Description

The `io` package provides access to the three standard streams — standard
input, standard output, and standard error — together with helpers for reading
the keyboard and inspecting which streams are terminals. It is the console
counterpart to the `fs` package: where `fs` works through named files and `File`
handles, `io` works through the process standard streams. `io` is a built-in
package: `IMPORT io` needs no manifest dependency. [[src/builtins/io.rs:is_io_call]]

Output functions accept `String` values only and perform no implicit
conversion; convert other values with `toString` first. Text is treated as UTF-8
and emitted byte for byte, with no escaping or newline translation beyond the
trailing newline that `io::print` and `io::printError` add. `io::write` and
`io::print` target standard output, `io::writeError` and `io::printError` target
standard error, and `io::flush` drains standard output. Standard-output buffering
is opt-in and off by default: `io::setBuffered(TRUE)` holds output in a per-thread
buffer (drained on `io::flush`, before any read, when full, and at exit) and
`io::setBuffered(FALSE)` drains and disables it, while `io::isBuffered` reports the
current mode. With buffering on, written text is not guaranteed visible to an
external reader until flushed; flush before blocking on a read when a prompt must
appear first. Standard error is never buffered — it is written immediately, so it
has no flush. [[src/builtins/io.rs:expected_arguments]]

Input functions read from standard input. `io::input` reads a whole line with
normal terminal echo and an optional prompt; `io::readLine` reads a line the same
way but never writes a prompt. `io::readChar` returns one whole Unicode scalar
value as a `String` and `io::readByte` returns one raw `Byte`, both reading a
single unit without waiting for a newline and, on a terminal, with echo and
canonical line mode suppressed for the read before the prior mode is restored.
Character and line reads decode input as UTF-8 and reject ill-formed byte
sequences rather than substituting replacement characters; `io::readByte`
transfers bytes verbatim with no decoding. End of input is reported as an error,
not as an empty or sentinel result. `io::pollInput` tests whether input is ready
to read, optionally waiting up to a timeout in milliseconds, without consuming
any input. [[src/builtins/io.rs:call_return_type_name]]

The terminal predicates `io::isInputTerminal`, `io::isOutputTerminal`, and
`io::isErrorTerminal` return a `Boolean` reporting whether the corresponding
standard stream is connected to an interactive terminal; they never block,
consume input, or raise. Output is directed to whichever destination is bound to
each standard stream: in a normal console program these are file descriptors 0,
1, and 2; in app mode the same calls are routed to the application transcript
window, which is treated as an interactive terminal.

Standard input is a per-thread broadcast. The runtime owns file descriptor 0 and
reads it in chunks into one process-global append-only log; each *subscribed*
thread reads its own cursor over that log, so the syscall count collapses from one
`read` per byte to roughly one per several kilobytes, every subscriber sees the
whole stdin stream from its subscription point, and a byte read by one thread is
never consumed out from under another. A single-threaded program is byte-identical
to a direct per-byte reader: the compiler subscribes the main thread at program
entry, so the same bytes, the same end-of-input position, and the same
`io::pollInput` results are observed with no source change. A thread other than
main must subscribe with `thread::openStdIn` before reading, or the read raises
`ErrInvalidContext`; `thread::closeStdIn` unsubscribes.

Thread cancellation is cooperative: the runtime does not asynchronously interrupt
a standard-input read, so a worker that needs prompt cancellation should poll
with `io::pollInput` and check `thread::isCancelled` between waits.

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020002` | `ErrOutput` | raised by `io::print`, `io::write`, `io::printError`, `io::writeError`, `io::flush`, and `io::input` (while writing or flushing a prompt) when the underlying write or flush to a standard stream fails [[src/target/shared/code/error_constants.rs:ERR_OUTPUT_CODE]] |
| `77020003` | `ErrEof` | raised by `io::input`, `io::readLine`, `io::readChar`, and `io::readByte` when standard input reaches end of file before any byte of the requested unit is read [[src/target/shared/code/error_constants.rs:ERR_EOF_CODE]] |
| `77020004` | `ErrEncoding` | raised by `io::input`, `io::readLine`, and `io::readChar` when the bytes read do not form a valid UTF-8 sequence [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
| `77020005` | `ErrInput` | raised by `io::input`, `io::readLine`, `io::readChar`, `io::readByte`, and `io::pollInput` when reading or polling standard input fails for any other reason [[src/target/shared/code/error_constants.rs:ERR_INPUT_CODE]] |
| `77010001` | `ErrOutOfMemory` | raised by `io::input`, `io::readLine`, and `io::readChar` when the line buffer or returned `String` cannot be allocated [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050019` | `ErrInvalidContext` | raised by `io::input`, `io::readLine`, `io::readChar`, and `io::readByte` when the calling thread is not the main thread and has not subscribed to standard input with `thread::openStdIn` [[src/target/shared/code/error_constants.rs:ERR_INVALID_CONTEXT_CODE]] |
