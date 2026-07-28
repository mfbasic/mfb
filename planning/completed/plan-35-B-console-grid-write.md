# plan-35-B: Console — route drawing + text into the shadow grid

Last updated: 2026-07-11
Effort: medium (1h–2h)
Depends on: plan-35-A

Give the **console** backend a retained cell grid. `term::on` allocates the D2
header block (back + front buffers); while active, console `io::write`/`io::print`
and the `term::` drawing calls (`moveTo`, `setForeground`, `setBackground`,
`setBold`, `setUnderline`, `clear`, `showCursor`, `hideCursor`) mutate the grid,
shadow cursor, and current-attribute set **instead of emitting ANSI**. So that
the result is observable before the diff renderer exists, `term::sync` in this
sub-plan presents with a **temporary full-repaint** (every cell, every call);
Phase C replaces that with the minimal diff. This sub-plan is the console mirror
of the `active`-gated grid routing app mode already ships
(`emit_app_io_write_helper`).

Shared design of record: `planning/plan-35-shadow-grid-unify.md` (§4.1 model, §4.2
storage, §4.4 term-aware `io::write`). Encodes D2 (arena header block, slot 48)
and D3 (`abi::` codegen).

## 1. Goal

- Console `term::on` allocates the arena header block
  `[rows, cols, cursorRow, cursorCol | back… | front…]` sized to
  `term::terminalSize()`, base pointer in reserved slot 48; `term::off` frees it
  and zeroes the slot; `_mfb_shutdown` frees it if `off` was skipped.
- While `TERM_STATE_ACTIVE != 0`, console `io::write`/`io::print` writes glyphs
  into the **back** buffer at the shadow cursor (unicode display-width aware,
  wrap at right edge, scroll at bottom) rather than calling `emit_write`.
- `moveTo`/`setColor`/`setAttr`/`clear`/`showCursor`/`hideCursor` mutate shadow
  state (cursor / current attrs / cells / cursorVisible) and emit no ANSI.
- `term::sync` presents via a temporary full-repaint so a drawn frame is visible;
  a program drawing then calling `sync()` shows correct content.

### Non-goals

- **No minimal diff yet** (that is Phase C) — the temporary full-repaint is
  explicitly allowed to be inefficient/flickery here.
- **No non-TUI change:** the `emit_write` fast path while TUI is off is untouched;
  byte-gate stays green.

## 2. Current State

Console term helpers are immediate ANSI with no grid or cursor
(`src/target/shared/code/term.rs`, plan-35 master §2.2). Console text output is
term-unaware: `lower_io_write_helper` (`src/target/shared/code/io_helpers.rs:262`)
stages fd/ptr/len and calls `platform.emit_write` (→ libc `write(2)`,
`src/target/macos_aarch64/code.rs:232`), with an optional per-arena stdout buffer
(`ARENA_OUT_ENABLED`). The reference to mirror: `emit_app_io_write_helper` (macOS
`app/app_io.rs:44-57`, GTK `app_io.rs:390-408`) loads `TERM_STATE_ACTIVE` and
routes to the grid writer when set. Grid write/wrap/scroll semantics to match:
macOS `mfbWriteString:` (`term_view.rs`), which handles `\n`/`\r`/`\t`, wraps at
`cols`, and scrolls when `cursor_row >= rows`. Arena alloc: `ARENA_ALLOC_SYMBOL`
(already used by `emit_get_color`/`emit_terminal_size` in `term.rs`). Unicode
display width: `src/unicode_backend.rs`.

## 3. Design

- **Allocation (`emit_on`).** After the existing state-defaults writes, call
  `ARENA_ALLOC_SYMBOL` for `32 + 2*rows*cols*CELL_SIZE` bytes (header 4×u64 +
  two cell arrays), where `rows`/`cols` come from the `terminalSize` ioctl
  (reuse the `emit_terminal_size` path). Zero-fill (calloc semantics = cleared
  grid). Store the base pointer at `term_state_offset + 48`. On alloc failure,
  fall back to `active=1` with a null grid pointer and the current immediate
  path? — No: instead surface the allocation error via the standard
  `ERR_OUT_OF_MEMORY` result (`term::on` already returns `Result`). Document that
  `term::on` can now fail on OOM.
- **Free (`emit_off`, `_mfb_shutdown`).** Free the block, zero slot 48. `off`
  frees after Phase C's final present (here: after the temp full-repaint).
- **Grid writer.** A shared helper (new, in `io_helpers.rs` or `term.rs`) taking
  the string object + current attrs + header pointer: iterate glyphs (UTF-8
  decode + display width via `unicode_backend`), handle `\n`/`\r`, write each
  cell `back[row*cols + col] = {glyph, curFg, curBg, curBold, curUnderline}`,
  advance/wrap/scroll the shadow cursor stored in the header.
- **term-aware `io::write`.** `lower_io_write_helper` gains a leading
  `TERM_STATE_ACTIVE` branch → grid writer; else the unchanged fd path.
- **Drawing calls.** Rewrite the `term.rs` arms to mutate shadow state:
  `moveTo` sets header cursorRow/Col (clamp ≥0); `setColor`/`setAttr` keep
  writing the global slots (now read as current attrs); `clear` zero-fills the
  back buffer + homes the cursor; `showCursor`/`hideCursor` set cursorVisible.
  None emit ANSI.
- **Temp present.** `term::sync` walks the back buffer and emits a full repaint
  (home, then every cell with SGR per cell), then `memcpy` back→front. Crude but
  correct; Phase C makes it minimal.

Risk: unicode width in the writer (wide glyphs consume two cells; zero-width
combine into the previous cell) and the scroll/wrap edge cases — validated by
`func_term_write_grid_*`.

## Phases

### Phase 1 — allocate/free the console grid on on/off

- [ ] `emit_on`/`emit_off` in `src/target/shared/code/term.rs`: size from
      `terminalSize`, `ARENA_ALLOC_SYMBOL` the header block, store ptr @ slot 48;
      free + zero on `off`; register a `_mfb_shutdown` free.
- [ ] Tests: a leak check — `term::on`/`off` in a loop under the acceptance
      harness shows no growth (mirror existing leak-sensitive func tests).

Acceptance: `term::on`/`off` allocates and frees the block with no leak; `on`
under simulated OOM returns `ERR_OUT_OF_MEMORY`. Commit: —

### Phase 2 — term-aware console `io::write` + grid writer

- [ ] Grid writer helper (`io_helpers.rs`/`term.rs`) with unicode-width iteration,
      wrap, scroll, `\n`/`\r` handling; writes cells + advances the header cursor.
- [ ] Leading `TERM_STATE_ACTIVE` branch in `lower_io_write_helper`
      (`io_helpers.rs:262`) → grid writer; unchanged fd path when off.
- [ ] Tests: `func_term_write_grid_valid` (cursor advance, right-edge wrap,
      bottom scroll, negative-clamp via `moveTo`), plus a wide-glyph case.

Acceptance: with the temp present, a program that `moveTo`+`io::write`s known text
shows the expected cells; a program that never calls `term::on` is byte-identical
(`scripts/artifact-gate.sh`). Commit: —

### Phase 3 — drawing calls mutate shadow state; temp full-repaint present

- [ ] Convert `moveTo`/`setForeground`/`setBackground`/`setBold`/`setUnderline`/
      `clear`/`showCursor`/`hideCursor` arms in `term.rs` to shadow-state mutation
      (no ANSI).
- [ ] Temporary full-repaint in the `term::sync` arm (every cell + SGR, then
      back→front copy).
- [ ] Tests: `func_term_draw_grid_valid` (color/attr applied to cells; `clear`
      blanks + homes; cursor visibility tracked).

Acceptance: a console frame drawn with color/bold/underline + `term::sync()`
renders correctly (full-repaint); no ANSI is emitted by any call other than
`sync`. Commit: —

## Validation Plan

- Tests: `func_term_write_grid_*`, `func_term_draw_grid_*`, leak check; full
  `func_term_*` suite; `tests/syntax/term/*`.
- Byte-gate: `scripts/artifact-gate.sh` (TUI-off byte-identical).
- Runtime proof: a small console program drawing a known grid, captured under a
  PTY, shows the expected cells after `sync()`.
- Acceptance: `scripts/test-accept.sh`; cross-target build (x86 + riscv) since the
  writer is neutral `abi::` codegen (D3).

## Summary

Console gains the retained grid: allocation on `on`, a term-aware `io::write`
mirroring app mode, and drawing calls that mutate shadow state. Correctness risk
is the unicode-width grid writer and scroll/wrap edges. The present is
deliberately a placeholder full-repaint — Phase C makes it minimal. Untouched:
TUI-off output (byte-gated) and the app backends.
