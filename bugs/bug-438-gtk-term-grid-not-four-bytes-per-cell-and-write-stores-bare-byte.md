<!-- Bug document. See .claude/skills/write-bug/template.md -->

# bug-438: GTK term backend regresses the UTF-8 char grid — `_mfb_gtkapp_state` is far larger than 4 bytes/cell and `term_write` stores a bare byte per grid cell (regression of bug-203)

Last updated: 2026-08-08
Effort: unknown (needs triage — real codegen regression vs. stale layout assertion)
Severity: MEDIUM
Class: Correctness (GTK terminal glyph storage / state layout)

Status: Open (pre-existing; discovered while landing bug-436, NOT caused by it)
Regression Test: tests/rt_gtk_term_utf8_grid.rs (already present — currently RED)

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
