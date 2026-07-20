# showCursor

Make the terminal cursor visible in presented frames

## Synopsis

```
term::showCursor() AS Nothing
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

`term::showCursor` marks the cursor as visible. It takes no arguments.
[[src/builtins/term.rs:arity]]

Like everything else on this retained surface, the call **emits no escape
sequence**. It sets a single visibility flag in the module's state; the terminal
is only told about it when `term::sync` presents a frame. Every present ends with
a trailing sequence that parks the terminal cursor at the shadow cursor's current
position and then shows or hides it according to this flag, so the visible cursor
always tracks where the next drawing would go.
[[src/target/shared/code/term.rs:emit_set_cursor_visible]]
[[src/target/shared/code/term_grid.rs:emit_grid_present]]

Visibility is independent of the colours and text attributes and of the cursor's
position: showing the cursor changes none of them, and `term::moveTo` does not
change visibility. Calling `term::showCursor` when the cursor is already visible
is harmless — it is a flag, not a toggle.

The flag persists until `term::hideCursor` or the next `term::on`, which resets
the cursor to visible. The call is gated: while TUI mode is off it does nothing
and reports no error. [[src/target/shared/code/term.rs:emit_gate_inactive]]

## Parameters

`term::showCursor` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of marking the cursor visible. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Hide the cursor while a frame is drawn, then show it again for input:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::hideCursor()
  term::clear()
  io::print("rendering...")
  term::sync()

  term::showCursor()
  term::moveTo(2, 0)
  io::write("Name: ")
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term hideCursor`
- `mfb man term moveTo`
- `mfb man term sync`
- `mfb man term on`
