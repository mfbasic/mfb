# write

Write a `String` to standard output with no trailing newline

## Synopsis

```
io::write(value AS String) AS Nothing
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

`io::write` writes `value` to standard output exactly as stored and adds nothing.
The text is treated as UTF-8 and emitted byte for byte, with no escaping and no
newline translation. An empty `String` writes nothing at all. It is the
newline-free counterpart of `io::print`, which is the same call with a trailing
LF appended. [[src/target/shared/code/io_helpers.rs:lower_io_write_helper]]

Only `String` is accepted, and exactly one argument; there is no implicit
conversion, so convert other values first — for example with `toString`.
[[src/builtins/io.rs:arity]] [[src/builtins/io.rs:resolve_call]]

The underlying write loops until every byte has been transferred: a short write
advances the cursor and re-issues, and an `EINTR` interruption retries with the
cursor unchanged. A zero-byte or failing write is a failure and raises
`ErrOutput`. [[src/target/shared/code/io_helpers.rs:lower_io_write_helper]]

With standard-output buffering enabled by `io::setBuffered(TRUE)` the text is
appended to a per-thread 4 KiB buffer rather than written immediately, so it may
not be visible to an external reader until drained. The buffer is drained when it
fills, on `io::flush`, before any standard-input read, and at program exit —
which is why a prompt written with `io::write` still appears before a following
`io::readLine` even under buffering.
[[src/target/shared/code/io_helpers.rs:lower_stdout_drain]]

While the program is in `term::` TUI mode, standard output is retained rather
than printed: `io::write` stamps its glyphs into the shadow grid's back buffer,
honouring `\n`, `\r`, right-edge wrap, and bottom-of-screen scroll, and nothing
reaches the terminal until `term::sync` presents the frame.
[[src/target/shared/code/term_grid.rs:emit_grid_write]]

Output goes to whatever is bound to standard output: file descriptor 1 in a
console program, and the application transcript window in app mode
(`mfb build --app`). [[src/target/shared/code/mod.rs:lower_runtime_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The text to write. Interpreted as UTF-8 and emitted unchanged; may be empty. [[src/builtins/io.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of writing to standard output. [[src/builtins/io.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020002` | `ErrOutput` | The write fails, or a buffered write's drain fails — for example the descriptor is closed or the disk is full. [[src/target/shared/code/error_constants.rs:ERR_OUTPUT_CODE]] |

## Examples

Write a prompt on the same line as the answer:

```
IMPORT io

SUB main()
  io::write("Name: ")
  LET name AS String = io::readLine()
END SUB
```

Build a line from several pieces:

```
IMPORT io

SUB main()
  io::write("x=")
  io::write(toString(3))
  io::print("")
END SUB
```

## See also

- `mfb man io print`
- `mfb man io writeError`
- `mfb man io flush`
- `mfb man io setBuffered`
