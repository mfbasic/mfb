# MFBASIC `term::` Built-in Module Plan

Last updated: 2026-06-22

This document proposes a new `term::` built-in module that gives MFBASIC
programs structured terminal / TUI control: cursor movement, colors, text
attributes, screen clearing, and a full-screen "TUI mode" toggle. It targets two
runtime backends from one language surface:

- **Console mode** — ordinary native executables emit ANSI escape sequences to
  the terminal.
- **macOS app mode** (`mfb build -app`) — drives a custom `TermView : NSView`
  grid surface **synthesized in codegen** (no Objective-C source file, no host
  toolchain), the same way `MFBTextView : NSTextView` is synthesized today
  (`src/target/macos_aarch64/app.rs:443`).

This is a planning document. It describes intended behavior, required compiler
and runtime changes, open design points, validation strategy, and a recommended
implementation sequence. It is a sibling of the app-mode plans and shares their
structure deliberately.

It complements:

- `specifications/plan-05-linux-app.md`
- `specifications/mfbasic.md`
- `specifications/architecture.md`
- `specifications/memory_layouts.md`
- `specifications/error_codes.md`

## 1. Summary

MFBASIC currently has no way to address the terminal as a 2-D surface. The
`io::*` module is a linear stream: print, write, read, and a few interactive
queries including `io::terminalSize()` (`src/builtins/io.rs:19`,
`src/builtins/io.rs:76`). Real terminal programs — editors, dashboards, games,
menus — need cursor positioning, color, and attribute control over a cleared
screen.

This plan adds a `term::` module with this surface:

```text
term::on()                                 enter TUI mode (resets state)
term::off()                                leave TUI mode, restore normal screen
term::isOn() AS Boolean                    is TUI mode currently active?
term::setForeground(r, g, b)               set current foreground (24-bit)
term::setBackground(r, g, b)               set current background (24-bit)
term::setBold(enabled)                     set current bold attribute
term::setUnderline(enabled)                set current underline attribute
term::showCursor()                         make the cursor visible
term::hideCursor()                         make the cursor invisible
term::clear()                              clear surface, home the cursor
term::moveTo(row, column)                  move the cursor (0-based)
term::getForeground() AS TermColor         read current foreground
term::getBackground() AS TermColor         read current background
term::getBold() AS Boolean                 read current bold attribute
term::getUnderline() AS Boolean            read current underline attribute
term::terminalSize() AS TermSize           visible size in character cells
```

**`term::on()` is the gate.** Every `term::*` call other than `term::on()` and
`term::isOn()` is a **no-op while TUI mode is off** (§4.2.1): setters change
nothing and emit nothing, surface calls do nothing, and getters return the inert
default state. This makes `term::` safe to call unconditionally and prevents
stray escape sequences from leaking into normal output. `term::on()` enters TUI
mode and **resets all state to defaults** (foreground white `255,255,255`,
background black `0,0,0`, bold off, underline off, cursor at row 0 / column 0,
cursor visible).

`io::*` continues to work while `term::` is active. After `term::on()`, ordinary
`io::write` / `io::print` output is rendered into the TUI surface at the cursor
using the current attributes, honoring `\n`, `\r`, and `\t`, and scrolling at the
bottom. `term::off()` returns to ordinary streaming output.

Two new built-in record types are introduced: `TermColor` (`r`, `g`, `b` as
`Byte`) and `TermSize` (`columns`, `rows` as `Integer`). `io::terminalSize()` and
its `TerminalSize` type are **removed** and replaced by `term::terminalSize()`
returning `TermSize` (see §8).

### 1.1 Why synthesize `TermView` in codegen

mfb's macOS backend has no Objective-C compilation step. AppKit usage is emitted
as raw `objc_msgSend` calls, and custom classes are built at runtime with
`objc_allocateClassPair` / `class_addMethod` / `objc_registerClassPair`; method
bodies are emitted as ordinary `CodeFunction`s installed as IMPs. `MFBTextView`
is already built exactly this way (`src/target/macos_aarch64/app.rs:443-461`,
`emit_key_down_helper` at `app.rs:1030`).

`TermView` follows that established template: `TermView : NSView` with a
`calloc`'d cell grid, a `drawRect:` IMP that paints each cell, and helper IMPs
(or direct ivar pokes from the `term::` runtime helpers) for glyph writing,
attribute changes, cursor movement, and clearing. This keeps the "compiler emits
and links everything, no host toolchain" architecture intact and reuses the
synthesis, selector-registration, and associated-object machinery already
present in `app.rs`.

## 2. User-Facing Goal

A program importing `term` can build a full-screen terminal UI that works
identically (modulo backend) as a console executable and as a macOS app:

```basic
IMPORT io
IMPORT term

SUB main()
  term::on()
  term::clear()
  term::setForeground(0, 255, 0)
  term::moveTo(2, 4)
  io::write("Hello, TUI")
  term::setBold(TRUE)
  term::moveTo(4, 4)
  io::write("Bold line")
  LET size AS TermSize = term::terminalSize()
  term::moveTo(6, 4)
  io::write("Size: " & toString(size.columns) & "x" & toString(size.rows))
  LET key AS String = io::readChar()
  term::off()
END SUB
```

Expected experience:

- **Console build**: the screen switches to an alternate buffer, the text is
  positioned and colored, and `term::off()` restores the prior shell screen.
- **App build**: the window's content view becomes the `TermView` grid, the same
  text is painted, and `term::off()` restores the transcript view.

## 3. Non-Goals

- No box-drawing/widget/layout library; `term::` is a primitive cell surface.
- No mouse input, no scrollback API, no terminfo capability database. The console
  backend assumes an ANSI/`xterm`-class terminal (24-bit color, alt screen).
- No new key-event API. Input continues through `io::readChar` / `io::readByte` /
  `io::readLine` / `io::pollInput`. (Raw-key behavior is an `io::` concern, not a
  `term::` one.)
- No Linux app-mode `term::` surface in v1. Linux app mode (GTK4,
  `plan-05-linux-app.md`) is not yet shipped; the `term::` console backend works
  on Linux executables. A GTK `term::` surface is deferred (§12).
- No change to non-`term::` `io::` behavior while TUI mode is off.

## 4. Proposed Semantics

`term::` defines a small explicit state machine plus a current-attribute set,
both held in a writable runtime global (the "term state", §6.2). Every function
returns a `Result` consistent with the existing built-in calling convention
(`Result{tag, value}` records).

### 4.1 Types

```text
TYPE TermColor      ' built-in, not user-declared
  r AS Byte
  g AS Byte
  b AS Byte
END TYPE

TYPE TermSize       ' built-in, replaces TerminalSize
  columns AS Integer
  rows AS Integer
END TYPE
```

Both are built-in record types resolved through `builtin_type_fields`
(`src/builtins/io.rs:51` is the pattern). Memory layout follows
`specifications/memory_layouts.md`: consecutive fields, arena-allocated, returned
as a pointer inside the `Result` value slot. `TermColor` packs three `Byte`
fields; `TermSize` two `Integer` fields (mirroring the current `TerminalSize`
layout so the migration in §8 is a rename plus field set).

### 4.2 Mode control

#### `term::on() AS Nothing`

First **resets all term state to defaults** (foreground `255,255,255`, background
`0,0,0`, bold off, underline off, cursor at `0,0`, cursor visible), then enters
TUI mode. The reset happens every time `on()` is called, including when already
active.

- **Console**: switch to the alternate screen buffer (`\e[?1049h`), apply the
  default SGR (`\e[0m` then default fg/bg), clear it, home the cursor, and show
  the cursor (the reset default; programs call `term::hideCursor()` to hide it).
  Sets `term_active = true` in the term state.
- **App**: ensure the `TermView` exists, size its grid to the window, reset its
  current-attribute ivars to the defaults, install it as the window content view
  (replacing the transcript scroll view), clear it, and route subsequent `io::*`
  output to it. "In app mode it just clears the screen" — there is no alternate
  buffer; the window *is* the surface.
- Idempotent in the sense that calling `on()` while already active re-resets state
  and clears the surface.

#### `term::off() AS Nothing`

- **Console**: show the cursor (`\e[?25h`), leave the alternate buffer
  (`\e[?1049l`), reset SGR (`\e[0m`). The prior shell screen is restored. Sets
  `term_active = false`.
- **App**: restore the transcript scroll view as content view and re-route
  `io::*` to the transcript. "In app mode it just clears the screen" — the
  `TermView` grid is cleared on the way out.
- No-op when already off.

On normal program completion, the runtime must auto-`off()` if still active so a
console is never left in the alternate buffer (§6.5).

#### `term::isOn() AS Boolean`

Returns `term_active`. This is one of only two calls (with `term::on()`) that is
**not** gated by §4.2.1 — it always returns the true current mode, even while off.

#### 4.2.1 No-op while inactive

When `term_active` is false, every `term::*` call **except `term::on()` and
`term::isOn()`** is inert and returns success without effect:

- setters (`setForeground`, `setBackground`, `setBold`, `setUnderline`) change
  nothing and emit no bytes;
- surface calls (`clear`, `moveTo`, `showCursor`, `hideCursor`) do nothing;
- getters (`getForeground`, `getBackground`, `getBold`, `getUnderline`) return the
  **inert default** (white fg, black bg, bold off, underline off) — they never
  reflect a value a no-op setter "set";
- `terminalSize()` is the one read that has no inert value: while off it returns
  `ERR_UNSUPPORTED_OPERATION` ("term mode is not active"). Programs query size
  after `term::on()`.

This guarantees a program can call `term::*` freely without first checking mode,
and nothing leaks into normal `io::*` output when TUI mode was never entered. The
gate is a single `term_active` check at the top of each helper (both backends).

### 4.3 Attribute setters

All setters update the term state and, in console mode, emit the corresponding
SGR immediately so subsequent `io::*` writes are styled. In app mode they update
the `TermView` "current attribute" ivars; cells inherit them when written.

- `term::setForeground(r AS Byte, g AS Byte, b AS Byte) AS Nothing` — console:
  `\e[38;2;r;g;bm`.
- `term::setBackground(r AS Byte, g AS Byte, b AS Byte) AS Nothing` — console:
  `\e[48;2;r;g;bm`.
- `term::setBold(enabled AS Boolean) AS Nothing` — console: `\e[1m` / `\e[22m`.
- `term::setUnderline(enabled AS Boolean) AS Nothing` — console: `\e[4m` /
  `\e[24m`.

(All setters are no-ops while inactive, §4.2.1.)

### 4.4 Cursor visibility

- `term::showCursor() AS Nothing` — make the hardware/rendered cursor visible.
  Console: `\e[?25h`. App: set the `TermView` cursor-visible ivar and redraw.
- `term::hideCursor() AS Nothing` — hide it. Console: `\e[?25l`. App: clear the
  ivar and redraw.

`term::on()` resets visibility to **visible**; programs call `hideCursor()` for a
classic full-screen look. Both are no-ops while inactive (§4.2.1).

### 4.5 Surface control

- `term::clear() AS Nothing` — clear the whole surface to the current background,
  home the cursor. Console: `\e[2J\e[H`. App: zero every cell glyph to `" "` with
  current attributes and reset the cursor (the `TermView clear` path).
- `term::moveTo(row AS Integer, column AS Integer) AS Nothing` — move the cursor,
  0-based, clamped to `[0, rows-1] × [0, cols-1]`. Console: `\e[(row+1);(col+1)H`
  (ANSI is 1-based). App: set the `TermView` cursor ivars.

### 4.6 Attribute getters

Getters read the term state (the single source of truth) — a real terminal
cannot be portably queried, so the runtime tracks what it last set. While inactive
they return the inert default (§4.2.1).

- `term::getForeground() AS TermColor`
- `term::getBackground() AS TermColor`
- `term::getBold() AS Boolean`
- `term::getUnderline() AS Boolean`

### 4.7 `term::terminalSize() AS TermSize`

Visible size in character cells. Requires TUI mode; while inactive it returns
`ERR_UNSUPPORTED_OPERATION` ("term mode is not active"), per §4.2.1.

- **Console**: the `TIOCGWINSZ` ioctl path that backs today's
  `io::terminalSize()` (reused — see §8). Returns `ERR_UNSUPPORTED_OPERATION`
  when not a TTY, as today.
- **App**: computed from the `TermView` grid (`rows`/`cols` ivars), which are
  derived from the view bounds and monospaced font metrics — the same computation
  shape as `emit_app_io_terminal_size_helper` (`app.rs:1420`) but reading the
  grid the view already maintains rather than the transcript scroll view.

### 4.8 `io::*` interaction while TUI mode is active

When `term_active` is true:

- `io::write(text)` / `io::print(text)` render into the surface at the cursor
  using the current attributes, advancing the cursor, wrapping at the right edge,
  and scrolling at the bottom. `\n` newline, `\r` carriage return, and `\t`
  tab-to-next-4 are honored (the `TermView write:` semantics from the reference
  `.m`; the console backend gets the same behavior from the terminal itself).
- `io::printError` / `io::writeError`: same surface in app mode; console keeps
  writing to fd 2 (which the terminal interleaves).
- Input (`io::readLine` / `readChar` / `readByte` / `pollInput`) is unchanged.

This is the one place `term::` changes `io::` behavior, and only the app backend
needs explicit routing work — the console terminal does cursor/attribute
rendering itself.

## 5. CLI / Surface Contract

No new CLI flag. `term::` is an ordinary importable module like `io`. It is valid
in both console and `-app` builds. The backend is selected by
`(target, build_mode)` during lowering exactly as `io::*` already is
(`src/target/shared/code/mod.rs:2793` dispatches app vs non-app for
`io.terminalSize`).

Registration touch points (mirror `io`):

- `src/builtins/mod.rs:17` — add `"term"` to `is_builtin_import`.
- `src/builtins/mod.rs:21` — add `term::is_builtin_type` to `is_builtin_type`.
- `src/builtins/mod.rs:42` — add `term::call_return_type_name`.
- `src/builtins/mod.rs:53` — add `term::is_term_call` to `is_builtin_call`.
- `src/builtins/mod.rs:70` — add `term::call_param_names`.
- `src/builtins/mod.rs:1` — `pub(crate) mod term;`.

## 6. Runtime Architecture

### 6.1 Two backends behind one helper boundary

As with `io::*`, keep backend behavior behind distinct runtime helper symbols and
select by `(target, build_mode, call)` in shared lowering; do not branch inside
helper bodies. Helper symbol naming follows the existing scheme
(`runtime::symbol_for_call`, e.g. `_mfb_rt_term_term_setForeground`).

```text
console backend   ->  emit ANSI bytes to fd 1 + update term state global
macOS app backend ->  drive synthesized TermView + update term state global
```

The console backend is almost entirely target-independent (the same ANSI on
macOS and Linux), so it can live in shared code lowering parameterized by the
platform's stdout-write primitive, like the existing console `io::*` helpers.

### 6.2 Term state global (writable shared state)

Introduce one writable runtime global holding term state. This is the
"writable shared-state global" already flagged as the prerequisite for app-mode
window state (`[[macos-app-mode-progress]]`). Shape (conceptual):

```c
typedef struct {
    uint64_t active;          // TUI mode on/off — the §4.2.1 gate
    uint8_t  fg_r, fg_g, fg_b;
    uint8_t  bg_r, bg_g, bg_b;
    uint64_t bold;
    uint64_t underline;
    uint64_t cursor_visible;
    // app backend only:
    void    *term_view;       // TermView* (also stashed as NSApp associated object)
} MfbTermState;
```

`term::on()` resets every field above to its default (active=1, fg=white,
bg=black, bold=0, underline=0, cursor_visible=1) as a single state write. Each
non-`on`/`isOn` helper begins with `if (!state.active) return ok_inert;` — the
§4.2.1 gate — so the no-op behavior lives in one place per helper rather than in
the language. Both backends read/write this; the getters always read it. In app mode the
`TermView`'s own ivars are the authoritative cursor/grid store; the global mirrors
the current attributes so getters are backend-uniform. Access is single-threaded
in practice (the language worker thread), but app-mode `TermView` mutation must be
marshaled to the main thread (§6.4).

### 6.3 Synthesized `TermView` (app backend)

Built once, lazily on first `term::on()` (or at app bootstrap), using the
`MFBTextView` synthesis as the template (`app.rs:443-461`):

```text
cls = objc_allocateClassPair(NSView, "TermView", extraBytes)
class_addIvar(cls, "_cells", ...)        // TermCell* grid
class_addIvar(cls, "_rows"/"_cols"/...)  // metrics + cursor + current attrs
class_addMethod(cls, @selector(drawRect:),  imp_drawRect,  "v@:{CGRect=...}")
class_addMethod(cls, @selector(isFlipped),  imp_isFlipped, "c@:")
objc_registerClassPair(cls)
```

`TermCell` mirrors the reference `.m`: glyph (`__unsafe_unretained NSString*`),
fg/bg bytes, bold, underline. Grid (re)allocation, cursor advance, scroll-up
(`memmove`), and clear are the reference algorithms, emitted as helper functions
the `term::` runtime helpers call directly (they can poke ivars via known offsets
rather than going through accessor IMPs, since the same compiler emits both
sides).

The expensive, must-get-right IMP is `drawRect:`: fill background, then per
visible cell fill the cell background rect, build a font+color attribute
dictionary, and draw the glyph at the baseline. This is the largest block of
hand-emitted `objc_msgSend` codegen in the feature and the main risk (§9).

**Content-view swap**: `term::on()` calls `setContentView:` with the `TermView`;
`term::off()` restores the transcript scroll view captured at bootstrap. Both
must run on the main thread; mutation that affects display calls
`setNeedsDisplay:`.

### 6.4 App-mode threading

The language program runs on the worker thread; AppKit requires main-thread UI
mutation (`app.rs` already establishes this split and the
`performSelectorOnMainThread:` marshaling used by the transcript append helper at
`app.rs:835`). Every `term::` operation that mutates the `TermView` or swaps the
content view must marshal to the main thread the same way. Pure state-global
updates (attribute setters' bookkeeping, getters) need no marshaling, but the
visible effect (SGR-equivalent ivar change + redraw) does.

For ordering correctness, an `io::write` into the surface and a following
`term::moveTo` must apply in program order on the main thread; marshal them onto
the same serial main-thread path the transcript append already uses so writes and
cursor moves don't reorder.

### 6.5 Auto-restore on exit

The program-finish path (`emit_finish_helper`, `app.rs:875`; the console exit
path) must call `term::off()` semantics if `term_active` is still true, so a
console is never left on the alternate screen and the app window returns to a
sane content view before showing the completion status line.

## 7. Compiler and Backend Changes

### 7.1 Built-in module (`src/builtins/term.rs`, new)

Mirror `src/builtins/io.rs`:

- call-name constants for all sixteen calls (`term.on`, `term.off`, `term.isOn`,
  `term.setForeground`, `term.setBackground`, `term.setBold`,
  `term.setUnderline`, `term.showCursor`, `term.hideCursor`, `term.clear`,
  `term.moveTo`, `term.getForeground`, `term.getBackground`, `term.getBold`,
  `term.getUnderline`, `term.terminalSize`)
- `is_term_call`, `is_builtin_type`, `builtin_type_fields` (`TermColor`,
  `TermSize`), `call_param_names`, `call_return_type_name`, `resolve_call`,
  `expected_arguments`, `arity`.

Argument typing: `setForeground`/`setBackground` take `(Byte, Byte, Byte)`;
`setBold`/`setUnderline` take `(Boolean)`; `moveTo` takes `(Integer, Integer)`;
the rest take no arguments. Returns: `isOn`/`getBold`/`getUnderline` →
`Boolean`; `getForeground`/`getBackground` → `TermColor`; `terminalSize` →
`TermSize`; everything else → `Nothing`.

The §4.2.1 no-op-while-inactive rule is a **runtime** concern (the `state.active`
gate in each helper), not a typecheck one — typing and arity are unconditional.

### 7.2 Typecheck (`src/typecheck.rs`)

Add `check_term_builtin_call` mirroring `check_io_builtin_call`
(`typecheck.rs:4748`), dispatched from the builtin-call router
(`typecheck.rs:4464`). `TermColor` / `TermSize` field access resolves through the
builtin-type-fields fallback already used for `TerminalSize`
(`typecheck.rs:~3870`).

### 7.3 Runtime helper specs (`src/target/shared/runtime.rs`)

Add a `RuntimeHelperSpec` per `term::` call (the `IO_TERMINAL_SIZE_SPEC` block at
`runtime.rs:366` is the template), with appropriate `params`, `returns`, and
`clobbers`. A new `RuntimeHelper::Term` variant groups them.

### 7.4 Shared lowering / dispatch (`src/target/shared/code/mod.rs`)

- Add a `CodegenPlatform` trait method per app-backed `term::` helper (the trait
  is at `code/mod.rs:293`; `emit_app_io_terminal_size_helper` at `code/mod.rs:600`
  is the pattern), defaulting to `None` so non-macOS targets fall through.
- Add dispatch arms (`code/mod.rs:2793` is the `io.terminalSize` app/non-app
  split) that select the app helper when `app_mode` else the console ANSI helper.
- Implement the console ANSI lowerers in shared code (target-independent bytes +
  stdout write + state-global update).

### 7.5 macOS app backend (`src/target/macos_aarch64/app.rs`, `…/mod.rs`)

- `TermView` synthesis + `drawRect:` / grid helpers (§6.3).
- One `emit_app_term_*` helper per call; register the platform trait impls in
  `src/target/macos_aarch64/mod.rs`.
- Content-view swap for `on`/`off`; `io::*` surface routing while active; auto-off
  on finish (`app.rs:875`).
- New selectors / class-name C-strings in the data-symbol block
  (`app.rs:1585`).

### 7.6 `io::terminalSize()` removal

See §8 — coordinated edit across `io.rs`, typecheck, runtime spec, the macOS app
helper, examples, and tests.

## 8. Removing `io::terminalSize()`

`io::terminalSize()` and its `TerminalSize` type are wired end-to-end and tested
(`builtins/io.rs:19,53,76`; `runtime.rs:366`; `emit_app_io_terminal_size_helper`
at `app.rs:1420`; plus the app-mode io coverage fixture from recent commits).
Replacement steps:

1. Remove `TERMINAL_SIZE` / `TERMINAL_SIZE_TYPE` from `src/builtins/io.rs`
   (constant, `is_io_call`, `is_builtin_type`, `builtin_type_fields`,
   `call_*`, `resolve_call`, `arity`).
2. Drop `io::terminalSize` from `is_builtin_type`/typecheck and the io runtime
   spec / dispatch.
3. **Reuse the implementations**: `term::terminalSize` console backend = the old
   `io::terminalSize` `TIOCGWINSZ` path; app backend = the old
   `emit_app_io_terminal_size_helper`, retargeted to read the `TermView` grid.
   Move/rename rather than rewrite.
4. `TermSize` replaces `TerminalSize` (same two-`Integer` `columns`/`rows`
   layout), so field access in existing programs only needs the call + import
   renamed.
5. Update examples and tests: anything referencing `io::terminalSize` /
   `TerminalSize`, and the app-mode io coverage fixture, move to `term::`.
6. Acceptance: regenerate goldens that mention the removed call/type.

Because this is a breaking removal, do it in its own phase after `term::` is
proven, so a bisect cleanly separates "added term" from "removed io API".

## 9. Risks

1. **`drawRect:` codegen.** The largest hand-emitted `objc_msgSend` block in the
   feature: nested cell loops, per-cell `NSRectFill`, attribute-dictionary
   construction, and `drawAtPoint:withAttributes:`. This is exactly the
   layout-sensitive, register-clobber-prone territory recorded in
   `[[macos-codegen-latent-bugs]]`, `[[copy-record-register-aliasing]]`, and
   `[[arena-alloc-clobbers-x14-x15]]`. Mitigate by keeping values that must
   survive `bl` in callee-saved registers, building the smallest correct
   `drawRect:` first (single-attribute, no underline), and growing it.
2. **Main-thread marshaling / ordering.** `io::write`-into-surface followed by
   `term::moveTo` must apply in program order on the main thread. Route both
   through the existing serial main-thread path; never mutate `TermView` from the
   worker thread.
3. **Auto-restore.** Forgetting auto-`off()` leaves a console on the alternate
   screen after a crash/exit. Wire it into the finish path and trap handler.
4. **Console assumptions.** 24-bit color + alt screen assume an xterm-class
   terminal; `term::` on a dumb terminal degrades. Acceptable for v1 (stated
   assumption), but `term::on()` should still no-op-safely (not corrupt output)
   when stdout is not a TTY.
5. **Breaking removal of `io::terminalSize`.** Coordinate §8 carefully; missing a
   test/example reference fails acceptance.
6. **Attribute/getter source of truth.** Two stores (state global vs `TermView`
   ivars) can drift. Make the global authoritative for attributes; the view
   mirrors on write.

## 10. Testing and Validation

### 10.1 Function test coverage

Each new `term::` function needs valid and invalid coverage under the repo
convention:

```text
tests/func_term_on_valid/**             tests/func_term_on_invalid/**
tests/func_term_off_valid/**            tests/func_term_off_invalid/**
tests/func_term_isOn_valid/**           tests/func_term_isOn_invalid/**
tests/func_term_setForeground_valid/**  tests/func_term_setForeground_invalid/**
tests/func_term_setBackground_valid/**  tests/func_term_setBackground_invalid/**
tests/func_term_setBold_valid/**        tests/func_term_setBold_invalid/**
tests/func_term_setUnderline_valid/**   tests/func_term_setUnderline_invalid/**
tests/func_term_showCursor_valid/**     tests/func_term_showCursor_invalid/**
tests/func_term_hideCursor_valid/**     tests/func_term_hideCursor_invalid/**
tests/func_term_clear_valid/**          tests/func_term_clear_invalid/**
tests/func_term_moveTo_valid/**         tests/func_term_moveTo_invalid/**
tests/func_term_getForeground_valid/**  tests/func_term_getForeground_invalid/**
tests/func_term_getBackground_valid/**  tests/func_term_getBackground_invalid/**
tests/func_term_getBold_valid/**        tests/func_term_getBold_invalid/**
tests/func_term_getUnderline_valid/**   tests/func_term_getUnderline_invalid/**
tests/func_term_terminalSize_valid/**   tests/func_term_terminalSize_invalid/**
```

Invalid cases: wrong arity, wrong argument types (e.g. `Integer` to
`setForeground`, `String` to `moveTo`), `term::` used without `IMPORT term`.

The §4.2.1 gate also needs **behavioral** coverage (not just per-function arity):
a test that calls setters/`clear`/`moveTo`/`showCursor`/`hideCursor` while off
asserts no bytes are emitted, getters return the inert default, `terminalSize()`
returns `ERR_UNSUPPORTED_OPERATION`, and `isOn()` returns `FALSE`; and a test that
`term::on()` resets state (set non-defaults, `off()`, `on()`, read defaults back).

### 10.2 Console backend runtime tests

Build a small `term::` console program and assert on the emitted byte stream
(deterministic and CI-friendly): drive stdout into a pipe and verify the exact
escape sequences (`\e[?1049h`, `\e[38;2;…m`, `\e[r;cH`, `\e[2J`, `\e[?1049l`).
Getters verify state round-trips. This needs no real TTY.

### 10.3 App backend runtime tests

Follow the app-mode io coverage approach already in the repo: a headless/offscreen
app-mode run that exercises the `term::` helpers and asserts on a runtime-
observable artifact (cell-grid snapshot or transcript-equivalent dump), plus at
least one manual real-window smoke recipe (text positioned, colored, bold; window
resize changes `terminalSize`; `off()` restores the transcript).

### 10.4 Acceptance

Run `scripts/test-accept.sh target/debug/mfb target/accept-actual`. Update
goldens for: new `term::` typecheck diagnostics, NIR/NPlan/NCode for the new
helpers, the macOS app object/code plans importing the new selectors, and the
`io::terminalSize` removal.

### 10.5 Manual checklist

1. Console: `term::on()` switches to alt screen; text is positioned/colored;
   `term::off()` restores the shell screen with scrollback intact.
2. Console: program crash/exit auto-restores the screen.
3. App: `term::on()` swaps content view to the grid; text renders with attributes;
   `term::off()` restores the transcript.
4. `io::write` after `moveTo` lands at the cursor with current attributes; newline
   wraps; bottom scrolls.
5. `term::terminalSize()` reflects the real size and updates on resize.
6. Getters return the last values set.

## 11. Recommended Implementation Sequence

### Phase 1: Module plumbing (both backends stubbed)
- `src/builtins/term.rs`; wire into `src/builtins/mod.rs`.
- `TermColor` / `TermSize` types; typecheck (`check_term_builtin_call`).
- Runtime specs + `RuntimeHelper::Term`; `CodegenPlatform` trait methods (default
  `None`); dispatch arms.
- Stub helpers (no-op returns) so the surface type-checks and links.

Deliverable: programs `IMPORT term` and compile; calls are recognized,
arity/types enforced; no runtime effect yet.

### Phase 2: Console (ANSI) backend
- Implement all `term::` helpers as ANSI emitters + term-state-global updates.
- Auto-`off()` on program finish / trap.
- `io::*` already renders correctly through the real terminal when active.

Deliverable: `term::` fully works for native console executables on macOS and
Linux. Byte-stream tests (§10.2) pass.

### Phase 3: `io::terminalSize()` removal / migration
- Execute §8. Move the TTY-size implementation under `term::terminalSize`.
- Update examples, fixtures, goldens.
- No backwards compat is needed.

Deliverable: single terminal-size API (`term::terminalSize`), acceptance green.

### Phase 4: macOS app `TermView` synthesis
- Synthesize `TermView : NSView`: ivars, grid alloc/copy/scroll/clear, cursor,
  current-attribute store.
- Implement `drawRect:` incrementally (smallest correct version first).
- Content-view swap for `on`/`off`; capture the transcript scroll view at
  bootstrap for restore.

Deliverable: an app build can enter TUI mode and paint static positioned, colored
text; `off()` restores the transcript.

### Phase 5: App `term::` helper wiring + `io::*` routing
- Implement each `emit_app_term_*` helper driving the `TermView` on the main
  thread.
- Route `io::write`/`io::print` into the surface while active (cursor advance,
  wrap, scroll, `\n`/`\r`/`\t`).
- App-mode `term::terminalSize` from the grid; auto-off on finish.

Deliverable: the §2 example runs identically in console and app builds.

### Phase 6: Validation
- Add the §10.1 function tests and §10.2–10.3 runtime tests.
- Full acceptance + manual smoke on both backends.

## 12. Open Design Decisions

1. **Spelling.** The request used `setForground` / `getForground`; this plan uses
   the correct `setForeground` / `getForeground`. Confirm the canonical spelling
   (and whether to keep the misspelled form as a deprecated alias, as the
   reference `.m` did).
   Confirm: `setForeground` / `getForeground`
2. **`moveTo` argument order.** This plan uses `(row, column)` (matching the
   reference `moveToRow:column:`). Confirm vs `(x, y)` / `(column, row)`.
   Confirm: `moveToRow:column:`
3. **Coordinate base.** 0-based (matching the reference `.m` clamping). Confirm.
   Confirm: 0-based
4. **Cursor visibility API.** Resolved: expose `term::showCursor` /
   `term::hideCursor` (§4.4). `term::on()` resets visibility to visible.
5. **`io::*` while active in console mode.** Output is interleaved by the terminal
   at the cursor (no routing needed). Confirm that is the desired model vs
   buffering through `term::`.
   Confirm: Confirm that is the desired model
6. **Non-TTY console behavior.** `term::on()` on a non-TTY stdout: no-op safely,
   or `ERR_UNSUPPORTED_OPERATION`? This plan recommends no-op-safe.
   Confirm: no-op safely
7. **Linux app-mode surface.** Defer the GTK `term::` surface until Linux app mode
   (`plan-05-linux-app.md`) ships; track as future work.
   Confirm: Defer
8. **Color model.** 24-bit truecolor only (`TermColor` is `r,g,b`). No 256-color
   or named-palette fallback in v1.
   Confirm: 24-bit truecolor only

## 13. Recommendation

Proceed with a single `term::` language surface over two backends:

- Console backend first (ANSI escape sequences + a writable term-state global) —
  small, portable, immediately useful on macOS and Linux executables.
- Then migrate terminal-size from `io::` to `term::` as a clean breaking change.
- Then synthesize `TermView : NSView` in codegen for macOS app mode, reusing the
  existing runtime class-synthesis machinery, with `drawRect:` built up
  incrementally to contain the codegen risk.

This keeps the no-host-toolchain architecture intact, reuses the `io::` built-in
and app-mode patterns wholesale, and delivers a coherent, testable TUI capability
in well-bounded phases.
