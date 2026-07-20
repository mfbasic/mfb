# terminalSize

Report the current size of the terminal surface as a `TermSize`

## Synopsis

```
term::terminalSize() AS TermSize
```

## Package

term

## Imports

```
IMPORT term
```

`term` is a built-in package, so no manifest dependency is required.
[[src/builtins/term.rs:is_term_call]]

## Description

`term::terminalSize` returns the size of the drawing surface as a freshly
allocated `TermSize` record with two `Integer` fields: `columns`, the width in
character cells, and `rows`, the height. Both are counts of whole cells, never
pixels. Valid cursor positions are rows `0` through `rows-1` and columns `0`
through `columns-1`. It takes no arguments.
[[src/builtins/term.rs:builtin_type_fields]]

**This is the one `term::` read that is not silently inert while TUI mode is
off.** There is no meaningful default size to report, so calling it before
`term::on` or after `term::off` raises `ErrUnsupported` rather than returning
something invented. Guard with `term::isOn` if the call site may run outside TUI
mode. [[src/target/shared/code/term.rs:emit_terminal_size]]

While TUI mode is on, the size is read live from the terminal with a `TIOCGWINSZ`
query on standard output, so it reflects the terminal as it is at the moment of
the call. If that query fails — standard output is not a terminal, or the host
does not answer — or if it reports zero rows or zero columns, the call raises
`ErrUnsupported`.

Because the query is live, the answer can change between calls when the user
resizes the window. A program that lays out, centres, or bounds-checks against
these dimensions should ask again rather than cache the first answer. The
drawing grid itself is reflowed to a new size by `term::sync`, which re-reads the
terminal on entry and, when the size changed, allocates a new grid preserving the
top-left overlap and forces a full repaint — so immediately after a resize and
before the next `term::sync`, this call can report the new size while the grid is
still the old one. [[src/target/shared/code/term_grid.rs:emit_grid_resize]]

Apart from the allocation, the call has no side effects: it draws nothing, moves
no cursor, and changes no `term::` state.

In app mode (`mfb build --app`) the size comes from the application's terminal
view rather than an ioctl, and the same `ErrUnsupported` is raised when TUI mode
is off or no view is attached.
[[src/target/macos_aarch64/app/app_io.rs:emit_app_terminal_size]]

## Parameters

`term::terminalSize` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `TermSize` | A record whose `columns` field is the surface width in cells and whose `rows` field is its height. Both are positive. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050007` | `ErrUnsupported` | TUI mode is off, or the terminal size cannot be obtained — the size query fails, or it reports zero rows or zero columns. [[src/target/shared/code/error_constants.rs:ERR_UNSUPPORTED_CODE]] |
| `77010001` | `ErrOutOfMemory` | The returned `TermSize` record cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Report the surface dimensions:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  LET size AS TermSize = term::terminalSize()
  term::off()
  io::print(toString(size.columns) & "x" & toString(size.rows))
END SUB
```

Draw near the centre of the surface:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  LET size AS TermSize = term::terminalSize()
  term::moveTo(size.rows / 2, size.columns / 2)
  io::write("middle")
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term moveTo`
- `mfb man term sync`
- `mfb man term isOn`
- `mfb man term on`
