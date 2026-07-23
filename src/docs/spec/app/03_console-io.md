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
| `io::readChar` / `io::readByte` | Set raw mode (each keystroke's bytes are delivered immediately), then read fd 0 via the unchanged console helper. | Same; reads fd 0 |
| `io::isInputTerminal` / `io::isOutputTerminal` / `io::isErrorTerminal` | Always `OK(TRUE)` — the window *is* the interactive console; no `isatty` probe. | Same (`OK(TRUE)`) |
| `io::terminalSize` | Not part of `io::`. The transcript-viewport sizing helper was retargeted to the `term::` backend; see *term-backend*. | n/a |

The read helpers (`readLine`/`readChar`/`readByte`/`input`) are **not** rewritten
for app mode beyond the input-mode prelude — they remain the standard console
helpers that read fd 0. The only thing that makes them window-aware is the pipe
`dup2`'d onto fd 0 at bootstrap. [[src/target/shared/code/io_stdin.rs:lower_io_read_byte_helper]]

`io::write`/`flush`/`input`/`isTerminal` are dispatched to the platform
`emit_app_*` bodies only when `app_mode` is set; otherwise the normal console
lowering is used. [[src/target/macos_aarch64/app/app_io.rs:emit_app_io_write_helper]]

### Result ABI

Every app-mode io helper obeys the standard fallible-call result ABI: tag in
`x0` (`0` = `RESULT_OK_TAG`), value in `x1`. The write/flush helpers return
`OK` with no value; `isTerminal` returns `OK(TRUE)`; `io::input` forwards the
`x0`/`x1`/`x2` result of the underlying `io::readLine` helper unchanged.
[[src/target/macos_aarch64/app/app_io.rs:emit_app_io_is_terminal_helper]]

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
to `write()` on fd 1/2 directly; reads still come from fd 0. [[src/target/macos_aarch64/app/app_io.rs:emit_app_io_write_helper]]

## The key handler that feeds the pipe

Typed keys are captured by a key event handler on the transcript view and
translated into bytes written to the pipe write fd. macOS overrides
`keyDown:` on a synthesized `MFBTextView : NSTextView`; Linux installs a GTK
`key-pressed` controller handler. Both run on the GUI main thread.
[[src/target/macos_aarch64/app/term_view.rs:emit_key_down_helper]] [[src/target/linux_gtk/bootstrap.rs:emit_key_pressed_handler]]

The handler dispatches on the current input mode and the key:

- **Modifier-only / non-character keys** are ignored (macOS: `[chars length] ==
  0`; Linux: `gdk_keyval_to_unicode == 0`).
- **Raw mode**: the key's UTF-8 bytes are written to the pipe immediately — no
  line buffer, no transcript echo.
- **Line modes**, printable key: append the character to the pending line
  buffer; in line-echo mode also echo it into the transcript.
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
| raw, no echo | `INPUT_MODE_RAW_NO_ECHO` `2` | `MODE_RAW` `2` | `io::readChar` / `io::readByte` | each key's bytes immediately | none |

[[src/target/macos_aarch64/app/mod.rs:INPUT_MODE_LINE_ECHO]] [[src/target/linux_gtk/mod.rs:ST_INPUT_MODE]]

macOS stores the mode as an objc associated object on `NSApplication` under
`INPUT_MODE_KEY` (ASSIGN); Linux stores it at `ST_INPUT_MODE` in the state global
(see `./mfb spec app linux-runtime` for the offset). On macOS the default (no `io::input`) is never written, so the
handler's line path runs whenever the mode is not `2` — only line-echo (`1`)
triggers the transcript echo. [[src/target/macos_aarch64/app/mod.rs:INPUT_MODE_KEY]]

### Mode toggles

- `io::input` sets line-echo, renders the prompt, then calls `io::readLine`.
  [[src/target/macos_aarch64/app/app_io.rs:emit_app_io_input_helper]]
- `io::readChar` / `io::readByte` set raw mode via `emit_set_raw_input_mode`,
  injected at the *start* of the console read-char/read-byte helper bodies in app
  mode, before the fd-0 read. [[src/target/macos_aarch64/app/app_io.rs:emit_set_raw_input_mode]] [[src/target/shared/code/io_stdin.rs:lower_io_read_byte_helper]]
- `io::readLine` does not change the mode; it relies on the zero-initialized /
  unset default (line, no echo).

The mode is sticky: nothing resets it after a read, so the last reader's mode
persists until the next `io::input`/`readChar`/`readByte` sets it again.

## See Also

* ./mfb spec app macos-runtime
* ./mfb spec app linux-runtime
* ./mfb spec app term-backend
* ./mfb spec memory program-startup
* ./mfb spec threading os-integration
