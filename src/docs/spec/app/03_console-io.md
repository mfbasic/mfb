# App-Mode Console I/O

In a GUI build (`mfb build --app`) the MFBASIC program runs unchanged on a worker
thread; the `io::` module is re-pointed at the window instead of the process's
real terminal. Output is appended to a transcript view, and typed input is fed
back to the program through an OS pipe whose read end is `dup2`'d onto file
descriptor 0, so the ordinary console read helpers see window input without
modification. This topic specifies the externally-observable contract for each
`io::` operation and the line/raw input-mode machinery behind it.

The redirection is dispatched through shared codegen hooks; each platform
supplies the `emit_app_*` bodies (AppKit on macOS, GTK4 on Linux). The per-call
behaviour below is identical across platforms; the storage and toolkit details
diverge and are flagged.

## io:: behaviour in a GUI build

| Call | GUI behaviour | Headless / no-window fallback |
|------|---------------|-------------------------------|
| `io::write` / `io::print` | Append the UTF-8 text to the transcript (and a newline for `print`). When TUI mode is active, route into the term surface instead. | `write(1, bytes, len)` (+ `'\n'`) — but when TUI mode is active the text is stamped into the console shadow grid's back buffer and shown only on the next `term::sync`, never written straight to fd 1. |
| `io::writeError` / `io::printError` | Same, prefixed with `"[stderr] "` to visually distinguish error output. | `write(2, bytes, len)` (+ `'\n'`) |
| `io::flush` | No-op returning `OK`; transcript writes are already synchronous. | Same (no-op `OK`) |
| `io::input` | Switch to line-echo mode, render the prompt via the `io::write` helper, then read one committed line via the console `io::readLine` helper (reads fd 0). | Same; reads fd 0 (the pipe) |
| `io::readLine` | Unchanged console helper; reads fd 0 (the window input pipe). | Same |
| `io::readRaw` / `io::readScalar` / `io::readGrapheme` / `io::readKey` | Set raw mode (each key event's bytes are delivered immediately), then read fd 0 through the shared read engine and key decoder. | Same; reads fd 0 |
| `io::isInputTerminal` / `io::isOutputTerminal` / `io::isErrorTerminal` | Always `OK(TRUE)` — the window *is* the interactive console; no `isatty` probe. | Same (`OK(TRUE)`) |
| `io::terminalSize` | Not part of `io::`. The transcript-viewport sizing helper was retargeted to the `term::` backend; see *term-backend*. | n/a |

The read helpers (`readLine`, `input` and the four keystroke readers) are **not**
rewritten for app mode beyond the input-mode prelude — they are the standard
readers that read fd 0 (`./mfb spec stdlib io-input`), treating the window as a
keyboard source. The only thing that makes them window-aware is the pipe
`dup2`'d onto fd 0 at bootstrap. [[src/codegen/builtins/io/gen_read_key_family.rs:lower_key_reader]]

`io::write`/`flush`/`input`/`isTerminal` are dispatched to the platform
`emit_app_*` bodies only when `app_mode` is set; otherwise the normal console
lowering is used. [[src/target/macos_aarch64/app/app_io.rs:emit_app_io_write]]

### Result ABI

Every app-mode io helper obeys the standard fallible-call result ABI: tag in
`x0` (`0` = `RESULT_OK_TAG`), value in `x1`. The write/flush helpers return
`OK` with no value; `isTerminal` returns `OK(TRUE)`; `io::input` forwards the
`x0`/`x1`/`x2` result of the underlying `io::readLine` helper unchanged.
[[src/target/macos_aarch64/app/app_io.rs:emit_app_io_is_terminal]]

## The input pipe (fd 0 redirection)

At bootstrap the runtime creates an anonymous pipe and `dup2`s its read end onto
fd 0. The program's read helpers therefore consume bytes the window's key
handler writes into the pipe's write end. The write fd is stashed where the key
handler can reach it.

macOS layout — fds live in the `_main` stack frame and the write fd is stored as
an objc associated object on the shared `NSApplication`:

```text
pipe(fds@sp+OFF_PIPE)        OFF_PIPE = 24   ; fds[0]=read @sp+24, fds[1]=write @sp+28
dup2(fds[0], 0)                              ; read end -> stdin (fd 0)
objc_setAssociatedObject(app, &PIPE_ASSOC_KEY, fds[1], ASSIGN)
```
[[src/target/macos_aarch64/app/mod.rs:OFF_PIPE]]

Linux layout — fds live in the writable `_mfb_gtkapp_state` global so every
helper reaches them without register preservation:

| Slot | Offset | Contents |
|------|--------|----------|
| `ST_PIPE_READ_FD` | 40 | pipe read fd (then `dup2`'d to 0) |
| `ST_PIPE_WRITE_FD` | 48 | pipe write fd (key handler writes here) |

[[src/target/linux_gtk/mod.rs:ST_PIPE_WRITE_FD]]

When no window is attached (the macOS `MFB_MACAPP_HEADLESS` test path, or before
the window exists) there is no transcript/buffer and the write helpers fall back
to `write()` on fd 1/2 directly; reads still come from fd 0. [[src/target/macos_aarch64/app/app_io.rs:emit_app_io_write]]

## The key handler that feeds the pipe

Typed keys are captured by key event handlers and translated into bytes written
to the pipe write fd. macOS overrides `keyDown:` and `keyUp:` on the synthesized
`MFBTextView : NSTextView` (the transcript), `TermView` (the TUI) and
`MFBCanvasView` (the canvas); Linux installs `key-pressed` and `key-released`
handlers on one window key controller in the capture phase; Windows handles the
key messages in the window procedure and in the transcript EDIT's subclass. All
run on the GUI main thread.
[[src/target/macos_aarch64/app/term_view.rs:emit_key_down_helper]] [[src/target/linux_gtk/bootstrap.rs:emit_key_pressed_handler]]

### Key events in the kitty form

In raw mode — and always on the macOS canvas view — every key event is written
as the bytes a kitty keyboard-protocol terminal sends for it under the flags
`term::on` pushes (flags 31: `CSI number ; mods : event ; text u`, or the legacy
functional form `CSI 1 ; mods : event D`), through the shared encoder
`_mfb_rt_input_encode_key`. A press, a repeat and a release are each an event;
the modifiers ride on every one; a text key carries its typed text as the
associated-text field. The key decoder reads the pipe exactly as it reads a kitty
terminal, so an `--app` read reports `Press`/`Repeat`/`Release` on every
platform (`./mfb spec stdlib io-input`). A modifier key alone writes nothing.
[[src/codegen/io/input/encode.rs:lower_input_encode_key]]

Each platform gathers the encoder's inputs its own way:

- **macOS** (`_mfb_macapp_key_inject`): the first code point of
  `charactersIgnoringModifiers` is both the AppKit table's native code
  (`NSLeftArrowFunctionKey` U+F700 …; Return, Tab, Escape and Delete as their C0
  characters) and a text key's unshifted number; `characters` is the typed text;
  `modifierFlags` gives Shift, Control, Option (Alt) and Command (Meta);
  `isARepeat` tells a repeat from a press. The transcript's and TUI's `keyUp:`
  report the release in raw mode only; the canvas view's always. `flagsChanged:`
  is not overridden.
  [[src/target/macos_aarch64/app/keys.rs:KEY_INJECT_SYMBOL]]
  [[src/target/macos_aarch64/app/keys.rs:emit_key_up_helper]]
- **Linux** (`_mfb_gtkapp_key_inject`): the keyval is the GDK table's native code
  for a non-text key; `gdk_keyval_to_lower` + `gdk_keyval_to_unicode` give a text
  key's unshifted number and `gdk_keyval_to_unicode` of the keyval its text; the
  `GdkModifierType` state gives the modifiers. GTK 4 reports no auto-repeat flag,
  so the helper keeps a 256-byte table of which hardware keycodes are down
  (`_mfb_gtkapp_keys_held`): a press of a key already down is a repeat. In line
  mode `key-released` only marks the key up. The injection runs before any "is
  this a character" test, which is what lets the arrows, Home/End, Page Up/Down
  and the F-keys through (`gdk_keyval_to_unicode` maps them all to 0).
  [[src/target/linux_gtk/keys.rs:KEY_INJECT_SYMBOL]]
  [[src/target/linux_gtk/keys.rs:emit_key_released_handler]]
  [[src/target/linux_gtk/keys.rs:KEYS_HELD_SYMBOL]]
- **Windows** (`_mfb_winapp_key_inject`, from both the window procedure and the
  EDIT subclass): a non-text key (the arrows, Home/End, Page Up/Down, Insert,
  Delete, the F-keys, Enter, Tab, Backspace, Escape) is injected at its
  `WM_KEYDOWN`/`WM_SYSKEYDOWN` and its release at the key-up, and its `WM_CHAR`
  is not delivered a second time. A text key depends on the mode
  (`_mfb_winapp_input_mode`): in RAW mode (the keystroke readers and `term::on`)
  its `WM_CHAR` carries the text, the unshifted character (`MapVirtualKeyW`) is
  the key number, and a Ctrl chord is injected at its key-down; in the default
  mode its text is written as UTF-8 (CR as LF) for `io::readLine`/`io::input`,
  and its release is not reported. UTF-16 surrogate pairs are joined before
  encoding; the modifiers come from `GetKeyState` (a window with no focused
  child receives every key as `WM_SYS*`); AltGr typing a printable character is
  that character. A `WM_SYSKEY*` with Alt down also goes on to `DefWindowProc`,
  so the system menu keeps working.
  [[src/target/win_x86_64/app/keys.rs:KEY_INJECT_SYMBOL]]
  [[src/target/win_x86_64/app/keys.rs:INPUT_MODE_SYM]]

### Line modes

In the line modes the handler keeps its own line editor:

- **Non-text keys** are ignored (macOS: `[chars length] == 0`, or a function-key
  code in U+F700–U+F8FF; Linux: `gdk_keyval_to_unicode == 0`).
  [[src/target/macos_aarch64/app/keys.rs:emit_skip_function_key]]
- **Printable key**: append the character to the pending line buffer; in
  line-echo mode also echo it into the transcript.
- **Line modes**, Return/Enter (macOS CR `13` / LF `10` / Enter `3`; Linux
  `GDK_KEY_Return` / `GDK_KEY_KP_Enter`): write `line + '\n'` to the pipe, echo a
  newline in line-echo mode, then clear the buffer.
- **Line modes**, Backspace/Delete (macOS `8` / `127`; Linux `GDK_KEY_BackSpace`):
  drop the last character from the line buffer (and, in line-echo mode on macOS,
  from the transcript text storage). The Linux transcript echo-delete is byte-
  granular and ASCII-only (transcript echo-delete). [[src/target/linux_gtk/bootstrap.rs:emit_key_pressed_handler]]

The macOS handler returns `Nothing`; the Linux handler returns `TRUE` for keys
it consumes and `FALSE` otherwise (so window shortcuts still fire).

### Line buffer storage

macOS keeps the pending line in an `NSMutableString` attached to `NSApplication`
via `INPUT_LINE_KEY` (RETAIN association). Committing reads its `UTF8String`,
`write()`s those bytes plus a `'\n'`, then `setString:@""` clears it. [[src/target/macos_aarch64/app/term_view.rs:emit_key_down_helper]]

Linux keeps it in the state global:

| Slot | Contents |
|------|----------|
| `ST_LINE_LEN` | length of the pending line in `ST_LINE_BUF` |
| `ST_LINE_BUF` | accumulated UTF-8 bytes (cap `LINE_BUF_CAP` = 1024) |

The byte offsets are deliberately not repeated here: `./mfb spec app linux-runtime`
owns the `_mfb_gtkapp_state` layout. This table restated them and went stale when
`ST_ARGC`/`ST_ARGV` were inserted ahead of these slots (bug-240), so the offset it
labelled `ST_INPUT_MODE` had become `ST_ARGC` — a reader following it read argc as
the input mode.

Committing `write()`s `ST_LINE_BUF[0..ST_LINE_LEN]` then a `'\n'` to
`ST_PIPE_WRITE_FD`, then resets `ST_LINE_LEN` to 0. [[src/target/linux_gtk/mod.rs:ST_LINE_BUF]]

## Input modes (line vs raw)

A single per-process input-mode value selects the key handler's behaviour. It is
set by the read helper before it reads fd 0, so the same physical typing produces
line-buffered or byte-at-a-time delivery depending on which `io::` call is
waiting.

| Mode | macOS value | Linux value | Selected by | Pipe delivery | Echo |
|------|-------------|-------------|-------------|---------------|------|
| line, no echo | (default; never set explicitly) | `MODE_LINE_NOECHO` `0` (zero-init) | `io::readLine` | whole line on Return | none |
| line, echo | `INPUT_MODE_LINE_ECHO` `1` | `MODE_LINE_ECHO` `1` | `io::input` | whole line on Return | typed chars + newline echoed to transcript |
| raw, no echo | `INPUT_MODE_RAW_NO_ECHO` `2` | `MODE_RAW` `2` | the keystroke readers; `term::on` | each key event, in the kitty form, immediately | none |

[[src/target/macos_aarch64/app/mod.rs:INPUT_MODE_LINE_ECHO]] [[src/target/linux_gtk/mod.rs:ST_INPUT_MODE]]

macOS stores the mode as an objc associated object on `NSApplication` under
`INPUT_MODE_KEY` (ASSIGN); Linux stores it at `ST_INPUT_MODE` in the state global
(see `./mfb spec app linux-runtime` for the offset). On macOS the default (no `io::input`) is never written, so the
handler's line path runs whenever the mode is not `2` — only line-echo (`1`)
triggers the transcript echo. [[src/target/macos_aarch64/app/mod.rs:INPUT_MODE_KEY]]

### Mode toggles

- `io::input` sets line-echo, renders the prompt, then calls `io::readLine`.
  [[src/target/macos_aarch64/app/app_io.rs:emit_app_io_input]]
- The keystroke readers (`readRaw`, `readScalar`, `readGrapheme`, `readKey`) set
  raw mode through the `emit_app_raw_input_mode` platform hook
  (`emit_set_raw_input_mode` on macOS and Linux, `emit_set_input_mode` on
  Windows), at the *start* of the reader body in app mode, before the wait and
  the fd-0 read. [[src/target/macos_aarch64/app/app_io.rs:emit_set_raw_input_mode]] [[src/codegen/builtins/io/gen_read_key_family.rs:lower_key_reader]]
- `io::readLine` does not change the mode; it relies on the zero-initialized /
  unset default (line, no echo).

The mode is sticky: nothing resets it after a read, so the last reader's mode
persists until the next `io::input` or keystroke reader sets it again.

## See Also

* ./mfb spec app macos-runtime
* ./mfb spec app linux-runtime
* ./mfb spec app term-backend
* ./mfb spec memory program-startup
* ./mfb spec threading os-integration
* ./mfb spec stdlib io-input
