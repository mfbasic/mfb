# plan-35-C: Console — front/back diff presenter (the crux)

Last updated: 2026-07-11
Effort: medium (1h–2h)
Depends on: plan-35-B

Replace Phase B's temporary full-repaint with the real **front/back-buffer diff**:
`term::sync()` compares the back buffer to the last-presented front buffer and
emits only the changed cells (minimal cursor moves + coalesced SGR + glyphs),
then copies back→front. `term::off` runs the same present once (final frame)
before restoring the screen. This is where the plan's correctness risk
concentrates. Outcome: a full-screen program that repaints every frame shows no
flicker and, on the console, emits O(changed cells) bytes per steady frame.

Shared design of record: `planning/plan-35-shadow-grid-unify.md` (§4.3 present).
Encodes D1 (mandatory `sync`; `off` implies a final present) and D3 (`abi::`
codegen).

## 1. Goal

- `term::sync()` walks back vs front: skip unchanged cells; for each run of
  changed cells emit one CUP (`ESC[r;cH`, or a bare relative move when adjacent),
  switch SGR only when fg/bg/bold/underline differ from the last-emitted
  attributes (coalesced across the run), then the glyph bytes; then restore the
  shadow cursor position + visibility, flush stdout, `memcpy` back→front.
- First present after `on`/resize is a full repaint (front all-dirty). The
  alt-screen enter/clear stays in `term::on`.
- `term::off` presents the final frame, then emits the alt-screen restore.
- Steady-state: a frame changing K cells emits O(K) cells of output, not O(rows×cols).

### Non-goals

- No app-backend change (Phases D/E).
- No new `term::` surface; `sync`/`off` behavior only.

## 2. Current State

After Phase B: console has a back+front grid in the D2 header block (slot 48), a
term-aware `io::write`, drawing calls that mutate shadow state, and a **temporary
full-repaint** `term::sync`. `term::off` currently frees the block after that
temp present. The fixed ANSI byte strings and the decimal-formatting /
`emit_write_const` / `emit_write_decimal` helpers already exist in
`src/target/shared/code/term.rs` (CUP uses `ESC_BRACKET` … `;` … `ESC_LETTER_H`;
SGR truecolor uses `ESC_FG_PREFIX`/`ESC_BG_PREFIX` … `m`; bold/underline
on/off strings all present). The `life` status bar deliberately writes only
`cols-1` of the last row (`examples/life/src/main.mfb:243`) to avoid the
last-cell scroll — the diff must preserve that "don't touch the very last cell
unless it changed" property naturally by only emitting changed cells.

## 3. Design

The presenter is one pass over `row, col`:

- Track `lastRow`, `lastCol`, and `lastFg/lastBg/lastBold/lastUnderline`
  (the SGR state actually emitted so far, initialized "unknown" so the first
  changed cell forces an SGR).
- For each cell where `back != front`:
  - **Cursor:** if `(row,col)` is not where the terminal cursor is
    (`lastRow,lastCol` after the previous glyph), emit a CUP. Optimization: when
    on the same row and one column ahead, no move is needed (the prior glyph
    advanced the cursor); otherwise CUP. Keep it simple first (always CUP on a
    new run), optimize adjacency second.
  - **SGR:** if the cell's fg/bg/bold/underline differ from `last*`, emit the
    minimal SGR (only the changed facets; reuse `ESC_FG_PREFIX`/`ESC_BG_PREFIX`/
    bold/underline strings) and update `last*`.
  - **Glyph:** emit the glyph's UTF-8 bytes; set `lastCol = col + width`.
- After the pass: emit SGR reset if needed, move the cursor to the shadow cursor
  and apply cursorVisible (show/hide), flush the stdout buffer, `memcpy`
  back→front.
- **Full-repaint mode:** a `dirty` flag in the header set by `on`/resize forces
  every cell to be treated as changed (front is logically blank). `clear` sets
  every back cell blank — which the diff naturally renders as erases; no special
  case needed beyond blanks emitting a space at the right SGR bg.

`term::off`: call the present once (final frame), then `emit_write_const(ESC_OFF)`
and free the block (moved here from Phase B's `off`).

Risks: (1) SGR minimization correctness — emitting a glyph under stale attributes
paints the wrong color; the `last*` tracking must be exact. (2) blank-cell
erasure — a cell that went from a glyph to blank must emit a space at the correct
bg. (3) wide/zero-width glyphs — column accounting after emit. (4) the last cell
of the last row — only emitted when actually changed, so the scroll hazard is
avoided by construction; add a test that asserts it is not emitted on an
unchanged frame.

## Phases

### Phase 1 — diff renderer

- [ ] Implement the back-vs-front walk in the `term::sync` arm of
      `src/target/shared/code/term.rs`: run detection, CUP, SGR coalescing (via
      the existing escape-string + `emit_write_decimal` helpers), glyph emit,
      blank-as-space erase, cursor restore + visibility, stdout flush,
      `memcpy` back→front.
- [ ] Header `dirty` flag: set by `on`; a full present clears it.
- [ ] Remove the temporary full-repaint from Phase B.

Acceptance: a program drawing frame 1, `sync()`, mutating a few cells, `sync()`
again — the second `sync` emits only the changed cells (verified by capturing the
escape stream under a PTY). Commit: —

### Phase 2 — resize + `term::off` final present

- [ ] Resize handling: when `terminalSize` differs from the header dims at
      `sync` entry, realloc the block, set `dirty` (full repaint). (`life`
      re-queries size each loop.)
- [ ] Move the final present into `emit_off` (present, then `ESC_OFF`, then free).
- [ ] Tests: `func_term_diff_minimal_valid` (steady frame emits only changed
      cells; the untouched last-row-last-cell is NOT emitted), a resize case
      (full repaint), and an `off`-flushes-last-frame case.

Acceptance: resize forces a correct full repaint; `term::off` leaves the correct
final frame on screen then restores; the minimal-diff test asserts byte count
scales with changed cells. Commit: —

## Validation Plan

- Tests: `func_term_diff_minimal_valid`, resize + off-flush cases; full
  `func_term_*`.
- Runtime proof: `examples/life` on the console (after Phase F adds its `sync()`
  call) — capture the escape stream and confirm a steady frame emits
  O(changed cells); visually, no flicker.
- Byte-gate: `scripts/artifact-gate.sh` (TUI-off unchanged).
- Acceptance: `scripts/test-accept.sh`; cross-target (x86 + riscv via ssh 2229) —
  the presenter is neutral `abi::` codegen, so all ISAs share it and must stay
  byte-deterministic (bug-87).

## Summary

The console becomes truly double-buffered: `sync` emits a minimal diff, `off`
flushes the last frame. Risk is entirely in the diff's SGR/cursor/erase
correctness and the wide-glyph column accounting — all covered by the
minimal-diff and grid-write tests. Nothing outside the console `sync`/`off`
path changes.
