# input

Read one line of UTF-8 text from standard input, optionally writing a prompt first

## Synopsis

```
io::input() AS String
io::input(prompt AS String) AS String
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

`io::input` optionally writes a prompt to standard output, then reads bytes from
standard input up to and including the next line feed (LF, byte `0x0A`) and
returns the line as a `String` with its terminator removed. A preceding carriage
return (CR, byte `0x0D`, from a CRLF ending) is stripped as well. A line that is
empty before its terminator returns an empty `String`.
[[src/target/shared/code/io_helpers.rs:lower_io_read_line_helper]]

**`io::input` does not change the terminal mode**, so typed characters are echoed
by the terminal in the usual way and the line is submitted with Return. This is
the difference from `io::readLine`, which suppresses echo for the read; reach for
`io::input` when the user should see what they type.
[[src/target/shared/code/io_helpers.rs:emit_configure_stdin_terminal]]

The prompt is written verbatim — no trailing space or newline is added — and it
is written **directly**, bypassing the standard-output buffer, so it is on screen
before the program blocks. Any bytes already sitting in that buffer are drained
first, keeping the prompt in order with earlier output. An empty prompt writes
nothing at all and therefore cannot fail; `io::input()` with no argument is
exactly `io::input("")`.
[[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]]

Like the flush, the prompt write is just a `write` loop — short writes advance the
cursor and re-issue, `EINTR` retries — and a genuine failure raises `ErrOutput`
before any input is read. There is no `fsync`.
[[src/target/shared/code/io_helpers.rs:lower_io_read_line_helper]]

Bytes are decoded as UTF-8 as they arrive, with the full validity check: lead
bytes outside `C2`–`F4` are rejected, as are overlong forms, surrogate encodings,
and continuation bytes outside `80`–`BF`. An ill-formed sequence fails rather
than yielding a replacement character. The accumulator grows as needed, so there
is no fixed line-length limit beyond available memory.

End of input is an error rather than an empty result, but only when it arrives
before any byte of the line: if input ends after some bytes were read, those bytes
are returned as the final unterminated line and the following call raises
`ErrEof`.

Standard input is a per-thread broadcast log. A thread other than the main thread
must subscribe with `thread::openStdIn` before reading, or the call raises
`ErrInvalidContext`; the main thread is subscribed automatically.
[[src/target/shared/code/stdin_broadcast.rs:emit_stdin_next_byte]]

In app mode (`mfb build --app`) the prompt goes to the application transcript and
the line is read from the window input pipe.
[[src/target/shared/code/mod.rs:lower_runtime_helper]]

## Overloads

**`io::input() AS String`**

Reads a line with no prompt. Equivalent to `io::input("")`, and identical to
`io::readLine()` except that `io::readLine` suppresses terminal echo while
`io::input` does not. [[src/builtins/io.rs:arity]]

**`io::input(prompt AS String) AS String`**

Writes `prompt` to standard output directly, then reads the line. A failure while
writing the prompt is reported before any input is consumed.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `prompt` | `String` | Text written to standard output before the read, verbatim and with nothing appended. An empty `String` writes nothing. Omitting the argument is the same as passing `""`. [[src/builtins/io.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The line read, with its trailing LF and a preceding CR if present removed. An empty line returns an empty `String`; a final unterminated line returns the remaining bytes. [[src/builtins/io.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020002` | `ErrOutput` | Writing the prompt to standard output fails. Only possible for a non-empty prompt. [[src/target/shared/code/error_constants.rs:ERR_OUTPUT_CODE]] |
| `77020003` | `ErrEof` | Standard input reaches end of input before any byte of the line is read. [[src/target/shared/code/error_constants.rs:ERR_EOF_CODE]] |
| `77020004` | `ErrEncoding` | The bytes read do not form a valid UTF-8 sequence. [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
| `77020005` | `ErrInput` | Reading standard input fails for any other reason. [[src/target/shared/code/error_constants.rs:ERR_INPUT_CODE]] |
| `77010001` | `ErrOutOfMemory` | The growing line accumulator or the returned `String` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050019` | `ErrInvalidContext` | The calling thread is not the main thread and has not subscribed to standard input with `thread::openStdIn`. [[src/target/shared/code/error_constants.rs:ERR_INVALID_CONTEXT_CODE]] |

## Examples

Prompt on the same line and greet the user:

```
IMPORT io

SUB main()
  LET name AS String = io::input("Name: ")
  io::print("Hello, " & name)
END SUB
```

Read a line without a prompt:

```
IMPORT io

SUB main()
  LET line AS String = io::input()
  io::print(line)
END SUB
```

## See also

- `mfb man io readLine`
- `mfb man io pollInput`
- `mfb man io write`
- `mfb man io isInputTerminal`
