# bug-313: term-grid `present` out-buffer overflows the arena block on terminals with ≥4-digit coordinates

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Memory-safety

Status: Open
Regression Test: tests/ (new) — a grid sized ≥1000 rows/cols with many scattered dirty cells does not overflow the present buffer

The per-block out buffer is sized `rows*cols*OUTBUF_PER_CELL(64) + TRAILER_SLACK(64)`,
but the present loop can emit, per changed cell, a CUP + a full SGR + the glyph.
Worst-case bytes/cell = CUP (`\x1b[` + dec(row+1) + `;` + dec(col+1) + `H` = 2·d + 4,
where d = coordinate digit count) + SGR (bold/underline + `\x1b[38;2;r;g;b m` +
`\x1b[48;2;r;g;b m` ≈ 50) + glyph (≤4) = 2·d + 58. The budget is 64/cell, so any cell
with d ≥ 4 (row+1 or col+1 ≥ 1000) exceeds it. `rows`/`cols` come from raw
`TIOCGWINSZ` `ws_row`/`ws_col` loaded as u16 with no clamp (unlike the CLI, which
clamps 20..=1000), so d can be 5. In the scattered-diff case aggregate overflow occurs
once the total excess exceeds `TRAILER_SLACK` (~>20 cells at d=4, >10 at d=5), writing
past the arena grid block into adjacent arena memory.

The single correct behavior a fix produces: `term::sync()` never writes past the
present buffer regardless of terminal geometry or dirty-cell pattern.

References:

- `bugs/completed-bugs/bug-175-*` (item G added `TRAILER_SLACK` but assumed the 64/cell
  budget sufficient).
- Found during goal-06 review of `src/target/shared/code/term_grid.rs`.

## Failing Reproduction

A `term::` program under a PTY sized ≥1000 rows or ≥1000 columns (tmux/xterm with a
tiny font, or a programmatically-sized pty) that dirties many scattered cells with
alternating colors and 4-byte-UTF-8 glyphs, then `term::sync()`.

- Observed: heap overflow into adjacent arena memory once the per-cell excess exceeds
  the 64-byte trailer slack.
- Expected: bounded, safe present output.

(Confirmed by arithmetic; reachability gated on terminal geometry ≥1000 in a
dimension.)

## Root Cause

`src/target/shared/code/term_grid.rs:56` (`OUTBUF_PER_CELL = 64`) + `:829-1028`
(`emit_grid_present`): the fixed 64-byte/cell budget is under the worst-case
`2·d + 58` for d ≥ 4, and `emit_grid_alloc` (`:300-301`) loads `ws_row`/`ws_col`
without clamping, so d can reach 5. The `:55-56` comment ("a 4-byte glyph fits
comfortably") is wrong for wide/tall terminals.

## Goal

- The present buffer cannot be overflowed: size it with a coordinate-aware per-cell
  bound (max CUP digits for the actual rows/cols), and/or clamp grid dims, and/or
  bounds-check `buf` against `outbuf + capacity` and flush when exhausted.

### Non-goals (must NOT change)

- Present output for normal-sized terminals.
- The double-buffered diff model (plan-35).

## Blast Radius

- `OUTBUF_PER_CELL` sizing + `emit_grid_present` + `emit_grid_alloc` dim loading —
  fixed here.
- Confirm the CLI-side clamp (20..=1000) and the codegen path use consistent bounds.

## Fix Design

Compute `OUTBUF_PER_CELL` from the actual max coordinate digit count
(`2*digits(max(rows,cols)+1) + 58 + slack`), or clamp `rows`/`cols` at alloc time like
the CLI, or add a running bounds check in the present loop that flushes when the
buffer nears capacity. Recommend the coordinate-aware size (exact) plus a defensive
bounds check. Rejected: relying on the fixed 64 budget — it is provably insufficient.

## Phases

### Phase 1 — failing test
- [ ] A test constructing a ≥1000-dimension grid with many scattered colored dirty
      cells and asserting no overflow (or an ASAN-style guard).
### Phase 2 — the fix
- [ ] Coordinate-aware sizing + defensive bounds check.
### Phase 3 — validation
- [ ] Full suite green; normal terminals unaffected.

## Validation Plan

- Regression: the large-grid scattered-diff test.
- Runtime proof: no overflow under a ≥1000-dimension PTY.
- Doc sync: none.

## Summary

The present buffer's fixed 64-byte/cell budget underflows the worst-case escape
sequence for ≥4-digit coordinates, overflowing the arena block on large terminals.
Coordinate-aware sizing plus a bounds check fixes it.
