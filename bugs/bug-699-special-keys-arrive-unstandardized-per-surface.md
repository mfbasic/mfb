# bug-699: special keys (arrows, Delete, Home/End, …) arrive in a different encoding on every input surface, or not at all

Last updated: 2026-09-24
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/app_special_keys.rs (to be written, Phase 1)

A program reads keys with `io::readChar` / `io::readByte`. What an arrow key or
Delete produces depends on the input surface the program runs on:

- **Console on a terminal:** the terminal emulator sends VT/xterm sequences (Left =
  `ESC [ D`). The six examples that handle arrows (`snake`, `dungeon`, `wind`,
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

The result: a program that says "arrow keys work" (`examples/wind` does, in its
header and in `commandFor`) is wrong in every `--app` build. Nothing reports an
error. The keys do nothing (Linux, Windows), or a stray three-byte character goes
to a decoder that throws it away (macOS).

**The single correct behavior a fix produces:** one special key produces the
same bytes on every surface and platform, whether console or `--app`, and whether
Console, TUI or Canvas mode, so one decoder handles all of them. Ideally there is
also one `io` call that returns the key already named, so no program has to
parse escape sequences.

References:

- `src/docs/spec/app/03_console-io.md` §"Input modes (line vs raw)": raw mode
  promises "each key's bytes immediately" but does not define what the bytes of a
  non-character key are. This is the missing contract.
- Found while designing `examples/water`: its left/right/up/down rotate/scale
  controls and Delete-to-remove need these keys in a canvas app.

## Failing Reproduction

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

A console build is immune because the *terminal emulator* does the encoding
before the bytes reach fd 0. Every `--app` surface replaces the terminal emulator
with our own key handler, and none of those handlers took over that job.

## Goal

- On every `--app` surface (Canvas, TUI/TermView raw, and the transcript in raw
  read) on macOS, Linux GTK and Windows, these keys write the xterm sequence a
  console build receives:

  | Key | Bytes |
  | --- | --- |
  | Up / Down / Right / Left | `ESC [ A` / `B` / `C` / `D` |
  | Home / End | `ESC [ H` / `ESC [ F` |
  | Insert / Delete (forward) | `ESC [ 2 ~` / `ESC [ 3 ~` |
  | Page Up / Page Down | `ESC [ 5 ~` / `ESC [ 6 ~` |
  | F1–F4 | `ESC O P` / `Q` / `R` / `S` |
  | F5–F12 | `ESC [ 15~ 17~ 18~ 19~ 20~ 21~ 23~ 24~` |
  | Backspace | `0x7F` on every platform |

- The macOS probe above prints the Expected line. The same probe on Linux and
  Windows prints the same bytes.
- `examples/wind`'s arrow keys work in its `--app` build (manual check on
  macOS).
- `src/docs/spec/app/03_console-io.md` states this table as the raw-mode key
  contract, with citations to each platform's translator.

### Non-goals (must NOT change)

- Printable characters, Enter→LF, Esc, Tab: unchanged bytes on every surface.
- Line-mode (`io::readLine`, `io::input`) editing: Backspace still erases one code
  point. Arrows in line mode stay ignored. This bug does not add line editing.
- Console builds: nothing in the non-app `io::readChar`/`readByte` path changes.
  The terminal already sends the target encoding.
- Mouse reporting (`term::pollMouse`, `canvas::pollMouse`) is a separate channel
  and stays untouched.
- **Tempting wrong fix:** teaching each example's `readKey` to also recognise
  U+F700 and friends. That fixes one program on one platform, leaves Linux and
  Windows dropping the keys, and makes every future program repeat the work.
  Forbidden.

## Blast Radius

Found with `grep -rlE 'u\{1B\}|"UP"' examples` and a search for key handlers
under `src/target`:

- `src/target/macos_aarch64/app/bootstrap.rs:emit_canvas_key_down_helper`: fixed by this bug.
- `src/target/macos_aarch64/app/term_view.rs` (`macapp.term.keyDown`, raw arm): fixed by this bug.
- `src/target/macos_aarch64/app` transcript `MFBTextView keyDown:` (raw read in
  Console mode): fixed by this bug, same translation. Phase 1 confirms whether it
  forwards `chars` verbatim like the other two.
- `src/target/linux_gtk/bootstrap.rs` key-press `raw` arm: fixed by this bug. The
  line-mode arm stays as it is (Non-goals).
- `src/target/win_x86_64/app/mod.rs:emit_wndproc` `wnd_check_char` (canvas) and
  `editproc` (transcript): fixed by this bug, by adding a `WM_KEYDOWN` arm that
  writes the sequence for the listed `VK_*` codes.
- **Sibling defect on the same Windows arms, fixed here too:** both write
  `wParam & 255`, the *low byte of a UTF-16 unit*. So any non-ASCII key (`é` =
  U+00E9) goes into the pipe as the lone byte `0xE9`, which is invalid UTF-8
  and makes `io::readChar` raise `ErrEncoding`. Found by reading the code
  (`and_registers … 255`, "low byte"), not measured. Phase 1 adds a test.
  The fix encodes the UTF-16 unit (and surrogate pairs) as UTF-8.
- `examples/snake`, `dungeon`, `wind`, `browser`, `bugs/canvas`, `bugs/tui`: each
  hand-rolls an `ESC [ A–D` decoder. They are unaffected by the byte-level fix,
  which is what makes them start working in `--app`. If the Phase 3 `io::readKey`
  lands, they migrate to it (see Open Decisions).
- `linux_x86_64` / `linux_aarch64` / `linux_riscv64` console targets: unaffected.
  They read fd 0 from a real terminal.

## Fix Design

**Layer 1: normalise at the source (required).** Each platform's key handler
translates special keys into the xterm sequences in the Goal table before writing
to the input pipe. This matches what a terminal does, so no `io` code and no
existing program changes. The console and `--app` builds become byte-identical
for the same keystroke.

- macOS: in the raw arms, look at `characterAtIndex:0` (the code is already loaded
  as `LOCAL[4]`/`LOCAL[5]`). If it is in the `NS*FunctionKey` range, look it up
  in a small shared table (`U+F700→ESC[A`, …) and write the sequence instead
  of `chars`. Put the table and the writer in one shared helper, called from all
  three macOS handlers, so they cannot drift apart.
- Linux: before the `gdk_keyval_to_unicode` call in the `raw` arm, compare
  `keyval` against the `GDK_KEY_*` constants and write the sequence. Map
  `GDK_KEY_Delete` explicitly to `ESC[3~`, because `gdk_keyval_to_unicode`
  would otherwise return `0x7F` and it would look like Backspace.
- Windows: add `WM_KEYDOWN` arms (canvas `WndProc` and `editproc`) that map
  `VK_UP…VK_F12` and `VK_DELETE` to the sequence and return 0. Map `VK_BACK`'s
  `WM_CHAR` (`0x08`) to `0x7F` so Backspace matches the other platforms.

**Layer 2: `io::readKey` (recommended, see Open Decisions).** A new
`io::readKey() AS io::Key` that reads one keystroke and decodes the sequences
above into a named value, for example a `Key` record `{ name AS io::KeyName, text AS String }`
with `KeyName.Char | Up | Down | Left | Right | Home | End | PageUp | PageDown |
Insert | Delete | Backspace | Enter | Tab | Escape | F1…F12 | EndOfInput`. It
needs a short timeout to tell a lone Esc from the start of a sequence. Every
example currently does that with `pollInput(20)`, and that logic moves into
one runtime helper. Layer 2 is only correct because Layer 1 exists: it decodes
one encoding, not four.

Rejected alternatives:

- *Decode the platform codes in `io::readChar`.* Rejected because `readByte`
  would still see the raw platform bytes, and Linux/Windows would still drop
  the keys before `io` sees anything.
- *Introduce a private encoding (e.g. U+E000+n per key).* Rejected because it
  breaks byte identity with console builds, so every program would need two
  decoders.

Expected output shift: none in existing goldens. No test currently feeds a
special key to an `--app` build. Any golden that shifts is a bug-hunt trigger.

## Phases

### Phase 1: failing tests and audit (no behavior change)

- [ ] Add `tests/app_special_keys.rs`. Follow the pattern of the existing
      emitted-helper tests (`src/target/linux_gtk/bootstrap.rs` has
      `bug-421` label/sequence tests). For each platform handler, assert that the
      emitted code contains the translation for Left, Delete and F5. They fail
      today.
- [ ] Add a Windows test that a non-ASCII `WM_CHAR` is written as UTF-8, not
      as a low byte. It fails today.
- [ ] Confirm the macOS transcript `keyDown:` raw path's behavior and record it in
      Blast Radius.
- [ ] Record the Linux and Windows rows of the matrix by running `/tmp/keyprobe` on
      the remote machines (`.ai/remote_systems.md`), using `xdotool key Left` or
      `SendInput` scripts, in place of the "code read" entries.

Acceptance: the new tests fail for the documented reason, and the matrix is
measured on all three platforms.
Commit: —

### Phase 2: Layer 1 translation

- [ ] Shared macOS translator helper, called from the canvas, TermView and transcript `keyDown:` raw arms.
- [ ] Linux GTK `raw` arm: keyval → sequence before `gdk_keyval_to_unicode`.
- [ ] Windows: `WM_KEYDOWN` arms in `emit_wndproc` and `editproc`, Backspace → `0x7F`, UTF-16 → UTF-8.
- [ ] Spec: add the key-encoding table to `src/docs/spec/app/03_console-io.md`
      with `[[path:Symbol]]` citations for all three translators.
- [ ] Man: `io readChar` / `io readByte` say that special keys arrive as the
      standard terminal sequences in every build, in developer terms (no internals).

Acceptance: the Phase 1 tests pass, the macOS probe prints the Expected line, and
the Linux and Windows probes match it.
Commit: —

### Phase 3: `io::readKey` (only if the Open Decision says yes)

- [ ] `io::readKey` + `io::Key`/`io::KeyName` registry descriptors and lowering,
      with the Esc-vs-sequence timeout in the runtime helper.
- [ ] Function tests under the project's validation/function-test conventions
      (`.ai/compiler.md`), fed by piped stdin sequences.
- [ ] Man page (`mfb man io readKey`, examples run by
      `scripts/man-run-examples.sh io --run`). Spec entry in `03_console-io.md`.
- [ ] Migrate `snake`, `dungeon`, `wind`, `browser`, `bugs/canvas` and
      `bugs/tui` from hand-rolled decoders to `io::readKey`.

Acceptance: every migrated example builds, and its arrow keys work in a console
build and in a macOS `--app` build.
Commit: —

### Phase 4: full validation

- [ ] Full suite (`cargo test`), goldens unchanged.
- [ ] `scripts/man-census.sh --memory-scope` reports 0 unclassified hits.
      `cargo test --bin mfb spec` passes.
- [ ] Re-run the probe on macOS, Linux and Windows. Manually check the arrow keys
      in `wind`'s `--app` build.

Acceptance: full suite green, and the probe prints the Expected line on all three
platforms.
Commit: —

## Validation Plan

- Regression tests: `tests/app_special_keys.rs` (emitted translation per
  platform and Windows UTF-8), plus `io::readKey` function tests if Phase 3 lands.
- Runtime proof: the `/tmp/keyprobe` program above, driven by `osascript`
  (macOS), `xdotool` (Linux) and a `SendInput` script (Windows), prints the
  Expected byte line.
- Doc sync: `src/docs/spec/app/03_console-io.md` (key contract table),
  `mfb man io readChar`/`readByte` (and `readKey`).
- Full suite: `cargo test`, `cargo test --bin mfb spec`.

## Open Decisions

- **Add `io::readKey` (Layer 2)?** Recommended: yes. Layer 1 alone standardises
  the bytes, but every program still needs a hand-written decoder with an
  Esc timeout, and six examples already carry copies of one. The alternative is
  Layer 1 only, leaving decoding to programs.
- **Modifier-qualified keys** (Shift+Arrow = `ESC[1;2A`, Ctrl+Arrow, …).
  Recommended: out of scope for this bug. Emit the unmodified sequence, and
  file a follow-up if a program needs modifiers.

## Summary

The engineering risk is in the Windows arms. They need a new `WM_KEYDOWN`
path in hand-emitted x86-64, a UTF-16→UTF-8 encoder in a `WndProc` frame whose
alignment is load-bearing (`≡ 8 mod 16`), and the Backspace byte change. The
macOS and Linux changes are table lookups placed in front of an existing write.
Console builds, line-mode editing and mouse input are untouched.
