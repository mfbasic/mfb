# print

Write a `String` to standard output followed by a newline

## Synopsis

```
io::print(value AS String) AS Nothing
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

`io::print` writes `value` to standard output and then appends a single line feed
(LF, byte `0x0A`). The text is treated as UTF-8 and emitted byte for byte, with no
escaping and no newline translation beyond the one trailing newline this call
adds. An empty `String` emits nothing but that newline.
[[src/target/shared/code/io_stdout.rs:lower_io_write_helper]]

Only `String` is accepted, and exactly one argument. There is no implicit
conversion, so a non-string value must be converted first — for example with
`toString`. [[src/builtins/io.rs:arity]] [[src/builtins/io.rs:resolve_call]]

The underlying write loops until every byte has been transferred: a short write
advances the cursor and re-issues, and an `EINTR` interruption retries with the
cursor unchanged. A zero-byte or failing write is a failure and raises
`ErrOutput`. [[src/target/shared/code/io_stdout.rs:lower_io_write_helper]]

With standard-output buffering enabled by `io::setBuffered(TRUE)` the text is
appended to a per-thread 4 KiB buffer instead of being written immediately; it is
drained when the buffer fills, on `io::flush`, before any standard-input read, and
at program exit. A chunk larger than the whole buffer is written directly after
draining, so ordering is always preserved. Buffering is off by default, in which
case each call writes straight through.
[[src/target/shared/code/io_stdout.rs:lower_stdout_drain]]

While the program is in `term::` TUI mode, standard output is **retained rather
than printed**: `io::print` stamps its glyphs into the shadow grid's back buffer,
honouring `\n`, `\r`, right-edge wrap, and bottom-of-screen scroll, and the
trailing newline advances the shadow cursor. Nothing reaches the terminal until
`term::sync` presents the frame. This routing applies to standard output only —
standard error is never retained — and only in a program that uses `term::`.
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
| `77020002` | `ErrOutput` | The write of the text or of the trailing newline fails, or a buffered write's drain fails — for example the descriptor is closed or the disk is full. [[src/target/shared/code/error_constants.rs:ERR_OUTPUT_CODE]] |

## Examples

Print a line of text:

```
IMPORT io

SUB main()
  io::print("Hello")
END SUB
```

Convert a non-string value before printing:

```
IMPORT io

SUB main()
  io::print(toString(42))
  io::print("total: " & toString(42))
END SUB
```

## See also

- `mfb man io write`
- `mfb man io printError`
- `mfb man io flush`
- `mfb man io setBuffered`
