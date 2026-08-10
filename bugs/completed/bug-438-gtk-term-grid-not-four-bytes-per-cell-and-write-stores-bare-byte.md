<!-- Bug document. See .claude/skills/write-bug/template.md -->

# bug-438: GTK term backend regresses the UTF-8 char grid — `_mfb_gtkapp_state` is far larger than 4 bytes/cell and `term_write` stores a bare byte per grid cell (regression of bug-203)

Last updated: 2026-08-09
Effort: small (triaged — stale layout assertion, no codegen change)
Severity: MEDIUM
Class: Correctness (GTK terminal glyph storage / state layout)

STATUS: FIXED (ce642096d) — stale test model, NOT a codegen regression.
Regression Test: tests/rt_gtk_term_utf8_grid.rs (already present)

## Resolution (triaged 2026-08-09)

Both hypotheses in the doc below turned out FALSE for the current tree — the
codegen was correct the whole time; only the test's mirrored constants were
stale, and they had already partially drifted forward since the doc was written:

- `gtk_term_write_decodes_a_code_point_per_cell` — **already GREEN.** bug-437
  (`527b36591`, "update stale GTK term grid tests for plan-70-E EGC pool")
  removed the blanket `!str_u8@0` assertion the doc flagged: that `str_u8@0` is
  the EGC pool slot's length-prefix byte (`emit_gtk_pool_append`), not a CHAR
  grid cell. The live bug-203 guard (CHAR cell is `str_u32`) still passes.
- `gtk_state_sizes_the_char_grid_at_four_bytes_per_cell` — the doc's 3.6× gap
  (185544 vs 677064) was also bug-437's already-applied EGC-pool fix. What
  remained on the current tree was a NEW 8-byte drift: got **677072**, expected
  **677064**. Cause: `e6720a230` (`term::didResize()`) inserted one new
  `ST_TERM_DID_RESIZE` u64 into `_mfb_gtkapp_state` between `ST_TERM_CELL_H` and
  `ST_TERM_CHARS` (src/target/linux_gtk/mod.rs:130), growing the state by
  exactly 8 bytes, but did not update this integration test's mirrored size
  model. The char grid is STILL 4 bytes/cell — the `6*CELLS*4` grid term is
  unchanged and the `str_u32` cell store still holds — so bug-203 is intact.

Fix (`ce642096d`): bump the test's geometry block `13 * 8` → `14 * 8` and note
the `ST_TERM_DID_RESIZE` latch in the comment. Verified:
`11*8 + 1024 + 14*8 + 6*CELLS*4 + 2*CELLS*32 + 8 = 677072` = actual. No codegen
touched. This is the recurring "codegen-inspection tests hardcode drifting
constants" pattern — the size model rots after any plan/feature that relayers
the state.

Original triage doc (superseded) follows.

Two codegen-inspection assertions in `rt_gtk_term_utf8_grid` (both guarding
bug-203, "the char grid stores one packed 32-bit glyph per cell, not raw bytes")
are RED on the current tree:

1. `gtk_state_sizes_the_char_grid_at_four_bytes_per_cell` — the `_mfb_gtkapp_state`
   data object is **677064** bytes, but the test's 4-bytes/cell layout model
   (`11*8 + 1024 + 13*8 + 6*CELLS*4 + 8`, `CELLS = 160*48`) expects **185544**.
   The state is ~3.6× larger than the model — either the grid widened past 4
   bytes/cell (regressing bug-203) or the state layout gained fields the test
   doesn't account for.

2. `gtk_term_write_decodes_a_code_point_per_cell` — `_mfb_gtkapp_term_write`
   contains a `str_u8` at `offset 0`, tripping the guard
   *"term_write must not store a bare byte into a grid cell (bug-203)"*: the write
   path is storing a lone byte into a cell instead of a packed u32 glyph.

Both reproduce identically on base commit `03309dd8a` (verified via
`git worktree add --detach`), so they predate bug-436 (diff:
`src/binary_repr/sections.rs` only) and were merely surfaced by bug-436's
full-suite finalization.

## Failing Reproduction

```sh
cargo build --release --bin mfb
cargo test --test rt_gtk_term_utf8_grid --no-fail-fast
```

Observed:

```text
gtk_state_sizes_the_char_grid_at_four_bytes_per_cell ... FAILED
  got size=677064, expected 185544
gtk_term_write_decodes_a_code_point_per_cell ... FAILED
  term_write must not store a bare byte into a grid cell (bug-203)
```

## Root Cause (unknown — triage required)

Two possibilities, resolve which before touching anything:

- **Real regression:** the GTK term grid/state emitter reverted to per-byte cell
  storage (bug-203), inflating the state and re-introducing the bare-byte write.
  Fix the emitter; the tests are correct.
- **Stale assertion:** an intentional GTK state-layout change (new per-cell fields
  or geometry) widened `_mfb_gtkapp_state`, and the test's hard-coded 4-bytes/cell
  size model + `str_u8`-offset-0 heuristic were never updated. Then correct the
  test's layout model against the new intended layout (only after proving the new
  layout is intended — do NOT re-baseline without that proof).

Start with `git log -S _mfb_gtkapp_state` / the bug-203 fix and the GTK term
codegen (`gtkapp` / `term_write` emitters) to see when the size diverged.

## Blast Radius

- GTK app/term codegen emitters (`_mfb_gtkapp_state`, `_mfb_gtkapp_term_write`).
- `.ncode` / data-object layout for GTK-term fixtures.

## Summary

The GTK terminal backend's `_mfb_gtkapp_state` is ~3.6× the 4-bytes/cell model the
test encodes, and `term_write` stores a bare byte per cell — both bug-203 guards
in `rt_gtk_term_utf8_grid` are RED on the current tree. Needs triage to decide
whether the emitter regressed or the layout assertion is stale. Pre-existing;
found during bug-436 finalization.
