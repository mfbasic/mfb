# bug-699: keyboard input differs on every input surface — special keys arrive in a different encoding or not at all, and `io`'s key readers cannot be uniform

Last updated: 2026-09-25
Effort: huge (>3d)
Severity: MEDIUM
Class: Correctness

Status: Open — **redesigned 2026-09-25 (owner).** Originally "special keys arrive
unstandardised per surface"; the owner widened it into one uniform `io` keyboard-input
surface, and folded in plan-69 (`term::` key down/up, deleted — see "plan-69").
Regression Test: `tests/app_special_keys.rs` plus the `io` function tests (to be
written, Phase 1)

A program reads keys with `io::readChar` / `io::readByte`, gated by `io::pollInput`.
What an arrow key or Delete produces depends on the input surface the program runs on:

- **Console on a terminal:** the terminal emulator sends VT/xterm sequences (Left =
  `ESC [ D`), and emulators disagree on details (`ESC O A` vs `ESC [ A` by cursor-key
  mode; Home as `ESC [ H`, `ESC [ 1 ~` or `ESC O H`; Backspace `0x7F` or `0x08`). The
  eight examples that handle arrows (`snake`, `dungeon`, `wind`, `water`, `ai_chat`,
  `browser`, `bugs/canvas`, `bugs/tui`) decode that shape by hand.
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
| macOS aarch64 | `--app`, Canvas | U+F700–F703 ✗ | U+F728 ✗ | `0x7F` ✓ | `0x1B` ✓ | measured (above) |
| macOS aarch64 | `--app`, TUI (TermView raw) | U+F700–F703 ✗ (guess) | U+F728 ✗ (guess) | `0x7F` | `0x1B` | code read: `macapp.term.keyDown` default arm appends `chars` verbatim |
| Linux (GTK) | `--app`, any raw | dropped ✗ | `0x7F` (guess) | `0x08` (guess) | `0x1B` | code read: `gdk_keyval_to_unicode` → 0 → `ignore` |
| Windows x86-64 | `--app`, Canvas | dropped ✗ | dropped ✗ | `0x08` | `0x1B` | code read: `emit_wndproc` handles `WM_CHAR` only |
| any | console on a VT terminal | `ESC[A–D` ✓ | `ESC[3~` ✓ | `0x7F` ✓ | `0x1B` ✓ | terminal emulator standard (not measured) |

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
  flags (disambiguate, report event types, report all keys as escape codes), and
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
- `io::input([prompt], [timeoutMs])`: two positional optionals, so a timeout with no
  prompt is `io::input("", 5000)`. Confirm against §6 overload/default rules.

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
  examples, descriptors, tests).
- **Examples to migrate — all 13** (`grep -rlE "io::(readChar|readByte|pollInput)\b"
  examples`, 2026-09-25; the list first named only six):

  | Example | Uses | Hand-rolled Esc decoder → `readKey` |
  | --- | --- | --- |
  | `ai_chat` | `readChar`, `pollInput` | yes |
  | `browser` (`app`) | `readChar`, `pollInput` | yes |
  | `bugs/canvas` | `readChar`, `pollInput` | yes |
  | `bugs/tui` | `readChar`, `pollInput` | yes |
  | `dungeon` | `readChar`, `pollInput` | yes |
  | `snake` | `readChar`, `pollInput` | yes |
  | `water` | `readChar`, `pollInput` | yes |
  | `wind` | `readChar`, `pollInput` | yes |
  | `emoji` | `readChar`, `pollInput` ×2 | no |
  | `life` | `readChar`, `pollInput` ×2 | no |
  | `hangman` | `readChar` ×3 | no |
  | `hello_input` | `readChar` | no |
  | `wide-demo` | `readChar` | no |

  The eight decoders move to `readKey`; the other five move to `readScalar` /
  `readGrapheme` (or `readKey` where they wait for "any key"), and every
  `pollInput` becomes the reader's `timeoutMs`. Re-run the grep at Phase 6 start —
  it, not this table, is the list.
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

- [ ] `tests/app_special_keys.rs`: for each `--app` handler, assert the emitted
      injection for Left, Delete, F5, a release, and Shift+Left (fails today). Follow
      the emitted-helper tests in `src/target/linux_gtk/bootstrap.rs` (bug-421).
- [ ] Windows: a non-ASCII `WM_CHAR` is written as UTF-8 (fails today).
- [ ] Record the Linux and Windows rows of the matrix by running the probe on the
      remote machines (`.ai/remote_systems.md`; `xdotool`, `SendInput`), replacing
      the "code read" entries. Confirm the macOS transcript raw path.
- [ ] Windows console: measure what a raw read sees for an arrow with and without
      `ENABLE_VIRTUAL_TERMINAL_INPUT`.
- [ ] Decoder test corpus: byte streams for every Goal-table row, every emulator
      variant, kitty `CSI u` press/repeat/release with modifiers, invalid UTF-8,
      lone Esc, Esc+letter, split sequences, ZWJ and combining clusters, CR LF.
- [ ] Settle the Open details with the owner.

Acceptance: the new tests fail for the documented reason; the matrix is measured on
three platforms.
Commit: —

### Phase 2: input buffer + decoder (runtime; readers unchanged)

- [ ] The buffer under the broadcast log; existing readers consume through it with
      today's behavior (golden-identical).
- [ ] The decoder and the 25 ms delay constant; unit-tested on the Phase 1 corpus.

Acceptance: full suite golden-identical; the corpus passes.
Commit: —

### Phase 3: `--app` injection and key release

- [ ] macOS: one shared translator helper for the three `keyDown:` paths; `keyUp:`
      and `flagsChanged:` IMPs; kitty-form injection.
- [ ] GTK: key-press and key-release handlers inject before `gdk_keyval_to_unicode`
      (`GDK_KEY_Delete` explicitly, not `0x7F`).
- [ ] Windows: `WM_KEYDOWN`/`WM_KEYUP`/`WM_SYSKEY*` arms in `emit_wndproc` and
      `editproc`; UTF-16 → UTF-8; Backspace `0x7F`. Mind the `WndProc` frame
      alignment (`≡ 8 mod 16`, `.ai/arch-abi.md`).

Acceptance: Phase 1 handler tests pass; the probe matches on all three platforms.
Commit: —

### Phase 4: the new readers and timeouts

- [ ] `io::readRaw`, `readScalar`, `readGrapheme`, `readKey`; `io::Key`,
      `io::KeyName`, `io::KeyEvent` registry descriptors and lowering.
- [ ] `[timeoutMs]` on `readLine` and `input` (a timeout leaves a partial line
      buffered).
- [ ] Function tests (`.ai/compiler.md` conventions), fed by piped stdin and the
      corpus; `readRaw` byte-exact on a pipe.
- [ ] Man pages per `.ai/man-content.md` (developer terms; `readRaw` states it
      differs by platform and terminal; `readKey` carries the `NotSupported`
      modifier table); `scripts/man-run-examples.sh io --run`.

Acceptance: function tests pass on macOS and Linux; man examples run.
Commit: —

### Phase 5: kitty in `term::on` / `term::off`

- [ ] Detection (`CSI ? u` + `CSI c`), push flags in `term::on`, pop in `term::off`
      and the exit-path restore.
- [ ] Console `readKey` reports `Press`/`Repeat`/`Release` under a kitty terminal and
      `NotSupported` elsewhere; `readScalar`/`readGrapheme`/`readLine` see plain text
      either way.
- [ ] Windows console: VT input per the Phase 1 measurement.

Acceptance: a kitty-capable terminal (manual, named in the commit) shows releases;
Terminal.app shows `NotSupported`; the terminal is clean after exit, including after
a crash.
Commit: —

### Phase 6: remove `readChar` / `readByte` / `pollInput`

- [ ] Delete the three members and their descriptors; migrate every caller in the
      Blast Radius (all 13 examples in its table — the eight decoders to `readKey` —
      and `pollInput` loops to `timeoutMs = 0` with `TRAP … ErrTimeout`).
- [ ] **Examples gate:** `bash scripts/build-examples.sh` (builds every executable
      project under `examples/` for all five registered targets and exits non-zero
      on any failure; CI job `examples` in `.github/workflows/coverage.yml`) must
      exit 0 **before the removal commit lands**, run locally against a compiler
      built from that commit. The script reuses an existing `target/release/mfb`,
      so rebuild it first — a stale compiler that still has `readChar` would pass.
      CI runs the job only after `test`/`acceptance`/`artifact`/`coverage`, so it
      is not a substitute for the local run.
- [ ] Spec and man updated; `grep -rn "readChar\|readByte\|pollInput" src examples
      tests packages` returns nothing live.

Acceptance: the grep is empty; `scripts/build-examples.sh` exits 0 on a fresh
compiler; every migrated example's keys work in a console build and a macOS `--app`
build.
Commit: —

### Phase 7: full validation

- [ ] Full suite (`cargo test`), `cargo test --bin mfb spec`,
      `scripts/man-census.sh --memory-scope` (0 unclassified),
      `bash scripts/build-examples.sh` exits 0.
- [ ] The probe on macOS, Linux and Windows, console and `--app`; `wind`'s arrow
      keys by hand in `--app`.

Acceptance: full suite green; the probe prints the same key sequence everywhere.
Commit: —

## Validation Plan

- Regression tests: `tests/app_special_keys.rs`, the decoder corpus, the `io` reader
  function tests.
- Runtime proof: the probe driven by `osascript` (macOS), `xdotool` (Linux) and
  `SendInput` (Windows) prints one key sequence on every surface; a kitty terminal
  shows releases, a plain one `NotSupported`.
- Doc sync: `03_console-io.md` (buffer, encodings, delay, `NotSupported`), `mfb man
  io`, `mfb man term on/off`.
- Full suite: `cargo test`, `cargo test --bin mfb spec`, and the examples gate
  `bash scripts/build-examples.sh` (every example, every target).

## Open Decisions

All design decisions were taken by the owner on 2026-09-25 (see "Decisions"). The two
Open details under Goal remain for Phase 1.

## Summary

The keyboard reaches a program through five different encodings today, and `io`
offers only byte and scalar readers over them. The fix is one input buffer and one
decoder under six readers with the standard timeout, `--app` handlers that inject
kitty-form events (so key release works in every app), kitty in `term::on` for
consoles, and a clean removal of `readChar`/`readByte`/`pollInput`. The engineering
risk is the Windows `WndProc` arms (hand-emitted x86-64, frame alignment) and keeping
every existing reader golden-identical while the buffer goes in underneath (Phase 2).
