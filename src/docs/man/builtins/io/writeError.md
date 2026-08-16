# writeError

Write a `String` to standard error with no trailing newline

## Synopsis

```
io::writeError(value AS String) AS Nothing
```

## Package

io

## Imports

```
IMPORT io
```

`io` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/io/mod.rs:register]]

## Description

`io::writeError` writes `value` to standard error exactly as stored and adds
nothing. The text is treated as UTF-8 and emitted byte for byte, with no escaping
and no newline translation. An empty `String` writes nothing at all. It is the
newline-free counterpart of `io::printError`.
[[src/codegen/builtins/io/native/stdout.rs:lower_io_write_helper]]

Only `String` is accepted, and exactly one argument; there is no implicit
conversion, so convert other values first — for example with `toString`.
[[src/codegen/builtins/io/mod.rs:register]] [[src/codegen/builtins/io/mod.rs:register]]

Standard error is **never buffered**. `io::setBuffered` controls standard output
only, so this call always issues its bytes immediately and there is no flush for
standard error. It is also never retained by `term::` TUI mode — the shadow-grid
routing covers standard output alone.
[[src/codegen/builtins/io/native/stdout.rs:lower_io_write_helper]]

The underlying write loops until every byte has been transferred: a short write
advances the cursor and re-issues, and an `EINTR` interruption retries with the
cursor unchanged. A zero-byte or failing write raises `ErrOutput`.

Output goes to whatever is bound to standard error: file descriptor 2 in a
console program, and the application transcript in app mode (`mfb build --app`).
[[src/target/shared/code/mod.rs:lower_runtime_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The text to write. Interpreted as UTF-8 and emitted unchanged; may be empty. [[src/codegen/builtins/io/mod.rs:register]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of writing to standard error. [[src/codegen/builtins/io/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020002` | `ErrOutput` | The write to standard error fails. [[src/builtins/errorcode.rs:ErrWriteFailed]] |

## Examples

Emit a progress marker on the error stream without breaking the line:

```
IMPORT io

SUB main()
  io::writeError("working")
  io::writeError(".")
  io::printError(" done")
END SUB
```

## See also

- `mfb man io printError`
- `mfb man io write`
- `mfb man io isErrorTerminal`
