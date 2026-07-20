# hideCursor

Hide the terminal cursor in presented frames

## Synopsis

```
term::hideCursor() AS Nothing
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

`term::hideCursor` marks the cursor as hidden. It takes no arguments.
[[src/builtins/term.rs:arity]]

Like everything else on this retained surface, the call **emits no escape
sequence**. It clears a single visibility flag in the module's state; the terminal
is only told about it when `term::sync` presents a frame, whose trailing sequence
shows or hides the cursor according to this flag. Until the next present the
cursor stays as the previous frame left it.
[[src/target/shared/code/term.rs:emit_set_cursor_visible]]
[[src/target/shared/code/term_grid.rs:emit_grid_present]]

Hiding the cursor is the usual choice for a full-screen program that repaints
every frame: the terminal cursor would otherwise be parked at whatever cell the
last write ended on, blinking in the middle of the drawing.

Visibility is independent of the colours and text attributes and of the cursor's
position — hiding it does not move it, and `term::moveTo` still works normally
while it is hidden. Calling `term::hideCursor` twice is harmless, since this is a
flag rather than a toggle.

The flag persists until `term::showCursor` or the next `term::on`, which resets
the cursor to visible; `term::off` also makes the cursor visible again as part of
restoring the terminal. The call is gated: while TUI mode is off it does nothing
and reports no error. [[src/target/shared/code/term.rs:emit_gate_inactive]]

## Parameters

`term::hideCursor` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of marking the cursor hidden. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Draw a full-screen frame with no blinking cursor:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::hideCursor()
  term::clear()
  term::moveTo(0, 0)
  io::print("drawing without a blinking cursor")
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term showCursor`
- `mfb man term moveTo`
- `mfb man term sync`
- `mfb man term off`
