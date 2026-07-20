# readChar

Read one whole Unicode scalar value from standard input

## Synopsis

```
io::readChar() AS String
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

`io::readChar` reads exactly one Unicode scalar value from standard input and
returns it as a one-character `String`. It reads the lead byte, derives the
sequence length from it, and reads the one to three continuation bytes that
complete the scalar. It takes no arguments and does not wait for a newline.
[[src/target/shared/code/io_helpers.rs:lower_io_read_char_helper]]

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

Decoding is strict UTF-8, not lenient: a lead byte below `C2` (other than plain
ASCII) or above `F4` is rejected, `E0`/`F0` sequences must not be overlong, `ED`
sequences may not encode a surrogate, `F4` sequences may not exceed U+10FFFF, and
every continuation byte must lie in `80`–`BF`. An ill-formed sequence raises
`ErrEncoding` rather than yielding a replacement character, and so does a
sequence cut short by end of input.
[[src/target/shared/code/io_helpers.rs:lower_io_read_char_helper]]

Note that this returns one *scalar value*, not one user-perceived character: a
grapheme cluster made of several scalars (an emoji with a modifier, a base letter
plus a combining mark) takes that many calls. Compare `io::readByte`, which
returns raw bytes with no decoding at all.

End of input is reported as an error, not as an empty result. Use `io::pollInput`
to test for readiness when the program must not block.

Standard input is a per-thread broadcast log. A thread other than the main thread
must subscribe with `thread::openStdIn` before reading, or the call raises
`ErrInvalidContext`; the main thread is subscribed automatically.
[[src/target/shared/code/stdin_broadcast.rs:emit_stdin_next_byte]]

## Parameters

`io::readChar` takes no parameters. [[src/builtins/io.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `String` | A one-scalar `String` holding the character read — one to four UTF-8 bytes. [[src/builtins/io.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020003` | `ErrEof` | Standard input reaches end of input before the lead byte is read. [[src/target/shared/code/error_constants.rs:ERR_EOF_CODE]] |
| `77020004` | `ErrEncoding` | The bytes read are not a valid UTF-8 scalar — a bad lead or continuation byte, an overlong form, a surrogate encoding, a value above U+10FFFF, or a sequence truncated by end of input. [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
| `77020005` | `ErrInput` | Reading standard input fails for any other reason, or the terminal mode cannot be changed or restored. [[src/target/shared/code/error_constants.rs:ERR_INPUT_CODE]] |
| `77010001` | `ErrOutOfMemory` | The returned `String` cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050019` | `ErrInvalidContext` | The calling thread is not the main thread and has not subscribed to standard input with `thread::openStdIn`. [[src/target/shared/code/error_constants.rs:ERR_INVALID_CONTEXT_CODE]] |

## Examples

Wait for any keypress to continue:

```
IMPORT io

SUB main()
  io::write("Press any key to continue...")
  LET ignored AS String = io::readChar()
  io::print("")
END SUB
```

Poll for a key inside a TUI frame loop:

```
IMPORT io
IMPORT term

SUB main()
  term::on()
  term::clear()
  io::write("q to quit")
  term::sync()
  IF io::pollInput(1000) THEN
    LET key AS String = io::readChar()
    IF key = "q" THEN
      term::off()
    END IF
  END IF
END SUB
```

## See also

- `mfb man io readByte`
- `mfb man io readLine`
- `mfb man io pollInput`
- `mfb man term on`
