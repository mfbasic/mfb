# printError

Write a `String` to standard error followed by a newline

## Synopsis

```
io::printError(value AS String) AS Nothing
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

`io::printError` writes `value` to standard error and then appends a single line
feed (LF, byte `0x0A`). The text is treated as UTF-8 and emitted byte for byte,
with no escaping and no newline translation beyond the one trailing newline this
call adds. An empty `String` emits nothing but that newline.
[[src/target/shared/code/io_stdout.rs:lower_io_write_helper]]

Only `String` is accepted, and exactly one argument; there is no implicit
conversion, so convert other values first — for example with `toString`.
[[src/builtins/io.rs:arity]] [[src/builtins/io.rs:resolve_call]]

Standard error is **never buffered**. `io::setBuffered` controls standard output
only, so error output is always issued immediately and can never sit unseen in a
buffer; there is correspondingly no flush for standard error. It is also never
retained by `term::` TUI mode — the shadow-grid routing applies to standard
output alone — so an error message written while a TUI frame is being composed
goes straight to the terminal rather than into the frame.
[[src/target/shared/code/io_stdout.rs:lower_io_write_helper]]

The underlying write loops until every byte has been transferred: a short write
advances the cursor and re-issues, and an `EINTR` interruption retries with the
cursor unchanged. A zero-byte or failing write raises `ErrOutput`.

Output goes to whatever is bound to standard error: file descriptor 2 in a
console program, and the application transcript in app mode (`mfb build --app`).
[[src/target/shared/code/mod.rs:lower_runtime_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The text to write. Interpreted as UTF-8 and emitted unchanged; may be empty. [[src/builtins/io.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of writing to standard error. [[src/builtins/io.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020002` | `ErrOutput` | The write of the text or of the trailing newline to standard error fails. [[src/target/shared/code/error_constants.rs:ERR_OUTPUT_CODE]] |

## Examples

Report a failure on the error stream:

```
IMPORT io

SUB main()
  io::printError("cannot open the input file")
END SUB
```

Colour the message only when standard error is a terminal:

```
IMPORT io

SUB main()
  IF io::isErrorTerminal() THEN
    io::printError("\u{1b}[31mError\u{1b}[0m: something went wrong")
  ELSE
    io::printError("Error: something went wrong")
  END IF
END SUB
```

## See also

- `mfb man io writeError`
- `mfb man io print`
- `mfb man io isErrorTerminal`
