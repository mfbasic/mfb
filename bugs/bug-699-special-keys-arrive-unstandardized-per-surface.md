# bug-699: keyboard input differs on every input surface — special keys arrive in a different encoding or not at all, and `io`'s key readers cannot be uniform

Last updated: 2026-09-25 (Phases 1–2)
Effort: huge (>3d)
Severity: MEDIUM
Class: Correctness

Status: Open — **redesigned 2026-09-25 (owner).** Originally "special keys arrive
unstandardised per surface"; the owner widened it into one uniform `io` keyboard-input
surface, and folded in plan-69 (`term::` key down/up, deleted — see "plan-69").
Phases 1 and 2 are done (2026-09-25; landed on `main` via `800a1b966`, full suite green there); the Open details were settled by the owner
(2026-09-25, Decision 9); Phases 3–7 remain.
Regression Test: `tests/codegen/app_special_keys.rs` (`cargo test --test
app_special_keys`; the Phase 3 cases are `#[ignore]`d until then), the decoder corpus
and reader tests (`cargo test --bin mfb codegen::io::input`), and
`tests/runtime/rt_native_term_runtime.rs::native_term_mouse_line_read_keeps_the_bytes_after_an_escape`.
The `io` reader function tests come with the readers (Phase 4).

A program reads keys with `io::readChar` / `io::readByte`, gated by `io::pollInput`.
What an arrow key or Delete produces depends on the input surface the program runs on:

- **Console on a terminal:** the terminal emulator sends VT/xterm sequences (Left =
  `ESC [ D`), and emulators disagree on details (`ESC O A` vs `ESC [ A` by cursor-key
  mode; Home as `ESC [ H`, `ESC [ 1 ~` or `ESC O H`; Backspace `0x7F` or `0x08`). The
  six examples that handle arrows (`snake`, `dungeon`, `wind`, `browser`,
  `swat/canvas`, `swat/tui`) decode that shape by hand.
- **`--app` build on macOS** (canvas, TermView raw, transcript): the runtime
  writes the raw `NSEvent characters` to the input pipe. Arrows arrive as AppKit
  private-use code points (U+F700–F703), forward-delete as U+F728, and Backspace
  as `0x7F`.
- **`--app` build on Linux (GTK):** the raw path converts keys with
  `gdk_keyval_to_unicode`. That returns 0 for arrows, Home, End, PageUp/Down and
  F-keys, and the handler **drops** a 0, so those keys never arrive.
- **`--app` build on Windows:** only `WM_CHAR` feeds the pipe. Arrows,
  Delete, Home/End and F-keys produce `WM_KEYDOWN` but no `WM_CHAR`, so they
  never arrive either.

Three API defects sit on top of the encoding one:

- `io::readChar() AS String` names a "char", which has no meaning in MFBASIC: it
  returns one Unicode scalar as a `String`, while `Scalar` is a primitive type. A
  grapheme needs several calls.
- No reader returns a *key*. Every program that wants arrows carries its own escape
  decoder with its own Esc timeout.
- No key release is observable anywhere, so a program cannot track a held key.

The result: a program that says "arrow keys work" (`examples/wind` does, in its
header and in `commandFor`) is wrong in every `--app` build. Nothing reports an
error. The keys do nothing (Linux, Windows), or a stray three-byte character goes
to a decoder that throws it away (macOS).

**The single correct behavior a fix produces:** every keyboard reader in `io`
returns the same value for the same keystroke on every target and surface — console,
`--app` Console/TUI/Canvas, macOS, Linux, Windows — with the one exception the owner
chose to expose: `io::readRaw` returns the platform's actual bytes, and says so.

References:

- `src/docs/spec/app/03_console-io.md` §"Input modes (line vs raw)": raw mode
  promises "each key's bytes immediately" but does not define what the bytes of a
  non-character key are. This is the missing contract.
- `mfb spec language builtin-functions` §18.4 "Timeout convention" — the timeout
  rules every reader adopts.
- plan-94-B (`planning/completed/`): terminal mouse reports share stdin, are
  stripped by a decoder before the reads see them, and `pollInput` runs pending bytes
  through that decoder. The unified input buffer extends this precedent.
- Found while designing `examples/water`: its left/right/up/down rotate/scale
  controls and Delete-to-remove need these keys in a canvas app.

## Decisions (owner, 2026-09-25)

1. **The API.** Every reader takes the standard trailing `[timeoutMs AS Integer]`
   (§18.4: omitted blocks, `0` is one immediate attempt, positive waits that long,
   negative is `ErrInvalidArgument`, an expired wait raises `ErrTimeout`):

   | Function | Returns | Contract |
   | --- | --- | --- |
   | `io::readRaw([timeoutMs]) AS Byte` | one byte | The actual bytes the platform delivered. **May differ by target and terminal; the man page says so.** Byte-exact on a pipe or file. |
   | `io::readScalar([timeoutMs]) AS Scalar` | one scalar | Text only (below). |
   | `io::readGrapheme([timeoutMs]) AS String` | one extended grapheme cluster | Text only (below). |
   | `io::readKey([timeoutMs]) AS io::Key` | one key event | Uniform on every target. |
   | `io::readLine([timeoutMs]) AS String` | a line | Gains the timeout. A half-typed line stays in the buffer when the timeout fires; nothing is lost or returned early. |
   | `io::input([prompt], [timeoutMs]) AS String` | a line | Same. |

2. **Text-only readers.** `readScalar` / `readGrapheme` return Unicode text. Keys that
   have an ASCII control code arrive as that scalar, unchanged (Unicode adopted
   ASCII's control codes): Enter U+000A, Tab U+0009, Backspace U+007F, Esc U+001B,
   Ctrl+A…Ctrl+Z U+0001…U+001A. Keys with **no** Unicode value (arrows, Home/End,
   Page Up/Down, Insert, Delete, F-keys) are consumed as a whole sequence and **not
   returned**. Invalid bytes (e.g. `0xFF`, a truncated sequence) are **dropped, not
   reported** — no `ErrEncoding`.
3. **One fixed 25 ms delay**, shared by the lone-Esc decision (Esc vs the start of a
   sequence) and `readGrapheme`'s "is more of this cluster coming" decision, on
   keyboard input. On a pipe or file there is no timer: read ahead until the
   sequence or cluster is complete or input ends, so the same file always splits the
   same way.
4. **The key record** (in `io`):

   ```basic
   TYPE Key
     key AS KeyName        ' Unicode | Up | Down | Left | Right | Home | End | PageUp | PageDown
                           '   | Insert | Delete | Backspace | Enter | Tab | Escape | F1 … F12
     grapheme AS String
     event AS KeyEvent     ' Press | Repeat | Release | NotSupported
     shift AS Boolean
     ctrl AS Boolean
     alt AS Boolean        ' Option on macOS
     meta AS Boolean       ' Cmd on macOS, the Windows/Super key elsewhere
   END TYPE
   ```

   - `grapheme` is what `readGrapheme` would return for that keystroke: `"a"`,
     `"\n"` for Enter, `"\t"` for Tab, `"\u{7F}"` for Backspace, `"\u{1B}"` for Esc.
     It is `""` only for keys with no Unicode value. Enter/Tab/Backspace/Escape are
     returned **by name and** with their grapheme.
   - Ctrl+A is `key = Unicode`, `grapheme = "a"`, `ctrl = TRUE`.
   - `event = NotSupported` when the surface cannot tell press, repeat and release
     apart (a terminal without the kitty keyboard protocol, a pipe, a file). There
     `Release` never arrives, and the modifiers are best-effort; the man page carries
     one table of which modifiers a plain terminal can report (Ctrl+letter, Alt as an
     Esc prefix, Shift/Ctrl on arrows — not Shift on a letter, never Meta). No
     separate capability function.
5. **Kitty keyboard protocol is enabled by `term::on()` and disabled by `term::off()`**
   (and by every path that already restores the terminal — the exit path that resets
   mouse modes). Outside `term::on`, a console reports `NotSupported`.
6. **`--app` builds inject into the same buffer.** Every surface feeds one input
   buffer. The window key handlers inject escape codes instead of platform
   characters, so one decoder serves every surface.
7. **Removed, in one break:** `io::readChar`, `io::readByte`, `io::pollInput`. Every
   caller migrates in the same change (Blast Radius). No deprecated aliases.
   `pollInput`'s job is `timeoutMs = 0` on the reader the program actually uses.
9. **Settled from Phase 1 (owner, 2026-09-25).** (a) `io::readLine`/`io::input` keep
   raising `ErrEncoding` on invalid UTF-8 — they read data, where a dropped byte
   would be a silently shortened line; only the keystroke readers drop. (b)
   `io::input([prompt], [timeoutMs])` keeps its signature; its man page points a
   timed read without a prompt at `io::readLine(timeoutMs)`. (c) `term::on` pushes
   kitty's alternate-keys `0b100` and associated-text `0b10000` flags as well, so a
   shifted or layout-dependent key's text arrives from the terminal rather than being
   guessed.
8. **plan-69 is deleted** and rolled into this bug: its `term::Key` /
   `term::getKeyDown` / `term::getKeyUp` design is replaced by `io::readKey` and
   `KeyEvent`.

## Failing Reproduction

(Measured with the readers that exist today; Phase 6 removes `pollInput`/`readByte`,
and the Phase 7 probe is this program rewritten on `io::readKey`.)

A canvas app that records every byte it reads (`/tmp/keyprobe`, reproduced here in
full):

```
' src/main.mfb   (project.json: "mode": "app", "kind": "executable")
IMPORT app
IMPORT canvas
IMPORT color
IMPORT fs
IMPORT io

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  canvas::present([canvas::Rectangle[x := 0.0, y := 0.0, w := 200.0, h := 200.0, paint := canvas::fill(color::rgb(20, 40, 80))]])
  MUT seen AS String = ""
  DO WHILE TRUE
    IF io::pollInput(200) THEN
      LET b AS Byte = io::readByte()
      IF b = toByte(113) THEN EXIT DO
      seen = seen & toString(b) & " "
      fs::writeText("/tmp/keyprobe/bytes.txt", seen)
    END IF
  LOOP
  RETURN 0
END FUNC
```

```
mfb build /tmp/keyprobe
open /tmp/keyprobe/build/keyprobe.app
osascript -e 'tell application "System Events"
  set frontmost of (first process whose name is "keyprobe") to true
  keystroke "a"
  key code 123   -- Left
  key code 124   -- Right
  key code 126   -- Up
  key code 125   -- Down
  key code 51    -- Delete (Backspace)
  key code 117   -- Forward Delete
  key code 53    -- Esc
  keystroke "q"
end tell'
cat /tmp/keyprobe/bytes.txt
```

(Run on 2026-09-24 with `delay 0.3` between keystrokes.)

- Observed (macOS aarch64):
  `97 239 156 130 239 156 131 239 156 128 239 156 129 127 239 156 168 27`,
  meaning `a`, U+F702, U+F703, U+F700, U+F701, `0x7F`, U+F728, `ESC`.
- Expected: `97 27 91 68 27 91 67 27 91 65 27 91 66 127 27 91 51 126 27`,
  meaning `a`, `ESC[D`, `ESC[C`, `ESC[A`, `ESC[B`, `0x7F`, `ESC[3~`, `ESC`. This
  is what the same program gets as a console build on an xterm-compatible
  terminal (standard xterm encoding; not measured this session).

| Environment | Surface | Arrows | Fwd-Delete | Backspace | Esc | How known |
| --- | --- | --- | --- | --- | --- | --- |
| macOS aarch64 (15.7.7) | `--app`, Canvas, TUI (TermView raw), Console transcript raw | U+F700–F703 ✗ | U+F728 ✗ | `0x7F` ✓ | `0x1B` ✓ | measured 2026-09-25, all four surfaces byte-identical (below) |
| Linux GTK **aarch64** (box 2226, Debian 12, GTK 4.8.3) | `--app`, Canvas and TUI raw | dropped ✗ | `0x7F` ✗ | `0x08` ✗ | `0x1B` ✓ | measured 2026-09-25 (below) |
| Linux GTK aarch64 (2226) | `--app`, Console transcript raw | dropped ✗ | dropped ✗ | dropped ✗ | `0x1B` ✓ | measured 2026-09-25 (below) |
| Linux GTK **x86_64** (box 2228) | `--app`, any | — | — | — | — | **UNMEASURED**: 2228 (and 2227) refused ssh for the whole session; 2226 runs the same `linux_gtk` handler on aarch64 |
| Windows 11 x86-64 (2230, 10.0.26100) | `--app`, Canvas, Console, TUI | dropped ✗ | dropped ✗ | `0x08` ✗ | `0x1B` ✓ | measured 2026-09-25 by `PostMessageW` (below); real input by `SendInput` UNMEASURED |
| Windows 11 x86-64 (2230) | console build under ConPTY, during a read | `ESC[A–D` ✓ | `ESC[3~` ✓ | `0x7F` ✓ | `0x1B` ✓ | measured 2026-09-25 (below) |
| any | console on a VT terminal | `ESC[A–D` ✓ | `ESC[3~` ✓ | `0x7F` ✓ | `0x1B` ✓ | terminal emulator standard (not measured) |

**Measured matrix, full key sequence** (2026-09-25, Phase 1). Keys, in order: `a`,
Left, Right, Up, Down, Backspace, Delete, Esc, F5, Shift+Left, Home, End, `é`, `q`.
Built with a release `mfb` of this tree's base.

- **macOS 15.7.7 aarch64** — `osascript` could not be used (the session was locked,
  `CGSSessionScreenIsLocked=true`: the app never came to the front); `CGEventPostToPid`
  (ctypes, `AXIsProcessTrusted=True`) delivered to all four surfaces (Console
  transcript raw, TUI `term::on`, Canvas with and without `pollInput`), which printed
  the same bytes: `97 | EF9C82 EF9C83 EF9C80 EF9C81 | 127 | EF9CA8 | 27 | EF9C88 |
  EF9C82 | EF9CA9 | EF9CAB | C3A9` — AppKit's private-use code points for the arrows,
  Delete (U+F728), F5 (U+F708), Home (U+F729), End (U+F72B). **Shift+Left is
  byte-identical to Left**: the modifier is lost. This confirms the TUI row that was a
  guess.
- **Linux GTK, box 2226 (aarch64)** — 2228 was down, so this is the aarch64 build of
  the same `linux_gtk` handler, not the x86_64 row. Method: the extracted AppImage's
  inner ELF under a private `Xvfb`, `GDK_BACKEND=x11 GSK_RENDERER=cairo`, keys by
  `xdotool key` (XTEST, xdotool 3.20160805) after `windowfocus --sync`. Canvas
  (blocking `readByte`) and TUI: `97 8 127 27 195 169` — arrows, F5, Shift+Left, Home,
  End dropped; **Backspace is `0x08`, Delete is `0x7F`**. Console transcript raw: `97 27
  195 169` — **Backspace and Delete are dropped too** (see "Findings for later
  phases"). The bug doc's own canvas probe printed nothing until Return was pressed:
  on Linux `io::pollInput` does not switch the window to raw input, only `readByte`
  does.
- **Windows 11, box 2230** — `PostMessageW` of `WM_KEYDOWN`/`WM_KEYUP` with real
  scan-code `lParam`s (so `TranslateMessage` runs in the app's own loop), `é` as
  `WM_CHAR 0xE9`. Canvas (to the top-level window) and Console/TUI (to the `Edit`
  child): `97 8 27 233` — every non-character key dropped, Backspace `0x08`, and **`é`
  arrives as the single byte `0xE9`** (the low byte of the UTF-16 unit). `SendInput`:
  from the ssh session `GetLastError=5`; from an interactive task in session 1 the
  events were inserted and focus set, yet the app received nothing on any surface,
  including the path `PostMessageW` proves works — **real-input delivery UNMEASURED**.
- **Windows console target (ENABLE_VIRTUAL_TERMINAL_INPUT)** — driven through a real
  ConPTY (`ssh -tt` under a local pty, writing xterm bytes). `io::readByte` already
  sets `ENABLE_VIRTUAL_TERMINAL_INPUT` for the read (`emit_apply_raw_mode` in
  `src/target/win_x86_64/code.rs`: `orig & ~(ECHO|LINE) | 0x200`, measured mode
  `0x1F7` → `0x3F1`). During a read every key arrives in standard xterm form: Left `27
  91 68`, Delete `27 91 51 126`, F5 `27 91 49 53 126`, Shift+Left `27 91 49 59 50
  68`, Home `27 91 72`, End `27 91 70`, Backspace 127, `é` `195 169`. Keys typed while
  no read is in progress lose the arrows and Delete and turn 0x7F into `0x08` (the
  mode is restored after each read). A standalone `ReadFile` harness: with `0x0200`
  (`0x3F1`) the same VT sequences; **without it (`0x1F1`) arrows, Delete, F5,
  Shift+Left, Home, End and Esc are all dropped** and Backspace is `0x08`.

Contrast cases that work today on every surface: printable characters, Enter
(each `--app` handler turns CR into LF), Esc, and Tab.

## Root Cause

No layer of the runtime owns the encoding of a non-character key. Each platform's
key handler was written only to get *text* into the input pipe, so it forwards
what the OS gives as "the characters of this key". The OSes disagree on that, and
two of them give nothing for special keys:

- `src/target/macos_aarch64/app/bootstrap.rs:emit_canvas_key_down_helper`: the
  `ckd_raw` arm writes `[[event characters] UTF8String]` verbatim. AppKit reports
  function keys as `NSUpArrowFunctionKey` (U+F700) … `NSDeleteFunctionKey` (U+F728),
  and those go into the pipe unchanged.
- `src/target/macos_aarch64/app/term_view.rs` (`macapp.term.keyDown`, the
  `TERM_KEY_DOWN_SYMBOL` IMP): in `INPUT_MODE_RAW_NO_ECHO` it writes `chars`
  verbatim too. Same private-use code points.
- `src/target/linux_gtk/bootstrap.rs`: the key-press handler's `raw` label calls
  `gdk_keyval_to_unicode(keyval)` and jumps to `ignore` on 0. GDK maps
  `GDK_KEY_Left/Right/Up/Down/Home/End/Page_*/F*` to 0, so those keys are dropped
  before the pipe.
- `src/target/win_x86_64/app/mod.rs:emit_wndproc`: the `wnd_check_char` arm is the
  canvas pipe writer and matches `WM_CHAR` only. Also the transcript EDIT
  subclass (`editproc`, same file) handles only `WM_CHAR`. Windows gives
  special keys only as `WM_KEYDOWN` with a `VK_*` code, which neither arm reads.

A console build gets *an* encoding because the terminal emulator does the encoding
before the bytes reach fd 0. Every `--app` surface replaces the terminal emulator
with our own key handler, and none of those handlers took over that job.

The console path is not uniform either: it depends on the emulator (the variants
listed at the top), so a console build needs the same normaliser the `--app` surfaces
get. The Windows console target reads stdin through the console API
(`SetConsoleMode`, `src/target/win_x86_64/plan.rs`); whether raw reads there see VT
sequences at all depends on `ENABLE_VIRTUAL_TERMINAL_INPUT` — Phase 1 measures it.

And no layer owns *decoding* either: the only structured reader is the plan-94-B mouse
decoder. Keys are left to every program.

## Goal

- One **input buffer** per stdin subscriber, holding raw bytes in arrival order.
  Every reader (`readRaw`, `readScalar`, `readGrapheme`, `readKey`, `readLine`,
  `input`) consumes from it; decoding happens on read, so mixing readers never loses
  or reorders input. It sits under the existing per-thread broadcast log
  (`thread::openStdIn`) and beside the plan-94-B mouse decoder, which keeps stripping
  mouse reports first.
- **Canonical encodings the decoder accepts** (and `--app` builds inject):
  - legacy xterm, normalised from every emulator variant:

    | Key | Canonical bytes |
    | --- | --- |
    | Up / Down / Right / Left | `ESC [ A` / `B` / `C` / `D` (also accepts `ESC O A–D`) |
    | Home / End | `ESC [ H` / `ESC [ F` (also `ESC [ 1 ~` / `4 ~`, `ESC O H` / `F`) |
    | Insert / Delete (forward) | `ESC [ 2 ~` / `ESC [ 3 ~` |
    | Page Up / Page Down | `ESC [ 5 ~` / `ESC [ 6 ~` |
    | F1–F4 | `ESC O P` / `Q` / `R` / `S` |
    | F5–F12 | `ESC [ 15~ 17~ 18~ 19~ 20~ 21~ 23~ 24~` |
    | Modified arrows etc. | `ESC [ 1 ; m X` (xterm modifier parameter) |
    | Backspace | `0x7F` (`0x08` normalised to it) |
    | Enter | `LF` (`CR` normalised to it) |

  - kitty `CSI … u` with the press/repeat/release event field and modifiers — what a
    kitty terminal sends under `term::on`, and the form `--app` builds inject, since
    only it can carry a release and all four modifiers.
- **`--app` builds** (macOS canvas / TermView / transcript, GTK, Windows canvas and
  transcript) inject every key press, repeat and release in the kitty form, with
  modifiers, including the keys the OS reports no character for. They always report
  real events (`Press`/`Repeat`/`Release`): the window knows. A console reports real
  events only while `term::on` has enabled kitty on a terminal that supports it.
- **Kitty lifecycle.** `term::on` detects support without hanging (the kitty query
  `CSI ? u` followed by Primary Device Attributes `CSI c`, which every terminal
  answers; only a DA reply means unsupported), pushes the progressive-enhancement
  flags (disambiguate `0b1`, report event types `0b10`, report alternate keys
  `0b100`, report all keys as escape codes `0b1000`, report associated text
  `0b10000` — owner, 2026-09-25, Decision 9), and
  `term::off` and the terminal-restore exit path pop them.
- **The six readers** behave per the Decisions on every surface, with the timeout
  convention.
- **`readChar` / `readByte` / `pollInput` do not exist**; `mfb man io` lists the new
  readers; the spec states the input buffer, the canonical encodings, the 25 ms delay
  and the `NotSupported` rule, with `[[path:Symbol]]` citations.
- The macOS probe below, rewritten on `io::readKey`, prints the same key sequence on
  macOS, Linux and Windows `--app` builds and in a console build.

### Non-goals (must NOT change)

- Line-mode editing: Backspace still erases one code point; arrows in line mode stay
  ignored. This bug does not add line editing.
- Mouse reporting (`term::pollMouse`, `canvas::pollMouse`) keeps its own channel and
  decoder; the input buffer only has to leave it working.
- `readRaw` on a pipe or file is byte-exact: no normalisation of piped input.
- **Tempting wrong fixes, forbidden:** teaching each example's decoder to recognise
  U+F700 and friends; a per-platform `KeyName` table; a `keyReleaseSupported()`
  function (the owner chose `KeyEvent.NotSupported`); widening `readScalar` to return
  arrow sequences as text.

### Open detail (implementer decides and records here, Phase 1)

- Whether `io::readLine` / `io::input` also drop invalid bytes instead of raising
  `ErrEncoding` (Decision 2 was made for `readScalar`/`readGrapheme`). Consistency
  argues yes; raise it with the owner before changing it.
  **Settled by the owner, 2026-09-25 (Decision 9): keep `ErrEncoding` on
  `readLine`/`input`.** The recommendation it confirmed: A
  line reader is the data reader: it is what reads a piped or redirected file, where
  an ill-formed byte means the input is not the text the program expects, and
  dropping it would hand back a silently shortened line that looks valid. The
  keystroke readers are different in kind — a stray byte there is a key the decoder
  could not classify, and there is no line to corrupt. The asymmetry is stated on
  both man pages. The decoder supports either choice unchanged (its Skip outcome
  names the ill-formed bytes), so this only decides what the Phase 4 line reader does
  with a Skip. If the owner prefers consistency, the line readers drop and the man
  page says so; nothing else moves.
- `io::input([prompt], [timeoutMs])`: two positional optionals, so a timeout with no
  prompt is `io::input("", 5000)`. Confirm against §6 overload/default rules.
  **Checked against §6 (`mfb spec language functions`, "Named args"): a call may not
  skip a parameter before a later named one, so `io::input(timeoutMs := 5000)` is
  rejected (`TYPE_CALL_ARITY_MISMATCH`), and the only spellings are
  `io::input("", 5000)` and `io::input(prompt := "", timeoutMs := 5000)`.
  **Settled by the owner, 2026-09-25 (Decision 9):** keep the signature as decided, and have the man
  page point a caller who wants a timeout without a prompt at `io::readLine(5000)`,
  which is exactly that call (`input` differs only by writing its prompt). No new
  overload is needed.**

## Blast Radius

Found with `grep -rlE 'u\{1B\}|"UP"' examples`, a search for key handlers under
`src/target`, and `grep -rlE "io::(readChar|readByte|pollInput)\b"`:

- **Key handlers to inject from** (all fixed here):
  - `src/target/macos_aarch64/app/bootstrap.rs:emit_canvas_key_down_helper`;
    `src/target/macos_aarch64/app/term_view.rs` (`macapp.term.keyDown`, raw arm); the
    transcript `MFBTextView keyDown:` raw path. Each gains a `keyUp:` sibling and
    `flagsChanged:` for modifiers.
  - `src/target/linux_gtk/bootstrap.rs` key-press `raw` arm, plus a key-release
    handler. The line-mode arm stays (Non-goals).
  - `src/target/win_x86_64/app/mod.rs:emit_wndproc` `wnd_check_char` (canvas) and
    `editproc` (transcript): `WM_KEYDOWN` / `WM_KEYUP` / `WM_SYSKEYDOWN` arms.
- **Sibling defect on the Windows arms, fixed here:** both write `wParam & 255`, the
  *low byte of a UTF-16 unit*, so `é` (U+00E9) enters the pipe as the lone byte
  `0xE9` — invalid UTF-8 (code read, `and_registers … 255`; Phase 1 adds a test).
  The fix encodes UTF-16 units and surrogate pairs as UTF-8.
- **Console targets:** the normaliser runs on every console read from a terminal
  (macOS, linux x86_64/aarch64/riscv64, Windows console).
- **`term::on` / `term::off`** (`src/codegen/term/core/term.rs`, `ESC_ON`/`ESC_OFF`
  and the exit-path restore that already resets `ESC_MOUSE_OFF`): kitty push/pop.
- **The `io` registry** (`src/codegen/builtins/io/`): new `func_read_raw`,
  `func_read_scalar`, `func_read_grapheme`, `func_read_key`, the `Key` record and
  the `KeyName`/`KeyEvent` enums; timeouts on `func_read_line`/`func_input`; delete
  `func_read_char.rs`, `func_read_byte.rs`, `func_poll_input.rs`.
- **Callers to migrate** (measured 2026-09-25):
  `grep -rlE "io::NAME\b" examples tests packages src --include='*.mfb'` —
  `readChar` 18 files, `readByte` 5, `pollInput` 15; `grep -rlE
  "io::(readChar|readByte|pollInput)\b" src --include='*.rs'` — 21 Rust files (man
  examples, descriptors, tests). The six hand-rolled decoders (`snake`, `dungeon`,
  `wind`, `browser`, `swat/canvas`, `swat/tui`) move to `readKey`.
- **Docs:** `src/docs/spec/app/03_console-io.md`, the io/term spec sections, `mfb man
  io` and `mfb man term` (`on`/`off` mention kitty in developer terms), the narrative
  guides that mention the removed readers (`grep -rn "readChar\|readByte\|pollInput"
  src/docs`).
- **plan-69:** deleted by this bug. Historical mentions stay in
  `planning/completed/plan-98-A-canvas-mode-plumbing.md` (a completed record, not a
  live caller).

## Fix Design

Four layers, bottom up:

1. **Input buffer.** Per-subscriber raw byte ring under the broadcast log; readers
   consume through a cursor so decoding is lazy and mixing readers is lossless. The
   mouse decoder keeps its place in front.
2. **Decoder.** One state machine over the buffer: UTF-8 (dropping invalid bytes),
   legacy xterm/SS3 sequences (normalised per the Goal table), xterm modifier
   parameters, kitty `CSI … u` with event type and modifiers. Emits
   `{key, grapheme, event, modifiers}`; `readScalar`/`readGrapheme` take only the
   text part; `readKey` takes all of it; `readRaw` bypasses it. The 25 ms timer
   applies only when stdin is a terminal or an `--app` pipe
   (`io::isInputTerminal`-style test); pipes and files read ahead.
3. **Sources.** Console: terminal bytes, plus kitty under `term::on`. `--app`: each
   handler injects kitty-form sequences for press/repeat/release with modifiers,
   and UTF-8 text.
4. **API.** The six readers over the decoder, each with the §18.4 timeout: the wait
   is a `poll`/event wait on the buffer's source, and a timeout leaves the buffer
   untouched (a half line or half sequence stays).

Rejected alternatives:

- *Decode platform codes inside the old `io::readChar`.* `readByte` would still see
  raw platform bytes and Linux/Windows would still drop the keys.
- *A private encoding (e.g. U+E000+n per key).* Breaks every terminal's native form
  and needs a second decoder.
- *plan-69's separate `term::getKeyDown`/`getKeyUp` event channel beside the `io`
  pipe.* Two channels for one keyboard, and its "legacy re-encoder" to keep `io::`
  byte-identical under kitty is unnecessary once every reader decodes one buffer.
- *Xterm-only injection in `--app` builds.* Cannot carry a release or Meta.

Expected output shift: every golden that exercises `readChar`/`readByte`/`pollInput`
changes with the migration (expected, on the removal commit). Any other golden that
shifts is a bug-hunt trigger.

## Phases

Each phase lands on its own; the removal (Phase 6) is last so every phase before it
keeps the old readers working.

### Phase 1: failing tests, measurement, audit (no behavior change)

- [x] `tests/app_special_keys.rs`: for each `--app` handler, assert the emitted
      injection for Left, Delete, F5, a release, and Shift+Left (fails today). Follow
      the emitted-helper tests in `src/target/linux_gtk/bootstrap.rs` (bug-421).
      — **`tests/codegen/app_special_keys.rs`** (target `app_special_keys`): a file
      directly in `tests/` is rejected by `tests/guards/test_targets_registered.rs`.
      The cases are behavioral, not instruction patterns: each builds an `--app`
      program with `mfb build -ncode` for the target, finds the handler the program
      registers (macOS `class_addMethod(…, keyDown:/keyUp:, …)`, GTK
      `g_signal_connect_data(…, "key-pressed"/"key-released", …)`, the two Windows
      procedures), and RUNS it in a new `.ncode` interpreter
      (`tests/common/ncode_emu.rs`) against a model of AppKit/GDK/Win32 that hands it
      the real event (the `NSEvent` fields, keyval + state, or window message) and
      records the pipe bytes. Expected bytes are what a kitty terminal sends under
      the flags `term::on` will push (`CSI D`, `CSI 3 ~`, `CSI 15 ~`, `CSI 1;1:3 D`,
      `CSI 1;2 D`), so any correct Phase 3 passes. A positive control per platform
      (`a` reaches the pipe through every handler) passes today. The 16 red cases
      are `#[ignore = "bug-699 Phase 3: …"]`, each naming its reason.
      **RED, measured** (`cargo test --release --test app_special_keys --
      --include-ignored`, 2026-09-25, 4 passed / 16 failed; default run: 4 passed, 16
      ignored): macOS — all three
      surfaces (`MFBTextView`, `TermView`, canvas view) write `EF 9C 82` for Left,
      `EF 9C A8` Delete, `EF 9C 88` F5, `EF 9C 82` for Shift+Left, and no class
      registers `keyUp:`; GTK — Left, F5, Shift+Left write nothing
      (`gdk_keyval_to_unicode` → 0 → `ignore`), Delete writes `7F`, and no
      `key-released` handler is connected; Windows — both procedures write nothing
      for `WM_KEYDOWN`/`WM_KEYUP`. These match the measured matrix above.
- [x] Windows: a non-ASCII `WM_CHAR` is written as UTF-8 (fails today).
      — `windows_non_ascii_wm_char_is_written_as_utf8` (same file): `WM_CHAR 0xE9`
      must reach the pipe as `C3 A9` and the surrogate pair `D83D DE00` as
      `F0 9F 98 80`. **RED, measured**: both procedures write `E9`. Control
      `windows_ascii_wm_char_is_written_verbatim` passes. Confirmed on the real
      box: `é` posted to 2230 arrived as the byte 233.
- [x] Record the Linux and Windows rows of the matrix by running the probe on the
      remote machines (`.ai/remote_systems.md`; `xdotool`, `SendInput`), replacing
      the "code read" entries. Confirm the macOS transcript raw path.
      — Done with two gaps, both recorded in the matrix: **Linux x86_64 (2228) is
      UNMEASURED** (2228 and 2227 refused ssh all session; 2226 aarch64 ran the same
      handler), and **Windows real-input (`SendInput`) delivery is UNMEASURED**
      (events inserted, nothing delivered on any surface; `PostMessageW` through the
      app's own `TranslateMessage` loop was measured instead). macOS transcript raw
      confirmed (identical to the canvas bytes).
- [x] Windows console: measure what a raw read sees for an arrow with and without
      `ENABLE_VIRTUAL_TERMINAL_INPUT`.
      — With it (today's read mode `0x3F1`): `ESC [ D`; without it (`0x1F1`):
      nothing (arrows, Delete, F-keys, Home/End and Esc are all dropped). See the
      matrix notes; Phase 5 must also keep VT input on between reads.
- [x] Decoder test corpus: byte streams for every Goal-table row, every emulator
      variant, kitty `CSI u` press/repeat/release with modifiers, invalid UTF-8,
      lone Esc, Esc+letter, split sequences, ZWJ and combining clusters, CR LF.
      — `src/codegen/io/input/corpus.rs`, 260+ cases written by hand from the
      protocol documents (not computed from the decoder's tables): every Goal row
      in both event regimes; SS3, VT220 `1~`/`4~`/`11~`–`14~`, rxvt `7~`/`8~`/`a`–`d`
      /`$ ^ @`, kitty `CSI P/Q/S`/`13~`, Linux console `[[A`–`E`, xterm
      `modifyOtherKeys`; kitty `u` press/repeat/release, Shift/Alt/Ctrl/Super/Meta,
      Caps/Num Lock ignored, alternate keys, associated text, keypad and functional
      keys, modifier-only and F13+ keys dropped, undefined event type and surrogate
      code point dropped; device replies/paste markers/unstripped mouse reports
      dropped; C0 keys and Ctrl+letters; lone Esc (waits, then Escape), Esc+letter/
      Esc+Esc/Esc+sequence (Alt), Alt+`[`/Alt+`O`; ten split prefixes (wait) and
      their abandonment at `FINAL`; invalid UTF-8 per the maximal-subpart rule;
      combining, ZWJ, flag and Hangul clusters, a cluster ending at a control or a
      sequence, the 128-byte cap at its exact edge; CR, LF, CR LF, CR CR LF, LF CR.
- [x] Settle the Open details with the owner. — Settled 2026-09-25, recorded as
      Decision 9 and under "Open detail": line readers keep `ErrEncoding`; `input`
      keeps its signature and its man page points a prompt-less timed read at
      `io::readLine(timeoutMs)`; Phase 5 also pushes alternate keys and associated
      text.

Acceptance: the new tests fail for the documented reason; the matrix is measured on
three platforms. **Met, with the two UNMEASURED gaps above** (Linux x86_64; Windows
real-input delivery).
Commit: e11e4f0e2 (Phase 1), 85a4a77f4 (owner answers; macOS setMode key handlers), dbf7cbd98 (Windows MFB_WINAPP_INPUT zeros)

### Phase 2: input buffer + decoder (runtime; readers unchanged)

- [x] The buffer under the broadcast log; existing readers consume through it with
      today's behavior (golden-identical).
      — `src/codegen/io/input/{mod,buffer}.rs`. **Design:** a per-arena region
      (length word + 256 bytes, `INPUT_STATE_SLOTS`) appended past the mouse-state
      region, so it is per subscriber, lock-free, and moves no existing offset.
      Data flow: log/pipe → mouse pump → input buffer → readers. Every plain reader
      takes buffered bytes first (`emit_take_buffered_byte` at the top of
      `emit_stdin_byte_read`, lead AND continuation bytes), and `io::pollInput`
      reports a buffered byte ready before it waits (the log and `poll(fd 0)`
      cannot see it). A look-ahead reader consumes only its own token from the
      front (`emit_consume`). **Golden identity is by compile-time gate** (the
      plan-94-B B4 precedent): the region, the reader fronts and the decoder are
      emitted only in a program that calls a reader in `INPUT_BUFFER_READERS` —
      the Phase 4 look-ahead readers, so the list is EMPTY today and no program
      changes. Phase 4 adds its readers' symbols to that list; nothing else.
      The buffered paths are exercised now through a `#[cfg(test)]` switch
      (`FORCE_INPUT_BUFFER`) that reserves the region, lowering the real reader
      helpers and running them in the `.ncode` interpreter
      (`src/codegen/io/input/reader_tests.rs`, 7 cases: `readByte` buffer-then-
      source, `readChar` joining a buffered lead byte with a source continuation,
      `readLine` joining buffered bytes with the rest of the line, `pollInput`
      ready on a buffered byte and unchanged when empty).
      **Byte-identity measured** (release `mfb`, 2026-09-25):
      `scripts/artifact-gate.sh … io|thread|term` → 7/7/7 goldens, 0 diffs each;
      the `--app` fixtures that emit readers/mouse (`syntax/app/macos-app-mode-io`,
      `macos-app-mode-term`, `app-mouse-surface`: 16 goldens incl. every
      `.app.ncodesum`) rebuilt in scratch copies → 0 diffs.
      While threading the buffer through, the read loop head was found BELOW the
      mouse pump's drain — New bug 1, fixed here (it moves no golden: the head
      moves only when a pump or buffer is emitted, and no golden fixture reads a
      line with the mouse on).
- [x] The decoder and the 25 ms delay constant; unit-tested on the Phase 1 corpus.
      — `_mfb_rt_input_decode` (`src/codegen/io/input/decode.rs`), a pure function
      (buffer, length, flags `FINAL|EVENTS|CLUSTER`) → a 192-byte token record
      (need-more / skip n / key+event+mods+grapheme+text). Tables in `keys.rs`;
      clusters use the existing UAX #29 emitters over the utf8proc tables. The
      delay is `INPUT_DELAY_MS = 25` (`io::input`), and the mouse decoder's
      `ESCAPE_DELAY_MS` is now defined as it (one delay, Decision 3); the decoder
      itself never waits — "need more" tells the reader to wait up to the delay
      and ask again with `FINAL`. Specification: a Rust model
      (`model.rs`, test-only), with `unicode-segmentation` for clusters as an
      independent second implementation. **GREEN, measured** (`cargo test --bin mfb
      codegen::io::`, 14 passed): the model passes the corpus; the EMITTED decoder,
      run in the `.ncode` interpreter, passes the corpus as lowered for all five
      backends (macOS/Linux aarch64, x86_64 SysV, Windows x64, riscv64) and agrees
      with the model on 4000 random fragment strings. Controls: a decoder that drops
      Ctrl fails ≥10 corpus cases; mutating the emitted `CSI Z` modifier or the
      128-byte cluster cap fails the emitted-corpus test (checked by hand).

Acceptance: full suite golden-identical; the corpus passes. **Met for the scoped
gates run** (io/thread/term + the three app fixtures, 0 diffs; the full suite and
`artifact-gate all` were not run in this phase by instruction — no golden is
expected to move, since nothing is emitted in any program without the switch).
Commit: 4e70277e0

### Phase 3: `--app` injection and key release

- [x] macOS: one shared translator helper for the three `keyDown:` paths; `keyUp:`
      and `flagsChanged:` IMPs; kitty-form injection.
- [x] GTK: key-press and key-release handlers inject before `gdk_keyval_to_unicode`
      (`GDK_KEY_Delete` explicitly, not `0x7F`).
- [x] Windows: `WM_KEYDOWN`/`WM_KEYUP`/`WM_SYSKEY*` arms in `emit_wndproc` and
      `editproc`; UTF-16 → UTF-8; Backspace `0x7F`. Mind the `WndProc` frame
      alignment (`≡ 8 mod 16`, `.ai/arch-abi.md`).

Acceptance: Phase 1 handler tests pass; the probe matches on all three platforms.
Deviation: macOS `flagsChanged:` is not installed. A lone modifier press has no
text and no key name the readers return (`io::KeyName` names no modifier key), so
the IMP would inject nothing a reader could see; modifiers still arrive on every
key through `modifierFlags`. Windows' default (non-`term::on`) window input stays
hybrid: typed text as UTF-8 (CR → LF) so `readLine`/`input` keep working on the
edit control, non-text keys in the kitty form; `term::on` switches to all-kitty.
Commit: e2a1a0fbd

### Phase 4: the new readers and timeouts

- [x] `io::readRaw`, `readScalar`, `readGrapheme`, `readKey`; `io::Key`,
      `io::KeyName`, `io::KeyEvent` registry descriptors and lowering.
- [x] `[timeoutMs]` on `readLine` and `input` (a timeout leaves a partial line
      buffered).
- [x] Function tests (`.ai/compiler.md` conventions), fed by piped stdin and the
      corpus; `readRaw` byte-exact on a pipe.
- [x] Man pages per `.ai/man-content.md` (developer terms; `readRaw` states it
      differs by platform and terminal; `readKey` carries the `NotSupported`
      modifier table); `scripts/man-run-examples.sh io --run`.

Acceptance: function tests pass on macOS and Linux; man examples run.
Deviation: the `End` key is `io::KeyName.EndKey` — `END` is a reserved word, so
an enum variant cannot be spelled `End`.
Measured: `cargo test --release --test rt_io_key_readers` (8 pipe/pty cases),
`cargo test --bin mfb reader_tests`, `STDIN_FILE=… scripts/man-run-examples.sh io
--run input isInputTerminal readGrapheme readKey readLine readRaw readScalar write`
(18 built, 18 ran, 0 failed).
Commit: 7e554824c

### Phase 5: kitty in `term::on` / `term::off`

- [x] Detection (`CSI ? u` + `CSI c`), push flags in `term::on` — disambiguate,
      event types, alternate keys, all keys as escape codes, associated text
      (`CSI > 31 u`; Decision 9) — pop in `term::off` and the exit-path restore.
- [x] Console `readKey` reports `Press`/`Repeat`/`Release` under a kitty terminal and
      `NotSupported` elsewhere; `readScalar`/`readGrapheme`/`readLine` see plain text
      either way.
- [x] Windows console: VT input per the Phase 1 measurement.
      — The readers set `ENABLE_VIRTUAL_TERMINAL_INPUT` for their call and
      `term::on` for its whole duration (`emit_apply_raw_mode`); measured on box
      206 (Windows 11, `tools/key-probe`, KEY_EVENT records through
      `WriteConsoleInputW`): the full sequence arrives in reader mode and under
      `term::on`, as `NotSupported` events (conhost answers the kitty query
      with device attributes only).

Acceptance: a kitty-capable terminal (manual, named in the commit) shows releases;
Terminal.app shows `NotSupported`; the terminal is clean after exit, including after
a crash.
Done: `term::on` (console, in a program with key readers, stdin in raw mode)
writes `CSI ? u CSI c`, reads the replies for up to 500 ms, keeps any key typed
meanwhile in the input buffer, and pushes `CSI > 31 u` only on a flags reply;
`term::off` (also run by the exit/crash/signal teardown) pops it. `io::readLine`
and `io::input` under `term::on` pop around the cooked line read and push again.
Found while testing: under the protocol Ctrl+C arrives as a key, not SIGINT, so
the console reader raises SIGINT itself for a bare Ctrl+C press (POSIX).
Measured: `cargo test --release --test rt_io_key_readers` — a pty that answers
as kitty (`ESC[?0u ESC[?62;c`) sees push → keys with Press/Release/Shift → pop;
one that answers only DA sees neither and keeps `z`/Left typed before the reply;
readLine under term::on pops then re-pushes; Ctrl+C exits 130 with the pop.
Manual run in a real kitty-capable terminal: not done (no GUI terminal can be
driven from this session) — the pty emulates the terminal's replies.
Commit: c724dab58

### Phase 6: remove `readChar` / `readByte` / `pollInput`

- [x] Delete the three members and their descriptors; migrate every caller in the
      Blast Radius (the six decoders to `readKey`, `pollInput` loops to
      `timeoutMs = 0` with `TRAP … ErrTimeout`).
- [x] Spec and man updated; `grep -rn "readChar\|readByte\|pollInput" src examples
      tests packages` returns nothing live.

Acceptance: the grep is empty; every migrated example builds and its keys work in a
console build and a macOS `--app` build.
Done: every caller migrated — the examples (`ai_chat`, `browser`, `dungeon`,
`emoji`, `hangman`, `hello_input`, `life`, `snake`, both `swat` editions,
`water`, `wide-demo`, `wind`) now read through `io::readKey` /
`io::readGrapheme` with a timeout instead of `pollInput` + a hand-written
escape decoder; a key's release is skipped where only presses count. The
tests, fixtures, `scripts/test-macapp.sh` and the three probes under `tools/`
are migrated (the `readChar`/`readByte`/`pollInput` syntax and rt-behavior
fixtures are deleted with their members). The spec gains `stdlib io-input`.
Measured: `grep -rnwE "readChar|readByte|pollInput" src examples tests packages
tools scripts` → only four historical mentions ("since removed", "bug-699
replaced …"); `python3 tools/pty-probe/snake_life_advance_under_input.py`,
`python3 tools/pty-probe/life_quit_no_return.py`,
`python3 tools/dungeon-key-probe/probe.py`,
`python3 tools/example-eof-probe/probe.py` all PASS; `mfb test
tests/acceptance < /dev/null` 782/782 after the fix below;
`scripts/spec-census.sh --citations` 0 missing, `--links` 0 unresolved.
Commit: 0ef602045 (migration), plus the spec commit that follows it

### Phase 7: full validation

- [ ] Full suite (`cargo test`), `cargo test --bin mfb spec`,
      `scripts/man-census.sh --memory-scope` (0 unclassified).
- [x] The probe on macOS, Linux and Windows, console and `--app`; `wind`'s arrow
      keys by hand in `--app`.
      — `tools/key-probe` (`9e6d3b4e3`). Every run printed the same
      `PRESSES: a Left Right Up Down Backspace Delete Escape F5 Shift+Left Home
      EndKey é q`: macOS console (reader mode, `term::on` legacy, `term::on`
      kitty — pty) and `--app` transcript / TUI / canvas (`CGEventPostToPid`);
      Linux aarch64 GTK 4 on box 2226, the same six; Windows 11 on box 206 (real
      session) `--app` transcript / TUI / canvas by `SendInput` and console reader
      mode / `term::on` by `WriteConsoleInputW`. Releases appear on every `--app`
      surface and under the kitty pty, `NotSupported` on legacy terminals and the
      Windows console. **Not measured:** Linux x86_64 GTK (2227 and 2228 refused
      ssh all session) and a real kitty-capable terminal emulator (the pty answers
      as one). `wind` `--app` on macOS, keys by `CGEventPostToPid` and checked by
      window screenshots: Left ×6 spun the map to centre the Pacific, `q` ×3
      zoomed to 1.6×, Up ×4 panned north, Esc quit. Not by a human hand — no one
      was at the keyboard.

Acceptance: full suite green; the probe prints the same key sequence everywhere.
Commit: —

## Findings for later phases (Phase 1–2, 2026-09-25)

Recorded where they bite; none changes a Decision.

- **Phase 5 flags lose text.** The Goal's flags (disambiguate `0b1`, event types
  `0b10`, all keys as escape codes `0b1000`) send a text key as its UNSHIFTED code
  with no text (`Shift+a` is `CSI 97;2u`), so on a non-US layout the decoder cannot
  know what Shift typed. It falls back to the kitty alternate-key subfield when
  present, else to ASCII upper case (corpus: "kitty Shift+a without text is A") —
  right on a US layout only. **Settled by the owner (Decision 9):** Phase 5 also
  pushes alternate keys `0b100` and associated text `0b10000`, so the fallback
  never runs under a kitty terminal. Phase 3's injection should carry the associated
  text the same way (`CSI 97;2;65u`).
- **Alt+letter's text.** The decoder gives `ESC a` (a terminal's Meta-sends-Escape)
  and kitty `CSI 97;3u` the same token: `Unicode`, grapheme `"a"`, Alt, text `"a"` —
  so `readScalar` returns `a` for Alt+a on every surface, and never an ESC it did
  not see as the Escape key.
- **Events on plain bytes.** Plain text, C0 bytes and SS3/rxvt sequences report
  `NotSupported` even from a surface that reports events — they carry no event, and
  a `Press` with no `Release` ever to follow would break a held-key tracker. A CSI
  key sequence with no event field is `Press` exactly when the source reports
  events (`FLAG_EVENTS`), because kitty sends a plain press as `CSI D`.
- **Keys `io::KeyName` has no member for are dropped**, not guessed: kitty's
  modifier-only events (Left Shift, …), F13 and up, media and lock keys.
- **A reader with a timeout must set raw input for its WAIT, not only its read.**
  On Linux `--app` (and on any console) `io::pollInput` does not leave line mode,
  so the bug doc's own probe saw nothing until Return. Phase 4's `timeoutMs` wait
  is inside the reader, which already switches mode — keep the switch around the
  wait.
- **Windows console: VT input only during a read.** Arrows typed between reads are
  lost and Backspace becomes `0x08` (measured). Phase 5 should keep
  `ENABLE_VIRTUAL_TERMINAL_INPUT` on while `term::on` is active, not per read.
- **GTK transcript raw drops Backspace and Delete** (measured on 2226; Canvas and
  TUI keep them). Likely the `GtkTextView` consumes them before the window's key
  controller (a guess — not measured). Phase 3's GTK handler has to see them.
- **The macOS transcript's and TUI's `keyDown:` exist only in a program that starts
  in `Console`** — see "New bugs found". `tests/codegen/app_special_keys.rs` reads
  those two surfaces from a Console-start program for that reason.

## New bugs found (Phases 3–6)

4. **FIXED here — every timed reader in a program that uses the mouse failed to
   assemble** (`duplicate label '…_stdin_poll_ready_unlock'`): the reader's
   deadline wait and the mouse pump's escape wait both ran the stdin log's
   readiness check, whose labels were keyed on the function symbol. Found
   building `examples/swat/tui`. Fixed in `87e88f130` (labels keyed on the call
   site); test `rt_io_key_readers::every_reader_builds_and_reads_in_a_mouse_program`.
5. **FIXED here — `io::readKey(-1)` raised `ErrInputFailed`, not
   `ErrInvalidArgument`**, when an earlier reader had left 1 in the frame slot of
   the terminal-restore flag: the negative-timeout exit is taken before the
   terminal is configured, and the shared restore read the stale flag. Found by
   the acceptance suite. Fixed in `e9ad2a263` (flag zeroed in both reader
   prologues); test `reader_tests::a_negative_timeout_is_an_invalid_argument`
   (poisoned stack).
6. **FIXED here — under the kitty protocol Ctrl+C no longer interrupted a
   `term::on` program**: the terminal sends it as a key, not the byte the line
   discipline turns into SIGINT. The console reader now raises SIGINT for a bare
   Ctrl+C press (Phase 5, `c724dab58`); test
   `rt_io_key_readers::ctrl_c_under_the_kitty_protocol_still_interrupts_the_program`.
7. **bug-742** (`io::pollInput` crashes at exit after a recovered negative
   timeout) is moot: `pollInput` is removed. Its reproduction, re-shaped for the
   readers that replace it (recovered `readKey(-1)`/`readLine(-1)` followed by
   successful timed reads, at EOF, on a pipe and inside a helper `FUNC`), exits 0.
   Archived with this bug.

## New bugs found (Phase 1–2)

1. **FIXED here — mouse-mode `io::readLine` repeats an ESC in place of the rest
   of the line.** `readLine` looped back to its per-byte read BELOW the plan-94-B
   pump's owed-byte drain; after an `ESC x` flush the next fresh byte re-entered the
   decoder mid-flush. Repro (`term::enableMouse(TRUE)` then `io::readLine()` on
   piped stdin): `a ESC x b` → `97 27 27 27`, `a ESC [ Z b` → `97 27 27 27`,
   `ESC ESC q` → `27 27 27` (base compiler); without `enableMouse` all three are
   exact. Fixed by the same change that threads the input buffer (the loop head now
   sits above every pre-read stage); test
   `native_term_mouse_line_read_keeps_the_bytes_after_an_escape`.
2. **FIXED here — macOS: a program that calls `app::setMode` never registered
   `MFBTextView keyDown:` or the `TermView` class.** Both are created only by the `Console`-start branch of
   `emit_main_bootstrap` (`src/target/macos_aarch64/app/bootstrap.rs`); a `None`-start
   program's first `setMode(Console)` builds a plain non-editable `NSTextView`
   (`emit_reconcile_build_helper`), and nothing registers `TermView`. Found by the
   Phase 1 harness: a `setMode` program's dump contains `_mfb_macapp_key_down` and
   `_mfb_macapp_term_keyDown` with no reference to either. **Measured (macOS
   15.7.7, base compiler, keys by `CGEventPostToPid`):** a program that calls
   `app::setMode(app::Mode.Console)` and then loops on `io::readByte` receives
   nothing for `a`, `b`, `q` and never exits; the same program without the
   `setMode` line prints `97 98` and exits on `q`; a Canvas → Console → Canvas
   program received nothing on any surface.
   **Fix** (registration only; the bytes the handlers write are untouched, Phase 3's):
   the `MFBTextView` class, the `TermView` surface and the input-line buffer are now
   built by helpers both start modes call (`emit_textview_class`,
   `emit_termview_surface`, `emit_input_line_buffer`; the Console-start path emits
   exactly what it did). A `None` start stashes the class under
   `RECONCILE_TV_CLASS_KEY`; `emit_reconcile_build_helper` instantiates it and also
   records the view as what `term::off` swaps back; the `Console` reconcile makes it
   first responder. **RED → GREEN:**
   `app_special_keys::macos_a_mode_switching_program_registers_every_key_down_handler`
   (RED: only the canvas `keyDown:` found; GREEN), and by hand with
   `CGEventPostToPid` on the fixed compiler: `setMode(Console)` + reads → `97 98`,
   then `term::on` → `99`, after `term::off` → `100`; Canvas → Console → Canvas →
   `97 98 99` (base: nothing). `cargo test --bin mfb macos_aarch64` 39 passed;
   `cargo test --release --test cli_app_canvas_mode` 11 passed.
   **Goldens that move (expected, regenerate on merge):** only the two `setMode`
   fixtures' macOS app code —
   `tests/syntax/app/app-mouse-surface/golden/app_mouse_surface.macos-aarch64.app.ncodesum`
   and `tests/syntax/app/app-window-surface/golden/app_window_surface.macos-aarch64.app.ncodesum`
   (checked by rebuilding every native golden of those two plus
   `macos-app-mode-io`/`-term`: every other one identical; both are identical
   on the base compiler).
3. **FIXED here — Windows `--app`: a program that never calls `app::setMode` got
   `0` from `io::readByte` immediately and forever.** Reproduced on 2230
   (10.0.26100, base compiler, no input at all): a Console-start program reading 20
   bytes printed `0 0 … 0 END` within 5 s — also with `IMPORT app` + `getMode`;
   the same program with `setMode(Console)` waited correctly. **Mechanism:** `_main`'s
   `MFB_WINAPP_INPUT` test affordance (`emit_main`, `src/target/win_x86_64/app/mod.rs`)
   took `GetEnvironmentVariableW`'s count from `abi::return_register()` — the Win64
   MFB result register `rcx`, which the call leaves undefined — instead of the C
   result `abi::c_return(0)` (`rax`): the exact trap `.ai/testing-gates.md` "Win64
   codegen invariant" names. With the variable UNSET the count was junk, so `_main`
   posted that many `WM_CHAR 0`s from its zeroed buffer to the transcript EDIT,
   whose subclass wrote each byte into the input pipe. A `None` start zeroes the
   EDIT handle before the loop, so its posts went nowhere — why only Console starts
   showed it. The affordance itself was broken too: `MFB_WINAPP_INPUT=ab` delivered
   nothing. **Fix:** read the count from `c_return(0)` (two instructions).
   **RED → GREEN** (`tests/codegen/app_special_keys.rs`, running `_main` in the
   `.ncode` interpreter, which now leaves caller-saved registers holding junk after
   an external call as hardware does): `windows_main_posts_no_keystrokes_when_mfb_winapp_input_is_unset`
   (RED: 65 `WM_CHAR 0`s) and `windows_main_posts_mfb_winapp_input_then_enter`
   (RED: `a b 0 … 0 CR`); both GREEN. On 2230 with the fixed compiler: no input →
   `readByte` blocks (`started`, still running); `MFB_WINAPP_INPUT=ab` →
   `97 98 10`. `cargo test --bin mfb win_x86_64` 40 passed.
   **Goldens that move (expected, regenerate on merge):** every Windows `--app`
   native golden, since `_main` is in every Windows app binary —
   `app-mouse-surface`, `app-window-surface`, `macos-app-mode-io`,
   `macos-app-mode-plumbing`, `macos-app-mode-term` `.windows-x86_64.app.ncodesum`
   (5 files; all identical on the base compiler).

## Validation Plan

- Regression tests: `tests/app_special_keys.rs`, the decoder corpus, the `io` reader
  function tests.
- Runtime proof: the probe driven by `osascript` (macOS), `xdotool` (Linux) and
  `SendInput` (Windows) prints one key sequence on every surface; a kitty terminal
  shows releases, a plain one `NotSupported`.
- Doc sync: `03_console-io.md` (buffer, encodings, delay, `NotSupported`), `mfb man
  io`, `mfb man term on/off`.
- Full suite: `cargo test`, `cargo test --bin mfb spec`.

## Open Decisions

All design decisions were taken by the owner on 2026-09-25 (see "Decisions", including
Decision 9, which settled the two Open details and the Phase 5 flag set).

## Summary

The keyboard reaches a program through five different encodings today, and `io`
offers only byte and scalar readers over them. The fix is one input buffer and one
decoder under six readers with the standard timeout, `--app` handlers that inject
kitty-form events (so key release works in every app), kitty in `term::on` for
consoles, and a clean removal of `readChar`/`readByte`/`pollInput`. The engineering
risk is the Windows `WndProc` arms (hand-emitted x86-64, frame alignment) and keeping
every existing reader golden-identical while the buffer goes in underneath (Phase 2).
