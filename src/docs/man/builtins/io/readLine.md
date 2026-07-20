# readLine

Read one line of UTF-8 text from standard input, with no prompt

## Synopsis

```
io::readLine() AS String
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

`io::readLine` reads bytes from standard input up to and including the next line
feed (LF, byte `0x0A`) and returns the line as a `String` with its terminator
removed. If the byte immediately before the LF is a carriage return (CR, byte
`0x0D`) — a CRLF ending — that CR is stripped as well. A line that is empty before
its terminator returns an empty `String`, while still consuming the terminator. It
takes no arguments. [[src/target/shared/code/io_helpers.rs:lower_io_read_line_helper]]

**On a terminal, `io::readLine` suppresses echo for the duration of the read.**
It clears `ECHO` on standard input while leaving canonical (line) mode intact, so
the user still edits the line normally and submits it with Return, but the typed
characters are not displayed. The previous line discipline is restored before the
call returns. This is the difference from `io::input`, which leaves the terminal
untouched and therefore echoes; use `io::readLine` for passphrases and
`io::input` when the user should see what they type. When standard input is not a
terminal the stream is read as is with no mode change.
[[src/target/shared/code/io_helpers.rs:emit_configure_stdin_terminal]]

Before blocking, any pending standard-output buffer is drained, so output already
produced — including a prompt written with `io::write` — appears before the
program waits. [[src/target/shared/code/io_helpers.rs:lower_stdout_drain]]

Bytes are decoded as UTF-8 as they arrive, one scalar value at a time, with the
full validity check: lead bytes outside `C2`–`F4` are rejected, as are overlong
forms, surrogate encodings, and continuation bytes outside `80`–`BF`. An
ill-formed sequence fails rather than yielding a replacement character. The
accumulator grows as needed, so there is no fixed line-length limit beyond
available memory, and it is returned to the arena once the result `String` has
been built. [[src/target/shared/code/io_helpers.rs:lower_io_read_line_helper]]

End of input is reported as an error, not as an empty result — but only when it
arrives before any byte of the line. If input ends *after* some bytes were read,
those bytes are returned as the final, unterminated line and the following call
raises `ErrEof`. Test with `io::pollInput` when the program must not block.

Standard input is a per-thread broadcast log. A thread other than the main thread
must subscribe with `thread::openStdIn` before reading, or the call raises
`ErrInvalidContext`; the compiler subscribes the main thread automatically, so an
ordinary single-threaded program is unaffected.
[[src/target/shared/code/stdin_broadcast.rs:emit_stdin_next_byte]]

In a console program that also uses `term::`, the read is bracketed so it works
inside a TUI: the cooked line discipline `term::on` saved is restored for the
duration of the line read and single-key raw mode is re-applied afterwards, so a
`pollInput` + `readChar` loop resumes seeing bare keypresses.
[[src/target/shared/code/io_helpers.rs:emit_console_raw_line_mode]]

## Parameters

`io::readLine` takes no parameters. [[src/builtins/io.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `String` | The next line of input with its trailing LF, and a preceding CR if present, removed. An empty line returns an empty `String`; a final unterminated line returns the remaining bytes. [[src/builtins/io.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020003` | `ErrEof` | Standard input reaches end of input before any byte of the line is read. [[src/target/shared/code/error_constants.rs:ERR_EOF_CODE]] |
| `77020004` | `ErrEncoding` | The bytes read do not form a valid UTF-8 sequence — a bad lead byte, a bad continuation byte, an overlong form, a surrogate encoding, or a sequence truncated by end of input. [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
| `77020005` | `ErrInput` | Reading standard input fails for any other reason, or the terminal mode cannot be changed or restored. [[src/target/shared/code/error_constants.rs:ERR_INPUT_CODE]] |
| `77010001` | `ErrOutOfMemory` | The growing line accumulator or the returned `String` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050019` | `ErrInvalidContext` | The calling thread is not the main thread and has not subscribed to standard input with `thread::openStdIn`. [[src/target/shared/code/error_constants.rs:ERR_INVALID_CONTEXT_CODE]] |

## Examples

Read a line and echo it back:

```
IMPORT io

SUB main()
  LET line AS String = io::readLine()
  io::print(line)
END SUB
```

Prompt without echoing the answer:

```
IMPORT io

SUB main()
  io::write("Passphrase: ")
  LET secret AS String = io::readLine()
  io::print("")
END SUB
```

Read a line only when input is already pending:

```
IMPORT io

SUB main()
  IF io::pollInput() THEN
    io::print(io::readLine())
  END IF
END SUB
```

## See also

- `mfb man io input`
- `mfb man io readChar`
- `mfb man io readByte`
- `mfb man io pollInput`
