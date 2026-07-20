# readByte

Read one raw byte from standard input

## Synopsis

```
io::readByte() AS Byte
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

`io::readByte` reads exactly one byte from standard input and returns it as a
`Byte` in the range 0 through 255. It takes no arguments and does not wait for a
newline. [[src/target/shared/code/io_helpers.rs:lower_io_read_byte_helper]]

**On a terminal the read is a single keypress.** For the duration of the call,
standard input is switched out of canonical mode and echo is suppressed
(`~ICANON`, `~ECHO`, `VMIN = 1`, `VTIME = 0`), so one key satisfies the read with
no Return and nothing is displayed; the previous line discipline is restored
before the call returns. When standard input is not a terminal the stream is read
as is with no mode change.
[[src/target/shared/code/io_helpers.rs:emit_configure_stdin_terminal]]

Before blocking, any pending standard-output buffer is drained, so a prompt
written with `io::write` appears before the program waits.
[[src/target/shared/code/io_helpers.rs:lower_stdout_drain]]

No decoding happens. The byte is transferred verbatim, so a multi-byte character
such as an emoji arrives one byte at a time across successive calls and there is
no `ErrEncoding` to raise — this is the difference from `io::readChar`, which
always returns one whole Unicode scalar value. Use `io::readByte` for binary
input or protocol framing, and `io::readChar` for text.

End of input is reported as an error, not as a sentinel value such as `0` or
`-1`, which keeps every one of the 256 byte values usable as data. Use
`io::pollInput` to test for readiness when the program must not block.

Standard input is a per-thread broadcast log. A thread other than the main thread
must subscribe with `thread::openStdIn` before reading, or the call raises
`ErrInvalidContext`; the main thread is subscribed automatically.
[[src/target/shared/code/stdin_broadcast.rs:emit_stdin_next_byte]]

## Parameters

`io::readByte` takes no parameters. [[src/builtins/io.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Byte` | The next byte of standard input, 0 through 255, uninterpreted. [[src/builtins/io.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020003` | `ErrEof` | Standard input reaches end of input before a byte is read. [[src/target/shared/code/error_constants.rs:ERR_EOF_CODE]] |
| `77020005` | `ErrInput` | Reading standard input fails for any other reason, or the terminal mode cannot be changed or restored. [[src/target/shared/code/error_constants.rs:ERR_INPUT_CODE]] |
| `77050019` | `ErrInvalidContext` | The calling thread is not the main thread and has not subscribed to standard input with `thread::openStdIn`. [[src/target/shared/code/error_constants.rs:ERR_INVALID_CONTEXT_CODE]] |

## Examples

Read one byte and report its value:

```
IMPORT io

SUB main()
  LET b AS Byte = io::readByte()
  io::print(toString(b))
END SUB
```

Wait for any keypress to continue:

```
IMPORT io

SUB main()
  io::write("Press any key to continue...")
  LET ignored AS Byte = io::readByte()
  io::print("")
END SUB
```

## See also

- `mfb man io readChar`
- `mfb man io readLine`
- `mfb man io pollInput`
