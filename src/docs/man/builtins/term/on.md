# on

Enter TUI mode: allocate the drawing surface and reset all `term::` state

## Synopsis

```
term::on() AS Nothing
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

`term::on` is the gate for the whole module. Every other `term::` call except
`term::isOn` short-circuits to a no-op (or, for the getters, to an inert default)
while TUI mode is off, so nothing a program draws takes effect until `term::on`
has returned. It takes no arguments.
[[src/target/shared/code/term.rs:emit_gate_inactive]]

The call does four things, in this order.

1. **Allocates the shadow grid.** It asks the terminal for its size with
   `TIOCGWINSZ`, falling back to 24 rows by 80 columns when that is unavailable,
   and allocates one arena block holding a back cell buffer, a front cell buffer,
   and the scratch the present builds its escape stream into. The block is
   zero-filled, which is a cleared surface, and its dirty flag is set so the first
   `term::sync` repaints in full. This happens **before** the active flag is set,
   so a program never observes TUI mode on with no surface behind it; if the
   allocation fails, `ErrOutOfMemory` is raised and the terminal is left
   completely untouched. [[src/target/shared/code/term_grid.rs:emit_grid_alloc]]
2. **Resets `term::` state to defaults**: foreground white (255, 255, 255),
   background black (0, 0, 0), bold off, underline off, cursor visible, and the
   shadow cursor at the home position (row 0, column 0).
   [[src/target/shared/code/term.rs:emit_on]]
3. **Switches the terminal to its alternate screen**, so the user's previous
   shell contents are preserved and restored by `term::off`, and resets the
   terminal's own colours.
4. **Puts a console tty into single-key mode**: `~ICANON`, `~ECHO`, `VMIN = 1`,
   `VTIME = 0`, so a `io::pollInput` + `io::readChar` loop registers bare
   keypresses without waiting for Return. The saved cooked line discipline is kept
   so `term::off`, `io::input`, and `io::readLine` can restore it. When standard
   input is not a terminal — piped input, a test harness — this step is inert, and
   if the terminal cannot be reconfigured it is abandoned rather than failing the
   call. [[src/target/shared/code/io_terminal.rs:emit_configure_stdin_terminal]]

The surface `term::on` establishes is **retained and double-buffered**: from here
on, drawing calls — including `io::print` and `io::write` — mutate the back cell
buffer rather than the terminal, and only `term::sync` presents a frame. A
program that draws without calling `term::sync` displays nothing.
[[src/target/shared/code/term_grid.rs:emit_grid_write]]

`term::on` is one of the two calls that are not gated, so calling it while TUI
mode is already on runs the whole sequence again: a fresh surface sized to the
terminal, defaults restored, and the previously drawn frame discarded. Guard with
`term::isOn` if that is not wanted.

## Parameters

`term::on` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of entering TUI mode. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The shadow-grid block sized to the terminal cannot be allocated. TUI mode is not entered and the terminal is left untouched. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Enter TUI mode, draw one frame, present it, and restore the terminal:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::clear()
  term::moveTo(0, 0)
  term::setForeground(255, 0, 0)
  io::print("hello in red")
  term::sync()
  term::off()
END SUB
```

Enter TUI mode only once:

```
IMPORT term

SUB main()
  IF NOT term::isOn() THEN
    term::on()
  END IF
END SUB
```

## See also

- `mfb man term off`
- `mfb man term sync`
- `mfb man term isOn`
- `mfb man term clear`
