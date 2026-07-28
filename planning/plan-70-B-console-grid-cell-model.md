# plan-70-B: Console grid cell model — width, wide-trailing cells, EGC pool

Last updated: 2026-07-27
Overall Effort (AI): huge (>3d)
Effort (Human): large (3h–1d)
Effort (AI): medium (1h–2h)   — codegen against the existing writer/present shape, but the EGC pool lifecycle is novel
Depends on: plan-70-A
Produces:
- The console cell **width** storage + **wide-trailing sentinel** convention
  (a reserved glyph value the presenter skips).
- The per-grid **EGC pool** region (base + used pointer) in the shadow-grid header
  block, and its allocation/growth/reset lifecycle.
- A width-aware `emit_grid_write` (segments graphemes, wraps wide-at-edge, writes
  primary + trailing) and `emit_grid_present` (emits cluster bytes, skips
  trailing, `last_col += width`).
- The cell-model invariants (sentinel value, width offset, pool tagging) that
  plan-70-C's draw helpers mirror.

Serves all three console backends at once — `term_grid.rs` is neutral `abi::`
codegen shared across aarch64/x86/riscv (`04_term-backend.md:117-119`).

## Prerequisites

See umbrella §Prerequisites (bug-392 — required for Windows-console verification;
macOS/Linux console verification does not need it, but the feature does not ship
until Windows is proven too).

## 1. Goal

- The console shadow grid places one grapheme cluster per cell, advancing the
  cursor 0/1/2 columns by A's width. A wide cluster reserves a trailing cell and
  wraps rather than straddling the right edge.
- `term::sync`'s diff-presenter keeps a contiguous run of changed cells
  column-aligned across wide glyphs (no accumulation error like bug-392's
  cascade), emitting a CUP only when the physical cursor is genuinely elsewhere.
- Multi-scalar clusters (combining, ZWJ, flags) round-trip through the pool and
  render as one glyph.

### Non-goals

- No draw-helper changes (`drawText`/`drawGlyph`/`drawBox`/`fillRect`) — that is
  plan-70-C. B changes only the `io::write`/`print` grid writer and the presenter.
- No change to VT escape output shape beyond cursor-advance accounting; no
  code-page work (bug-392).

## 2. Current State

- Cell = 16 bytes (`term_grid.rs:49` `CELL_SIZE`): glyph u32 (packed UTF-8, ≤4
  bytes; `:519-576`), fg u32, bg u32, bold u8, un u8, **2 pad bytes at offset
  14–15** (per `04_term-backend.md` cell model / macOS `TermCell` twin). The pad
  bytes are free for a width field.
- Writer `emit_grid_write` (`:438`): decodes one UTF-8 scalar into the u32 glyph
  and does `col += 1` (`:581`); wraps at `col==cols` (`:558-562`), scrolls at
  `row==rows` (`:564-570`).
- Present `emit_grid_present` (`:856`): diff loop; emits CUP only when the cursor
  model `(last_row,last_col)` disagrees (`:991-999`), emits SGR + glyph
  (`append_glyph:186`), sets `last_col = col+1` (`:1064`), advances `col` per cell
  (`:1072`), wraps (`:1073-1077`).
- Header block (`04_term-backend.md:88-102`): rows/cols/cursorRow/cursorCol/dirty
  + back cells + front cells + out buffer. `emit_grid_alloc` (`:292`) sizes it;
  `emit_grid_resize` (`:648`) reflows on terminal resize; `emit_scroll_back`
  (`:232`) shifts rows. Term-state slot 56 is reserved (`04_term-backend.md:34`).
- The out buffer is sized `rows*cols*OUTBUF_PER_CELL(72) + TRAILER_SLACK(64)`
  (`:85-112`) as the worst-case escape run per changed cell; a cluster emitting
  more bytes than one scalar must be accounted for (see §3 risk).

### Verified properties

- The console cell holds ≤1 scalar; a cluster needs the pool (read
  `:519-576`+`append_glyph`). (Umbrella §2.)
- The presenter's `last_col += 1` is the exact site bug-392 identified as the
  auto-advance assumption (`:1064`; bug-392 Root Cause §). Changing it to
  `+= width` is the alignment fix. (Verified by read.)
- UNVERIFIED (task): whether `OUTBUF_PER_CELL=72` still bounds the worst-case
  per-cell escape run once a cell can emit a multi-scalar cluster (20+ bytes) plus
  CUP+SGR. Must re-derive the bound (the `worst_case` test at `:1199` is the
  guard) — a pooled cluster can exceed 72. This is a correctness-critical size.

## 3. Design

**Cell width + trailing sentinel.** Use cell offset 14 (u8) for `width` (0/1/2).
Reserve glyph value `0xFFFF_FFFF` as `WIDE_TRAIL` — a cell that the presenter and
draw helpers skip (it draws nothing; it exists so cursor math and diffing treat
the wide glyph's second column as occupied). A wide cluster writes `{glyph, width=2}`
at the primary and `{WIDE_TRAIL, width=0}` at primary+1.

**EGC pool.** Add two header fields (reuse the reserved slot / extend the header):
`egc_pool_base` and `egc_pool_used`, pointing at a growable byte region appended
after the out buffer (or a separate arena block). Glyph field encoding:
- high bit clear → inline packed UTF-8 (the existing ≤4-byte scalar path,
  unchanged for the common case);
- high bit set → `offset` into the pool of a length-prefixed cluster (u16 len +
  bytes), mirroring notcurses `egcpool`.
`append_glyph` (used by present) branches on the tag: inline → existing byte
unpack; pooled → copy `len` bytes from the pool. Pool is reset to empty on a full
repaint / resize and compacted on scroll (or, simpler and recommended for Phase 3:
never compact within a frame — the pool is per-grid and reused; stale offsets are
harmless because every live cell that references the pool is rewritten each frame
it changes). Decide compaction policy in Phase 3 (§Open Decisions of umbrella).

**Writer.** `emit_grid_write` gains a grapheme layer: instead of decoding one
scalar and writing one cell, it (a) finds the grapheme-cluster boundary via the
UAX #29 walker (reuse A's/`emit_grapheme_break_branch`), (b) computes the cluster
width via A's `emit_grapheme_display_width` logic on the cluster, (c) if width==2
and `col == cols-1`, wraps first (write a blank at the last col or just advance
row) so the wide glyph never straddles, (d) stores the cluster (inline if a single
≤4-byte scalar, else pool) + width at the primary cell, (e) for width==2 writes a
`WIDE_TRAIL` cell, (f) advances `col += width`. Newline/CR/scroll paths unchanged.

**Present.** In the diff loop: a `WIDE_TRAIL` cell is skipped for emission but
still consumes its column in the diff/advance bookkeeping. For a primary cell,
emit the cluster bytes (pool-aware `append_glyph`) and set `last_col = col + width`
(not `+1`). The change-detection XOR over the 16 cell bytes already covers the new
width byte and the pooled-offset glyph, so dirty-tracking still works.

**Where uncertainty is (schedule first):** the 2-cell layout + presenter advance.
Phase 1 does **single-scalar wide only** (no pool) to prove alignment on a real
terminal. **Where blast radius is (last):** the pool + the `OUTBUF_PER_CELL`
re-derivation, since a mis-sized out buffer overflows on a saturating repaint.

## Phases

### Phase 1 — width + wide-trailing for single-scalar wide glyphs (spike, no pool)

- [ ] Add the `width` byte (cell offset 14) and `WIDE_TRAIL` sentinel const.
- [ ] `emit_grid_write`: after decoding a scalar, call A's width helper; for
      width==2 write primary + `WIDE_TRAIL`, wrap-if-at-edge, `col += width`.
- [ ] `emit_grid_present`: skip `WIDE_TRAIL`, set `last_col = col + width`.
- [ ] Tests: a fixture printing `"[日]"` and `"[A]"` in a known grid; assert (via
      the console-grid test harness / a captured escape stream) that the `]`
      lands at the correct column after the wide `日`.

Acceptance: on a real UTF-8 terminal (macOS host), `"日本語"` followed by `"|"`
shows the `|` aligned (3 wide glyphs = 6 columns); the diff-present of a scrolling
CJK frame stays aligned (no bug-392-style scatter). Commit: —

### Phase 2 — EGC pool for multi-scalar clusters

- [ ] Add `egc_pool_base`/`egc_pool_used` header fields; size + zero them in
      `emit_grid_alloc`; reflow in `emit_grid_resize`; reset on full repaint.
- [ ] Glyph-field tagging (inline vs pooled); pool-aware `append_glyph`.
- [ ] Writer: segment full grapheme clusters (combining/ZWJ/flags), store pooled
      when >1 scalar; width from A's cluster rule.
- [ ] Re-derive and assert the out-buffer bound (`OUTBUF_PER_CELL` /
      `worst_case` test at `:1199`) now that a cell can emit a pooled cluster +
      CUP + SGR; raise `OUTBUF_PER_CELL` if the cluster max exceeds it.

Acceptance: `"café"` (NFD: `e`+`U+0301`) renders as 4 cells with the accent on the
`e`; `"👨‍👩‍👧‍👦"` renders as one 2-wide glyph; the out-buffer assert holds for a
worst-case pooled row. Commit: —

### Phase 3 — pool lifecycle hardening

- [ ] Decide + implement compaction/reset policy on scroll + resize (recommend:
      pool reset on full repaint, per-frame rewrite makes stale offsets inert;
      verify no unbounded growth across many frames).
- [ ] Tests: a long-running fixture that repeatedly rewrites pooled clusters and
      asserts `egc_pool_used` does not grow without bound.

Acceptance: a fixture that redraws a pooled-cluster frame N times keeps pool usage
bounded; `cargo test` + artifact-gate green (goldens deferred to G). Commit: —

## Validation Plan

- Tests: console-grid write/present unit or function tests for width advance,
  wrap-at-edge, `WIDE_TRAIL` skip, and pooled clusters.
- Coverage check: a fixture exercising width==2 **and** a pooled multi-scalar
  cluster through the diff-present path (not just the writer).
- Runtime proof: `browser` example + a CJK/emoji panel on the macOS host and a
  Linux box; box borders stay aligned. (Windows console proof is in G, gated on
  bug-392.)
- Doc sync: `04_term-backend.md` cell model + header block (G).
- Acceptance: `cargo test` + artifact-gate.

## Corrections

<Filled in during execution.>
