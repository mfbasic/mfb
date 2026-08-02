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

- [x] Add the `width` byte (cell offset 14, `C_WIDTH`) and `WIDE_TRAIL` sentinel
      const (`0xFFFF_FFFF` = "4294967295"; never a valid packed UTF-8 scalar).
- [x] `emit_grid_write`: decode the scalar's codepoint and its charwidth via the
      new free primitives (`emit_utf8_codepoint_by_len` +
      `emit_unicode_charwidth_free`, since the writer is a raw-instrs free function,
      not a `CodeBuilder`); for width==2 write primary + `WIDE_TRAIL`, wrap a wide
      glyph off the right edge, `col += width`. A standalone zero-width scalar falls
      back to width 1 (the EGC pool that folds combining marks is Phase 2). Threaded
      a `relocations` param through `emit_grid_write` (and its `io_stdout` caller)
      for the property-table data-address loads.
- [x] `emit_grid_present`: skip a `WIDE_TRAIL` cell (emit nothing, mark presented),
      and set `last_col = col + (width==2 ? 2 : 1)` — robust to cells the
      width-aware writer never touched (stored width 0 → advance 1).
- [x] Tests: `tests/rt-behavior/term/func_term_wide_glyph_valid` — runs piped
      (24x80 fallback, deterministic escape stream) and its `build.log` golden
      captures `日本語|` (| at column 6), a wide-at-edge wrap, and a diff pass.

Acceptance: verified on the macOS host via the captured escape stream — `"日本語|"`
emits with a **single** leading `[1;1H` CUP and no intermediate CUP (presenter
cursor model advances by width, the bug-392 fix), 73 aligned trailing blanks
(cols 7-79), zero `0xFF` sentinel leak; the writer's cursor lands at display col 7
(`[1;8H`, i.e. 6+1, not the 3+1 a scalar-count advance would give); a width-2 glyph
at column 79 wraps to the next row (`[4;1H`); the diff-present re-emits only the one
changed cell, still aligned (`[1;7H`). `cargo test` 3748 passed; existing term
acceptance goldens unchanged (ASCII width==1 path is byte-identical). Commit: fbdce19e7

### Phase 2 — EGC pool for multi-scalar clusters

- [x] ~~Add `egc_pool_base`/`egc_pool_used` header fields; size + zero them in
      `emit_grid_alloc`; reflow in `emit_grid_resize`; reset on full repaint.~~ —
      **redesigned to per-cell pool slots** (see Corrections): each cell owns a
      fixed `POOL_BYTES_PER_CELL`-byte slot at `pool_base + cell_index*POOL`, so
      there is NO bump pointer, NO `egc_pool_used`, NO reset, and NO reflow/compaction
      lifecycle — the slot is overwritten whenever the cell is. This sidesteps the
      whole umbrella lifecycle concern that Phase 3 was to harden. Block per-cell
      size grew by `POOL_BYTES_PER_CELL` in all 6 alloc/free/resize size formulas;
      `pool_base = gp + HDR_SIZE + ncells*(2*CELL+OUTBUF)`.
- [x] Glyph-field tagging (inline vs pooled); pool-aware emission. Pooled tag =
      top byte `0xC0` (NOT bit 31 — see Corrections: an inline 4-byte scalar sets
      bit 31); low 24 bits hold the cluster length. Presenter emits pooled bytes
      from the slot, else `append_glyph` for inline; pooled cells always re-emit
      (their bytes are outside the 16-byte XOR diff).
- [x] Writer: segment full grapheme clusters via a peek-ahead loop over the
      free-function UAX #29 break machinery (`emit_grapheme_break_branch_free` +
      `emit_grapheme_state_update_free`), store pooled when >1 scalar; width = the
      cluster's first non-zero-width scalar. A cluster is capped at
      `POOL_BYTES_PER_CELL` bytes (graceful).
- [x] Re-derived the out-buffer bound: `OUTBUF_PER_CELL` raised 72→136 (a pooled
      cell can emit up to `POOL_BYTES_PER_CELL`=64 glyph bytes vs. 4); the
      `outbuf_per_cell_covers_the_worst_case_escape_run` test now checks the
      pooled worst case.

Acceptance: verified on the macOS host via the captured escape stream —
`"cafe"+U+0301+"|"` (NFD café) renders as `café|` with the `é` folded into ONE
cell and `|` at column 4; `"👨‍👩‍👧‍👦|"` renders the ZWJ family as ONE wide cluster with
`|` at column 2; the diff pass overwrites the café marker while the pooled `é`
stays aligned; a **found-and-fixed bug**: an astral emoji (`😀`, a 4-byte inline
scalar) SIGSEGV'd the presenter under the original bit-31 tag (it collided with a
real inline glyph) — fixed by moving the tag to top-byte `0xC0`, caught by the
`func_term_drawGlyph_valid` regression. `cargo test` 3748 passed (incl. the
re-derived out-buffer test); all 34 term acceptance fixtures unchanged. Commit: fd630daaf

### Phase 3 — pool lifecycle hardening

- [x] ~~Decide + implement compaction/reset policy on scroll + resize~~ — **moot
      under the per-cell-slot design** (Phase 2 Corrections): there is no bump
      allocator to compact/reset and no unbounded growth is possible — the pool is
      a fixed `ncells * POOL_BYTES_PER_CELL` region, each cell overwriting its own
      slot in place. What DID need doing, and is done, is keeping a pooled cell's
      slot in sync with its cell across the two content-moving paths:
      `emit_scroll_back` now shifts the pool slots up one row in lockstep with the
      cells (and blanks the last row's slots); `emit_grid_resize` copies the
      overlap's pool slots into the new block alongside the cells (else a copied
      pooled tag would point at the new block's zeroed slot and emit NUL bytes — a
      found-and-fixed latent bug).
- [x] ~~Tests: a long-running fixture asserting `egc_pool_used` does not grow~~ —
      **moot** (no `egc_pool_used`; growth is structurally impossible). Instead
      verified the scroll path directly: a pooled `é` (NFD) written near the bottom
      row and then scrolled up renders correctly at its new row with ZERO NUL bytes
      in the escape stream (the slot moved with the cell).

Acceptance: `cargo test` 3748 passed; all 35 term acceptance fixtures green
(including the new cluster-pool fixture); a pooled cluster survives a scroll with
no NUL/garbage emission. **Note (accepted tradeoff, not a bug):** a pooled cell is
re-emitted on every present (its cluster bytes live outside the 16-byte XOR diff),
so `term::off`'s final present idempotently re-draws pooled cells — correct, minor
bandwidth only; a content-hash-in-glyph optimization is a possible future follow-up.
Commit: fd630daaf

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

- **2026-08-02 — A's width helpers are `CodeBuilder` methods; the console
  writer/present are raw free functions, so B added free-function mirrors.** The
  plan (§3) says to "reuse A's `emit_grapheme_display_width` logic" and A's
  charwidth helper, but `emit_grid_write`/`emit_grid_present` in `term_grid.rs`
  are free functions that push `abi::` into a `Vec<CodeInstruction>` (+ a
  `Vec<CodeRelocation>`), not `CodeBuilder` methods that go through `self.emit`.
  So B added three `pub(in crate::target::shared::code)` free functions in
  `private/unicode.rs` — `emit_load_data_address_free`, `emit_utf8_codepoint_by_len`,
  `emit_unicode_charwidth_free` — mirroring the `CodeBuilder` helpers but taking
  the current `symbol` (relocation `from`), a `label_prefix`, and explicit scratch
  vregs. The existing `CodeBuilder` methods are left untouched (no golden churn on
  the strings/regex paths). plan-70-C's draw helpers (also free functions in
  `term.rs`) will reuse these same free primitives.
- **2026-08-02 — `emit_grid_write` needed a new `relocations` parameter.** It had
  only `instrs`; the property-table lookup emits data-address relocations, so the
  signature grew a `relocations: &mut Vec<CodeRelocation>` threaded from its sole
  caller (`io_stdout.rs` `lower_io_write_helper`, which already has `relocations`
  and `symbol` in scope).
- **2026-08-02 — EGC pool redesigned to per-cell slots (not a bump arena).** The
  umbrella/§3 describe a growable `egc_pool_base`/`egc_pool_used` bump arena with a
  reset/compaction lifecycle (the concentrated-risk part). Implemented instead as
  a fixed `POOL_BYTES_PER_CELL`-byte slot per cell (`pool_base + idx*POOL`), which
  is bounded by construction and needs no bump pointer, reset, or compaction —
  removing the entire Phase 3 lifecycle surface. The only extra work is keeping a
  slot in sync with its cell on scroll (`emit_scroll_back` shifts slots) and resize
  (`emit_grid_resize` copies overlap slots).
- **2026-08-02 — the pooled glyph tag is top-byte `0xC0`, NOT bit 31.** First cut
  used `0x8000_0000` (high bit). But a 4-byte UTF-8 scalar packs its 4th byte (a
  continuation byte, always `0x80..=0xBF`) into bits 24-31, so an inline astral
  glyph (e.g. `😀` = `0x80989FF0`) has bit 31 set and was misread as pooled → the
  presenter emitted `len = glyph & 0x7FFF_FFFF` ≈ 10 MB from the pool slot →
  SIGSEGV. Caught by the `func_term_drawGlyph_valid` acceptance regression (which
  draws `😀`). Inline glyphs never have top byte `0xC0..=0xFE`, so the tag moved to
  `0xC0000000 | len`, with the length masked by `0x00FF_FFFF`.
