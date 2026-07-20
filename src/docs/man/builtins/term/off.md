# off

Leave TUI mode: present the final frame and restore the terminal

## Synopsis

```
term::off() AS Nothing
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

`term::off` tears down the TUI surface entered by `term::on` and returns the
terminal to the state it had before. It takes no arguments and is gated: while
TUI mode is already off the call does nothing at all and reports success.
[[src/target/shared/code/term.rs:emit_off]]

When TUI mode is on, the teardown runs in this order.

1. **A final `term::sync`.** `term::off` calls the present routine itself, so the
   last frame the program composed is displayed even if it never called
   `term::sync` explicitly. [[src/target/shared/code/term_grid.rs:emit_grid_present]]
2. **The cooked line discipline is restored**, undoing the single-key
   (`~ICANON`/`~ECHO`) mode `term::on` put a console tty into, so typing echoes
   and lines are submitted with Return again. A no-op when raw mode was never
   entered. [[src/target/shared/code/term.rs:emit_off]]
3. **The terminal is restored**: the cursor is made visible, the alternate screen
   is left so the user's previous shell contents reappear, and the terminal's
   colour and attribute state is reset so ordinary output that follows is drawn
   normally.
4. **The active flag is cleared and the shadow-grid block is freed** back to the
   arena. [[src/target/shared/code/term_grid.rs:emit_grid_free]]

After `term::off` returns, `term::isOn` reports `FALSE` and every `term::` call
except `term::on` and `term::isOn` is a no-op again. A later `term::on` starts
over with a freshly allocated surface and the default state; nothing drawn before
`term::off` survives it.

Because the alternate screen and the terminal's line discipline are both process
state, a program that enters TUI mode should reach `term::off` on every exit path
— including its error paths — or leave the user's terminal in single-key mode on
the alternate screen.

## Parameters

`term::off` takes no parameters. [[src/builtins/term.rs:call_param_names]]

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of leaving TUI mode and restoring the terminal. [[src/builtins/term.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Draw one frame and restore the terminal:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::moveTo(0, 0)
  io::print("done")
  term::off()          ' presents the frame, then restores the screen
END SUB
```

Leave TUI mode only if it was entered:

```
IMPORT term

SUB main()
  IF term::isOn() THEN
    term::off()
  END IF
END SUB
```

## See also

- `mfb man term on`
- `mfb man term sync`
- `mfb man term isOn`
