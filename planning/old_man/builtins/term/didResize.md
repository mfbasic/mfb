# didResize

Report whether the terminal was resized since the last check

## Synopsis

```
term::didResize() AS Boolean
```

## Package

term

## Imports

```
IMPORT term
```

`term` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/term/mod.rs:register]]

## Description

`term::didResize` returns `TRUE` when the terminal (CLI) or window (`--app`) has
changed size since the last time `term::didResize` was called, and `FALSE`
otherwise. The flag is **cached**: once a resize is detected it stays `TRUE`
across every intervening call until `term::didResize` observes it, so a program
that only polls occasionally never misses a resize. Reading it clears it, so the
very next call reports `FALSE` unless another resize has happened in between.
[[src/target/shared/code/term.rs:emit_did_resize]]

The resize is detected wherever the surface tracks its own geometry:

- In the CLI backend the shadow-grid present (`term::sync`) re-reads the terminal
  size each frame and reflows the grid; a genuine change latches the flag.
  [[src/target/shared/code/term_grid.rs:emit_grid_resize]]
- In `--app` mode each window backend records the change in its own resize hook —
  macOS in the `setFrameSize:` view callback, and Linux/GTK in the drawing area's
  `resize` signal — so `term::didResize` reports live window resizes too.

Like `term::isOn`, this query is **not gated**: it reads state only and never
touches the terminal, the alternate screen, or the shadow grid. Before any
`term::on` — or on a fixed-size app surface that never reflows — it simply reads
`FALSE`. A companion `term::terminalSize` call returns the new extent after
`term::didResize` reports a change.

## Parameters

`term::didResize` takes no parameters. [[src/codegen/registry/mod.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` if a resize occurred since the last call, `FALSE` otherwise. [[src/codegen/builtins/term/mod.rs:register]] |

## Errors

No errors.

## Examples

Reflow a layout only when the terminal changes size:

```
IMPORT term

SUB main()
  term::on()
  IF term::didResize() THEN
    term::clear()
    term::sync()
  END IF
  term::off()
END SUB
```

Re-read the new extent after a resize:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  IF term::didResize() THEN
    LET size = term::terminalSize()
    io::print("resized to " & toString(size.columns) & "x" & toString(size.rows))
  END IF
  term::off()
END SUB
```

## See also

- `mfb man term terminalSize`
- `mfb man term isOn`
- `mfb man term sync`
