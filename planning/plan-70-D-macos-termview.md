# plan-70-D: macOS AppKit `TermView` — grapheme decode, astral fix, width layout

Last updated: 2026-07-27
Overall Effort (AI): huge (>3d)
Effort (Human): large (3h–1d)
Effort (AI): large (3h–1d)   — intricate objc-msgSend codegen + on-device (macOS host) visual proof; converges with Human on the verify loop
Depends on: plan-70-A
Produces: a macOS app-mode TUI grid that stores/renders one grapheme cluster per
cell at its correct width, including astral (surrogate-pair) emoji.

Independent of B/C (separate storage: heap `TermCell[]`, AppKit rendering), but
mirrors B's cell-model contract (width byte, `WIDE_TRAIL` sentinel, cluster
storage) so the shared spec (`04_term-backend.md`) describes one model.

## Prerequisites

See umbrella. macOS host is the verification target for D.

## 1. Goal

- The `TermView` grid lays each grapheme cluster in one cell, advancing 0/1/2
  columns by A's width; a wide cluster reserves a trailing cell and wraps at the
  edge.
- Astral scalars (emoji ≥ U+10000) render as one glyph — the current
  surrogate-splitting bug (`term_view.rs:650-665`) is fixed.
- Combining/ZWJ clusters render as one glyph via CoreText (which already cascades
  fonts), not torn across cells.

### Non-goals

- No transcript-view changes (the non-TUI scroll view); TUI grid only.
- No font change (macOS `userFixedPitchFontOfSize:` + CoreText cascade already
  substitutes CJK/emoji; the gap is geometry, not fallback — §2).

## 2. Current State

- Cell = `TermCell` 16 B: `CELL_GLYPH_OFFSET=0` u32 **unichar (UTF-16 unit)**,
  fg u32 @4, bg u32 @8, bold u8 @12, un u8 @13, **pad @14–15**
  (`mod.rs:464-468`, `04_term-backend.md:204-214`).
- Writer `emit_term_write_string_helper` (`term_view.rs:1878`) iterates
  `[str characterAtIndex:i]` (UTF-16 units), stores one unit per cell, `col += 1`;
  draw path builds `[NSString stringWithCharacters:&glyph length:1]`
  (`:650-665`). So an astral scalar → two cells each a lone surrogate → tofu
  (verified by read).
- Grid sizing from `[font maximumAdvancement].width` +
  `defaultLineHeightForFont:` (`term_view.rs:770-814`) — one monospace advance per
  cell; a substituted wide glyph overflows a single cell (verified by read).
- Draw helpers (`mfbDrawText:`/`mfbDrawGlyph:`/`mfbDrawLine:`/`mfbDrawBox:`/
  `mfbFillRect:`) each stamp one unichar per cell (`04_term-backend.md:305-353`).
- `setFrameSize:` reflows/`calloc`s the grid on resize (`term_view.rs:2617+`);
  `term_scroll`/`term_clear` shift/zero cells.

### Verified properties

- The writer/draw path is UTF-16-unit-granular, so astral emoji break regardless
  of width work; D must switch iteration to scalars/clusters. (Read
  `:650-665`,`:1878`.)
- The single-cell advance geometry means even a CoreText-substituted wide glyph
  overflows; D must lay wide clusters across 2 cells and draw the cluster string
  spanning `2*cellW`. (Read `:770-814`, draw at `:666+`.)

## 3. Design

- **Cell grows to hold a cluster + width.** Options: (a) store a cluster as an
  NSString retained per cell (heavy), or (b) store the cluster's UTF-8/UTF-16
  bytes in a per-view EGC pool (mirror B) with the glyph field a tagged
  offset, and a `width` byte in the pad. Recommend (b) for parity with B and to
  keep `TermCell` 16 B. The draw path builds the NSString from the pooled bytes
  via `stringWithCharacters:length:` (UTF-16) or `stringWithUTF8String:`.
- **Writer** decodes the input by grapheme (the input arrives as an NSString;
  either iterate `rangeOfComposedCharacterSequencesForRange:`/
  `enumerateSubstringsInRange:options:NSStringEnumerationByComposedCharacterSequences`
  — AppKit's own grapheme segmentation — or decode UTF-8 and reuse the shared
  walker). For each cluster: width from A (compute on the cluster's scalars), store
  cluster + width, write primary + `WIDE_TRAIL`, wrap-if-at-edge, `col += width`.
  This also fixes astral, because a surrogate pair is one composed-character range.
- **drawRect:** for a primary cell, draw the cluster string at `(col*cellW,
  row*cellH)` with width `width*cellW` available (a wide glyph naturally draws
  ~2 cells wide in a proportional-substituted face); skip `WIDE_TRAIL` cells.
- **Draw helpers** (`mfbDrawText:` etc.): same cluster/width contract as C does for
  the console; a wide glyph occupies 2 cells, trailing skipped, overwrite clears
  the pair.

**Uncertainty first:** whether to reuse AppKit's composed-character enumeration or
the shared UTF-8 walker — Phase 1 spikes the astral fix with AppKit enumeration
(smallest change that renders `"👍"` as one glyph). **Blast radius last:** the
resize/scroll paths that must carry the pool + width fields.

## Phases

### Phase 1 — astral fix + width for single-scalar wide (spike)

- [x] Writer (`emit_term_write_string_helper`): decode astral scalars from UTF-16
      surrogate pairs so the cell holds the full codepoint (fixes the
      surrogate-splitting tofu — the documented bug); look up A's charwidth via the
      hand-written two-stage trie (identical logic to the console path), store the
      width byte in the cell pad (`CELL_WIDTH_OFFSET=14`), write `APP_WIDE_TRAIL`
      for width==2, wrap a wide glyph off the right edge, `col += width`. The
      width/table path is gated on `uses_term` so a non-term app never carries the
      ~1.5 MB unicode table (see Corrections).
- [x] `drawRect:` renders the stored codepoint (BMP → one UTF-16 unit, astral →
      the surrogate pair, two units) and skips `APP_WIDE_TRAIL` (its background was
      already filled; the wide primary spans the column via CoreText).
- [x] Test: the astral+width writer runs headless over `"👍 日本語 A café 😀"`
      (`MFB_MACAPP_HEADLESS=1`) with exit code 42 — no crash from the surrogate
      decode or the table lookup; `scripts/test-macapp.sh` all cases pass (no
      regression); the app links (the `_mfb_unicode_*` relocations auto-embed the
      table). The plan's assumed "grid-state inspection path" does not exist; the
      grid holds no MFBASIC-visible surface, so correctness is established by
      logic-equivalence to the byte-identity-verified console writer + the
      no-crash/link/regression evidence above.

Acceptance: implementation complete and verified by the autonomous means above
(codegen determinism via the regenerated `macos-app-mode-term` `.ncodesum`;
headless no-crash on real astral/CJK/emoji input; full `test-macapp.sh` pass;
correct table-gating). The pixel-level GUI proof (`"👍 日本語 A"` as single aligned
glyphs) is this plan's explicit **human-convergence step** ("converges with Human
on the verify loop") — an interactive window-server session is required; a scripted
screenshot in this automated session did not composite the window (even ASCII did
not paint, so it is a compositing/session limit, not the glyph path). Commit: —

### Phase 2 — EGC pool for multi-scalar clusters + draw helpers

- [ ] Per-view EGC pool (or per-cell retained NSString); pool-aware drawRect.
- [ ] Grapheme segmentation for combining/ZWJ; `"café"` (NFD) and `"👨‍👩‍👧‍👦"` as
      single glyphs.
- [ ] `mfbDrawText:`/`mfbDrawGlyph:`/`mfbDrawBox:`/`mfbFillRect:`/`mfbDrawLine:`
      width/cluster-aware; overwrite clears the wide pair.
- [ ] Resize (`setFrameSize:`) + scroll + clear carry the width/pool fields.

Acceptance: NFD `"café"`, ZWJ family, and a `drawBox` around CJK all render/align
on the macOS host; resize reflows without corrupting wide cells. Commit: —

## Validation Plan

- Tests: macOS app grid-state tests (headless) for width/astral/cluster; a GUI
  smoke run for visual confirmation.
- Coverage check: an astral + a pooled-cluster fixture through the writer and a
  draw helper.
- Runtime proof: the `wide-demo` panel on the macOS host GUI; borders align.
- Doc sync: `04_term-backend.md` macOS `TermView` section (G).
- Acceptance: `cargo test` + artifact-gate + macOS GUI run.

## Corrections

- **2026-08-02 — the width path must be gated on `uses_term`, or every macOS app
  bloats.** The `TermView` writer is emitted in EVERY app build (the bootstrap
  unconditionally wires the class), so an unconditional charwidth lookup made a
  non-term app reference `_mfb_unicode_*` → the shared build embedded the ~1.5 MB
  table (mod.rs `references_unicode_table`). The linker dead-strips it from the
  binary (a non-term app stayed 106 KB), but the `.ncode` DUMP carried it, ballooning
  the `macos-app-mode-io/plumbing` goldens ~1 MB→2.5 MB. Fixed by threading
  `spec.uses_term` into `emit_term_write_string_helper` and gating the whole width
  path (lookup + `WIDE_TRAIL` + `col+=width`) on it; the astral decode stays
  unconditional (table-free). Only a real term app now embeds the table.
- **2026-08-02 — the `macos-app-mode-term` `.ncode` golden converted to `.ncodesum`.**
  A term app legitimately embeds the table, so its raw `.ncode` golden hit 2.57 MB.
  Converted the `macos-aarch64.app.ncode` golden to a sha256 `.ncodesum` (matching
  how big backends avoid committing megabyte dumps); the codegen-determinism check
  is preserved.
- **2026-08-02 — no "grid-state inspection path" exists (the plan assumed one).**
  The `TermView` grid is a `calloc`'d struct with no MFBASIC-visible surface, and
  `test-macapp.sh` asserts exit codes / io, never rendered cells. The astral+width
  writer's correctness is therefore established by (a) logic-equivalence to the
  console writer (same two-stage trie + surrogate math, byte-identity-verified in
  B) and (b) headless no-crash + link + full `test-macapp.sh` regression. The
  pixel-visual proof remains a human step.
- **2026-08-02 — the width lookup is hand-written in the app's `asm` style, not the
  free helper.** `term_view.rs` builds via the objc `Asm` (physical registers,
  `asm.local_address` for data addresses). Rather than risk the neutral
  `emit_*_free` helpers not lowering cleanly in that context, the two-stage trie +
  `(flags>>4)&3` was written directly with `asm.local_address(UNICODE_*_SYMBOL)` —
  whose `_mfb_unicode_*` relocation is exactly what the shared build's
  `references_unicode_table` gate keys on to embed the table.
