# plan-162-A: a parse-once face index owned by the `canvas::Font` resource

Last updated: 2026-09-27
Overall Effort: huge (>3d) — the whole plan-162 text feature, split A–F below
Effort: large (3h–1d)
Depends on: bug-756 (see Prerequisites)

plan-162 is the single plan for every font/text/canvas/strings change that follows
bug-756. It is split, in implementation order:

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | Parse-once **face index** on the `Font` resource; text code reads glyphs, advances and metrics by handle, never by copying the font file | large |
| B | Kerning — `GPOS` pair adjustment (formats 1 and 2, through extension lookups) and the legacy `kern` table, applied in `__canvas_shapeRun` | large |
| C | The public font list — `measureText` overload over `List OF RES canvas::Font`, `canvas::Text.fonts`, per-codepoint fallback when drawing | large |
| D | Letter-spacing, decoration metrics (underline/strikeout), and caret stops for hit-testing | large |
| E | `strings::lineBreaks` — UAX #14 line-break opportunities | large |
| F | Standard ligatures (`GSUB` `liga`, lookup type 4) | large |

This letter makes the `canvas::Font` resource own its own parsed form. That means
parsing the font's tables once, when it is loaded, into an immutable **face index**
(a flat `List OF Integer`) that the resource holds and publishes next to its bytes.
After this letter, text code works from a font **handle**:

- mapping a codepoint to a glyph, finding a glyph's advance, and reading the font-wide
  metrics each read the face index in place through one native accessor;
- none of them copies the font file.

Only reading a glyph's outline still needs font bytes, which is the one case where
they are truly needed.

**The single behavioral outcome:** `canvas::measureText` and every text geometry build
produce byte-identical results to HEAD, and none of them calls `__canvas_fontBlob`
or `canvas::fontBlobUnchecked` except to read `glyf` outlines.

References:

- bug-756 (`bugs/completed/bug-756-*.md` once fixed) — introduces
  `__canvas_shapeRun(fonts AS List OF Integer, text)`. This letter changes only its body.
- `.ai/canvas-threading.md` §1–2 (per-thread arena; "never allocate from another
  thread's arena"), §7 (closing an image or font — the closed flag), and §8 rows
  R17/R17b (`destroyFont` unregisters immediately).
- `src/codegen/builtins/canvas/gen_font_table.rs` — the 16-slot process-global font
  table (module doc).
- `src/codegen/builtins/canvas/func_measure_text.rs` — the `fontBytes` "It copies, and the
  first version did not" history (exit-139 segfault from aliasing).
- `src/docs/spec/app/06_canvas.md` §"A loaded font is always one standalone face".
- `.ai/specifications.md`, `.ai/spec-content.md` (spec sync), `.ai/testing-gates.md`
  §"Canvas reference images" (a PNG diff is a bug hunt, not a regeneration).

## Prerequisites

This gate is the only hard stop for plan-162. Each row names the letter it gates. A
letter may not start while any row gating it is NOT MET.

| Must be true | Gates | Command | Status |
|---|---|---|---|
| bug-756 fixed and archived (`__canvas_shapeRun` over a handle list exists) | A–F | `ls bugs/completed/bug-756-* && grep -n "FUNC __canvas_shapeRun(fonts AS List OF Integer" src/codegen/builtins/canvas/helper_font.rs` → both match | MET (2026-10-14: `bugs/completed/bug-756-canvas-text-has-four-copies-of-the-glyph-walk.md`; grep hit `helper_font.rs:310`) |
| bug-753 fixed (a `List OF RES` bound from a call no longer leaks) | C | `ls bugs/completed/bug-753-*` → match | MET (2026-10-14: `bug-753-a-list-of-res-bound-from-a-call-leaks-its-block.md`) |
| bug-755 fixed (a function returning a passed handle doesn't close it early) | C | `ls bugs/completed/bug-755-*` → match | MET (2026-10-14: `bug-755-a-user-function-handing-back-a-passed-handle-gets-it-closed-early.md`) |

Why bug-753 and bug-755 gate C: C makes `canvas::Text` hold a `List OF RES
canvas::Font`. The natural program shape is a helper that returns the chain, for
example `FUNC uiFonts(primary AS RES canvas::Font) AS List OF RES canvas::Font`, called
every frame. bug-753 makes that leak one list block per call. bug-755 closes `primary`
at the caller's inner scope. C would ship an API whose idiomatic use is broken.

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you.

## 1. Goal

- A `canvas::Font` owns a face index built once in `canvas::loadFont`. It is stored on
  the resource record and published in the font table beside the bytes.
- `__canvas_shapeRun`, `__canvas_measureText` and `__canvas_glyphRunHeader`'s
  ascent/descent box read the index through `canvas::faceAt(handle, i)`, and never
  through a font-byte copy.
- `grep -n "__canvas_fontBlob\|fontBlobUnchecked" src/codegen/builtins/canvas/helper_*.rs src/codegen/builtins/canvas/func_*.rs`
  lists only the outline readers (`__canvas_glyphEntry`'s caller and
  `__canvas_textEdges`) and the definition itself.
- All `tests/golden/canvas/*.png` are byte-identical, and every `rt_canvas_*` suite
  passes unchanged.

### Non-goals (explicit constraints)

- **No behavior change.** This includes rendered pixels, measured metrics, raised
  errors (`measureText` on a closed font still raises `ErrResourceClosed`), and the
  unmapped-codepoint → glyph 0 rule. This letter is **provably neutral**, so
  byte-identical PNG goldens and `rt_canvas_*` results are its correctness gate. A
  diff means root-cause it (dump one frame) and fix it. A diff never means the design
  is dead.
- **No public API change.** Every new member is `internal_only: true`.
- **No kerning, fallback, spacing or ligatures.** Letters B–F add those. The index
  layout reserves no slots for them; each letter appends its own section and bumps
  the index version word (§4.1).
- **Outlines keep reading bytes.** Composite-glyph support (bug-757) and any narrower
  outline copy are out of scope.
- **The font table's chunk geometry is unchanged** apart from the slot stride (Correction 5).

## 2. Current State

- **Loading.** `canvas::loadFont` → `__canvas_loadFont` / `__canvas_loadFontBytes`
  (`func_load_font.rs`), in MFBASIC on the worker, extracts one face (`__canvas_fontTable`
  rebuilds a standalone face from a collection) and calls `canvas::fontFromBytes(face)`.
  The lowering `func_load_font.rs:lower_font_from_bytes` allocates a `RESOURCE_RECORD_SIZE`
  (96-byte, `error_constants.rs:RESOURCE_RECORD_SIZE`) record, stores the owned byte block
  at `FONT_BYTES` (offset 32, `gen_font.rs:FONT_BYTES`), and calls
  `gen_font_table::emit_register_font(handle = record, block = bytes)`.
- **Font table.** `gen_font_table.rs`: 16 fixed `(handle, block)` slots, no count word.
  A zero handle marks an empty slot. `canvas::fontRegistered(id)` scans the slots.
  `canvas::fontBlobUnchecked(id)` returns "an owned copy" of the file.
  `__canvas_fontBlob(id)` answers `[]` when the handle is unregistered.
- **Readers.** `helper_font.rs` parses tables from a `List OF Byte`:
  - `__canvas_faceTable` (table directory search);
  - `__canvas_fontUnitsPerEm`, `__canvas_fontAscent`, `__canvas_fontDescent`,
    `__canvas_fontLineGap`;
  - `__canvas_glyphAdvance` (`hmtx`, with the shared trailing advance);
  - `__canvas_glyphIndex` → `__canvas_cmapSubtable` (prefers format 12 over 4) →
    `__canvas_cmap12` / `__canvas_cmap4`.

  `__canvas_glyphIndex` re-runs `__canvas_cmapSubtable`'s record scan **on every
  call** (read: `helper_font.rs:__canvas_glyphIndex` calls `__canvas_cmapSubtable(b)`
  first).
- **Threads.** Text geometry is built on the **graphics** thread (the geometry cache is
  the graphics thread's, `.ai/canvas-threading.md` §1). `measureText` runs on the
  **worker**. MFBASIC module globals are per-thread (§2), so a cache kept in MFBASIC
  globals would be built separately on each thread. It would also be keyed by a handle
  that is a record **address**, which can be reused after `destroyFont`.
- **Why bytes are copied.** `canvas::fontBytes` history: handing MFBASIC the resource's
  own block let a `LET`'s scope-drop free it, and the next call segfaulted (exit 139).
  Any worker-owned block read on another thread must therefore be read **in place by
  native code**, or copied. It must never be bound to a MFBASIC name.

### Measured populations

| What | Count | Command |
|---|---|---|
| Font table slots | 16 | `gen_font_table.rs` module doc ("Sixteen fixed slots") |
| Font-byte fetches in canvas MFBASIC outside the table module | 8 (measured 2026-10-14): `helper_font.rs` `__canvas_shapeRun`; `func_measure_text.rs` `__canvas_measureText`; `helper_geometry.rs` `__canvas_textGlyphRun`, `__canvas_textEdges`, `__canvas_glyphRunHeader`, `__canvas_geoRunReference`; `helper_hit.rs` `__canvas_textBoxHeader`; `helper_particles.rs` `__canvas_particleTextBox`; `helper_autopaint.rs` `__canvas_autoTextBox`. Only `__canvas_textGlyphRun` (via `__canvas_glyphEntry`) and `__canvas_textEdges` (via `__canvas_glyphEdges`) read outlines | `grep -n "__canvas_fontBlob(" src/codegen/builtins/canvas/helper_*.rs src/codegen/builtins/canvas/func_*.rs` |
| Existing `rt_canvas_font` tests | 24 | `grep -c "^#\[test\]" tests/canvas/rt_canvas_font.rs` → 24 |
| Face-index size for real fonts | 638 TrueType faces: median 997 words, max 122,707 words (`STHeiti Light.ttc` face at dir 32: R = 23,495 format-12 runs, H = 52,213). Worst index-bytes / (4 × (hmtx + cmap bytes)) = 1.09 (`NISC18030.ttf`), under the 4× stop line. No format-12 subtable is unsorted or overlapping (0 of 105) | `python3 -I /tmp/p162/probe.py` (a scratch copy of §4.1 over every face under `/System/Library/Fonts`, `/Library/Fonts`, `~/Library/Fonts`, `/System/Library/AssetsV2`) |

### Verified properties

- *The record has room for one more pointer — at offset 48, not 40.* Verified
  2026-10-14 by reading `gen_font.rs` and `error_constants.rs`: the canonical header
  grew an owner word (`RESOURCE_OFFSET_OWNER` = 32, bug-755), so
  `RESOURCE_TAIL_START` = 40 (`const _: () = assert!(RESOURCE_TAIL_START == 40)`) and
  `FONT_BYTES = RESOURCE_TAIL_START` = **40**. The face block goes at
  `FONT_FACE = RESOURCE_TAIL_START + 8` = **48**. `grep -rn "40\b\|48\b\|FONT_FACE"
  src/codegen/builtins/canvas/gen_font.rs src/codegen/builtins/canvas/func_*font*.rs` →
  only `FONT_BYTES`'s own asserts and unrelated MFBASIC literals (`count > 4096`,
  `rec + 8`), so nothing stores at 48 for `RESOURCE_TAG_FONT`. (Correction 1.)
- *Who frees `FONT_BYTES`, and when: nobody, ever.* Verified 2026-10-14:
  `func_destroy_font.rs:lower_destroy_font` runs `emit_unregister_font`, then sets
  `closed@16`, and its doc says "The font bytes are arena-owned and reclaimed with the
  arena; there is no OS handle to give back". The scope-drop of a `RES canvas::Font`
  routes to the same `destroyFont`. Group retirement (`__canvas_closeRetired`, R17b)
  calls `destroyFont` too. `.ai/canvas-threading.md` §7 states the rule: "**Close
  never frees.** … the image's record and its pixel blocks are never freed by anyone",
  which is what makes a close safe while the graphics thread is mid-frame. So the face
  block follows exactly that path: it is never freed, like the bytes and the record.
  (Correction 2.)
- *What reads geometry-record slot 2 (the run's font handle) for `__CANVAS_GEO_TEXT`:
  nothing.* Verified 2026-10-14. `grep -rn "GEO_TEXT" src/codegen/builtins/canvas/*.rs`
  lists every text-record consumer: `helper_glyph_cache.rs` (pins and remaps read slot
  20 and the run), `helper_items.rs:529` (draw: slot 20, colour slots 8–11, the run),
  `helper_render.rs` (`__canvas_blockInstances` slot 20, `__canvas_drawsJoin` slot 0,
  `__canvas_blockCost` via `__canvas_runSamples`), `helper_hit.rs` (the text box at
  `__CANVAS_GEO_TEXT_BOX`). Natively, `HEADER_SHAPE` (= 2) is read only by
  `ITEM_RESIDUAL_FIELDS` in `src/codegen/runtime/canvas/mod.rs`, which the item-block
  packer uses, and both GPU publishers branch a text record away before that packer
  (`metal.rs`: `compare_immediate(SCRATCH[1], GEO_KIND_TEXT)` → `text_item`, ahead of
  `emit_item_block`; `vulkan.rs:8172` likewise). So letter C may write fonts[0]'s
  handle there.

## 3. Design Overview

Three pieces:

1. **Build.** `__canvas_faceIndex(bytes AS List OF Byte) AS List OF Integer` is
   MFBASIC in `helper_font.rs`. It runs **once**, inside `__canvas_loadFontBytes` on
   the worker, over the bytes the load already holds. It reuses the existing readers
   unchanged, so the index says exactly what the per-call readers say today.
2. **Own and publish.** `canvas::fontFromBytes(face, index)` stores the index block at
   record offset 40 (`FONT_FACE`), and `emit_register_font` publishes
   `(handle, bytes, index)`: the slot grows from 16 to 24 bytes. `destroyFont` and
   retirement free the index on exactly the path that frees the bytes.
3. **Read in place.** `canvas::faceAt(id AS Integer, i AS Integer) AS Integer` is
   native and `internal_only`. It scans the table like `fontRegistered`, bounds-checks
   `i` against the block's length, and loads the word. It never allocates and never
   hands out the block. An unregistered handle or an out-of-range index answers
   `0`. Callers check `canvas::fontRegistered` once per run, as `__canvas_fontBlob`
   does today, so `0` is never mistaken for data.

Then `__canvas_shapeRun`, `__canvas_measureText` and `__canvas_glyphRunHeader` switch
from byte readers to index readers (`__canvas_faceGlyph`, `__canvas_faceAdvance`,
`__canvas_faceMetric`).

**Correctness gate:** this letter is provably neutral, so byte-identical PNG goldens
plus unchanged `rt_canvas_*` results are the gate. `tests/syntax/app/app-mouse-surface/golden/*`
is **expected** to move, because it is the canvas change sentinel
(`.ai/testing-gates.md`).

**Where design uncertainty concentrates (scheduled first, Phase 1):**
- the index size for real fonts (CJK fonts have tens of thousands of glyphs);
- the free path for `FONT_BYTES`;
- whether a builtin record field can be `List OF RES canvas::Font`. That is letter C's
  premise, but it takes minutes to test, so it is tested here, first.

**Where correctness risk concentrates (scheduled last):** the free/retire path of the
index block across threads. A use-after-free there is silent on the software path and
can crash the graphics thread.

Rejected alternatives:
- **Per-thread lazy cache in MFBASIC globals.** Rejected: it is built twice (worker and
  graphics), keyed by a reusable address, and has no invalidation hook on `destroyFont`.
- **Hand MFBASIC the resource's own byte block.** Rejected: that is the exit-139 bug
  (`fontBytes` history).
- **Native parsing of `cmap`/`hmtx`/`GPOS` in emitted IR.** Rejected: it duplicates
  already-correct MFBASIC readers in a far harder medium. Only the *read* primitive
  needs to be native.
- **Store the index as a separate RES.** Rejected: the user-facing resource stays one
  opaque `Font`, and the cache is its implementation.

## 4. Detailed Design

### 4.1 Face index layout (version 1)

A flat `List OF Integer`. Header words, then sections addressed by offsets stored in
the header:

| Word | Meaning |
|---|---|
| 0 | layout version = 1 (B, D and F bump it and append sections) |
| 1 | `unitsPerEm` (0 when the font has no `head`, so all metrics read 0, matching HEAD) |
| 2 | ascent (`hhea.ascender`, font units) |
| 3 | descent as stored (negative, `hhea.descender`); readers flip it, as today |
| 4 | lineGap |
| 5 | number of cmap runs `R` |
| 6 | offset of cmap runs |
| 7 | number of advances `H` (`numberOfHMetrics`) |
| 8 | offset of advances |

- **cmap runs:** `R` triples `(firstCp, lastCp, delta)`, sorted by `firstCp` and
  non-overlapping. `gid = cp + delta` for `firstCp <= cp <= lastCp`, and any codepoint
  outside every run maps to 0. They are built by walking the subtable
  `__canvas_cmapSubtable` chooses:
  - format 12: one run per group;
  - format 4: one run per maximal span of consecutive codepoints whose
    `__canvas_cmap4` result has a constant `gid - cp` and is non-zero. This is computed
    by calling `__canvas_cmap4` itself, so `idRangeOffset` segments flatten exactly as
    they read today.

  Lookup is a binary search.
- **advances:** `H` words, the `hmtx` advance pairs' advance column. A glyph `>= H`
  takes word `H - 1`, which is `__canvas_glyphAdvance`'s shared-trailing-advance rule.

### 4.2 Readers (MFBASIC, `helper_font.rs`)

- `__canvas_faceGlyph(font AS Integer, cp AS Integer) AS Integer`: binary search over
  the runs via `canvas::faceAt`.
- `__canvas_faceAdvance(font AS Integer, gid AS Integer) AS Integer`.
- `__canvas_faceMetric(font AS Integer, word AS Integer) AS Integer` for words 1–4.

`__canvas_shapeRun` uses the first two. `measureText` and the run header use the third.

### 4.3 Threading and lifetime

- The index is allocated on the worker (inside `loadFont`) and only **read** elsewhere,
  in place, by native code. This is allowed by §2's corollary: reading another
  thread's block is fine, allocating into another thread's arena never is.
- It is freed wherever and whenever `FONT_BYTES` is freed, by the same code.
- `faceAt` rescans the table on each call. That is a 16-slot scan, the same cost
  `fontRegistered` already pays. A per-run cached slot index is a possible follow-up,
  measured before it is added.

## Compatibility / Format Impact

- **Changed (internal only):** the font-table slot stride (16 → 24 bytes), the `Font`
  record's offset 40, and `canvas::fontFromBytes`'s arity.
- **Removed:** nothing public.
- **Unchanged:** every public signature, man page, pixel, and error.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work. `- [~]` = partial, with one line on what remains. Moot tasks become
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` the moment a phase lands. An
> unticked box means NOT DONE.

### Phase 1 — measure and falsify (no production code)

Settles the three unknowns before anything lands.

- [x] Re-run the prerequisite commands and update the table. All three MET
      (2026-10-14, commands and hits in the table).
- [x] Record the font-byte fetch census (the Measured populations row) with its command
      and output. 8 sites, not the 3–4 the plan assumed (Correction 3).
- [x] Read `func_destroy_font.rs` and the retirement path, and write here who frees
      `FONT_BYTES`, on which thread, and behind which gate (§7, R17b). Nobody frees it
      (§2 Verified properties, Correction 2).
- [x] `grep -rn "40\b\|FONT_FACE" src/codegen/builtins/canvas/gen_font.rs src/codegen/builtins/canvas/func_*font*.rs`:
      confirm offset 40 is unused for fonts, and record the result. 40 is now
      `FONT_BYTES` itself; 48 is the free word (Correction 1).
- [x] Face-index size probe: a `/tmp` program that loads every
      `canvas::listSystemFonts()` face, builds the index with a scratch copy of
      §4.1's algorithm, and prints `len(index)` and load time per face. Record the max
      and the median here. If the largest index exceeds 4 × its font's `hmtx` + `cmap`
      byte size, stop and revise §4.1 before Phase 2 (a design correction, recorded
      in Corrections).
      Result: `python3 -I /tmp/p162/probe.py` → 638 TrueType faces, median 997 words,
      max 122,707 (STHeiti Light, face 2), worst ratio 1.09 < 4, so §4.1 stands. Load
      time is measured on the real builder in Phase 2 (Correction 4).
- [x] Letter C premise: a `/tmp` app program declares a local record with a
      `List OF RES canvas::Font` field, builds a `canvas::Text`-like value from two
      loaded fonts, presents a scene containing it, and destroys the fonts. Record
      whether it compiles, whether `present`'s deep copy aliases the handles (no
      double close), and any diagnostic. If it fails, file a bug and add it to
      the Prerequisites as gating C.
      Result: `/tmp/p162/cpremise` (a `TYPE Chain` with `fonts AS List OF RES
      canvas::Font`, filled each frame from `chainOf(a, b)` returning `[a, b]`, each
      font measured through `FOR EACH f IN c.fonts`, a `Text` presented, three frames)
      compiles with no diagnostic and prints `len 2` / `w 15.57` / `w 17.50` per frame.
      Then `canvas::destroyFont(a)` and `(b)` both succeed and the program prints
      `done`, exit 0 (`./target/debug/mfb build -app /tmp/p162/cpremise &&
      MFB_HEADLESS=1 …/cpremise`), so neither the chain's drop nor `present` closed
      them. The builtin-union-variant half is re-checked in C Phase 1.
- [x] Resolve the slot-2 UNVERIFIED property (who reads the run's font handle) and
      record it. Nothing reads it (§2 Verified properties).

Acceptance: every UNMEASURED and UNVERIFIED item above has a recorded value and the
command behind it.
  Check: `grep -c "UNMEASURED\|UNVERIFIED" planning/plan-162-A-font-face-index.md` →
  counts only this Phase 1 text and the Corrections section (est. 1 min).
  Result: → 3, all three in this phase's own task/acceptance text.
Commit: 91f130e1e

### Phase 2 — build and own the index (no reader uses it yet)

- [x] `helper_font.rs`: add `__canvas_faceIndex(bytes)` per §4.1, reusing
      `__canvas_cmapSubtable`/`__canvas_cmap4`/`__canvas_cmap12`/`__canvas_glyphAdvance`/metric
      readers. `__canvas_cmap4`'s segment body became `__canvas_cmap4Segment` and
      `__canvas_cmap12`'s bound became `__canvas_cmap12Groups`, so the builder and the
      per-call lookups share them (Correction 7). The probe's sizes are reproduced
      exactly: Arial 5,361 words, STHeiti 122,707 (`MFB_CANVAS_FACE_DUMP` on
      `/tmp/p162/dump`), and that two-font program ran in `perf.program` 97.7 ms.
- [x] `gen_font.rs`: add `FONT_FACE = RESOURCE_TAIL_START + 8` (= 48, Correction 1), with the same compile-time asserts as `FONT_BYTES`.
- [x] `func_load_font.rs`: `__canvas_loadFontBytes` builds the index and calls
      `canvas::fontFromBytes(face, index)`. `lower_font_from_bytes` stores it at `FONT_FACE`.
- [x] `gen_font_table.rs`: stride 24, `emit_register_font(handle, block, face)`, and
      update `emit_unregister_font` and every slot scan to the new stride. Every scan
      already steps by `CANVAS_FONT_SLOT_BYTES`, now 24 with `CANVAS_FONT_SLOT_FACE` = 16
      (`error_constants.rs`).
- [x] ~~Free the index on the `FONT_BYTES` free path (Phase 1 finding).~~ — moot: nothing frees `FONT_BYTES` (`func_destroy_font.rs:lower_destroy_font` doc, `.ai/canvas-threading.md` §7 "Close never frees"), so following its path means the index is never freed either. The Phase 2 live-bytes test pins that it is the only thing left behind.
- [x] Tests (`tests/canvas/rt_canvas_font.rs`): the index of the fixture font equals
      a hand-computed list, reached through an internal-only debug read in a `--debug`
      build, following the `MFB_CANVAS_STATS` precedent. A font with no `head` yields
      word 1 = 0. `face_index_of_the_fixture_is_its_own_tables` and
      `face_index_of_a_font_with_no_head_has_no_units_per_em`, through the new
      `MFB_CANVAS_FACE_DUMP` (`helper_font.rs:FACE_DUMP_DEBUG`, spec
      `tooling/09_debug-report.md`) → ok.
- [x] Tests: `tests/canvas/rt_canvas_present_leak.rs`-style load/destroy loop over
      `arena.0.live_bytes`. Memory cannot stay flat, because a font's bytes, record and
      index are never freed (Correction 2). The check is that each cycle leaves exactly
      those three blocks behind and nothing else: per-cycle growth between N and 2N cycles
      ≤ the font's byte length + 8 × the index's word count + one record + the
      collection headers and allocator rounding. That catches a transient leak in the
      builder, which is the real risk.
      `a_load_destroy_cycle_keeps_only_the_fonts_own_blocks` asserts it **exactly**: the
      growth over 40 extra cycles is 40 × 656 bytes (352 bytes block, 192 index block,
      112 record; Correction 6) → ok. The same program over Arial
      (`/tmp/p162/cycles.sh`) leaves 773,392 bytes a cycle at `main` and 816,320 here;
      the difference, 42,928, is exactly 40 + 8 × 5,361, Arial's index block.

Acceptance: the index is built and owned, it rides the bytes' never-freed lifetime and
leaves nothing else behind per load, nothing reads it, and pixels are unchanged.
  Check: `cargo test --test rt_canvas_font --test rt_canvas_present_leak --test rt_canvas_golden`
  → all pass (est. 5 min).
  Result: 36 + 28 + 6 passed, 0 failed (`/tmp/p162/phase2-accept.log`), and
  `git status --short tests/golden/canvas` → empty.
Commit: (Phases 2 and 3 landed in one commit, Correction 8)

### Phase 3 — the in-place accessor

- [x] `gen_font_table.rs`: `canvas::faceAt(id, i) AS Integer`, `internal_only: true`,
      `errors: []`, and a doc comment carrying the no-aliasing rule and its exit-139
      history. `lower_face_at`; `emit_lookup_font` takes the slot word to read. Added
      to every backend's `RUNTIME_CALLS`.
- [x] `helper_font.rs`: `__canvas_faceGlyph`, `__canvas_faceAdvance`,
      `__canvas_faceMetric` per §4.2.
- [x] Tests: for every codepoint 0..0x10FFFF, excluding surrogates, in the fixture font
      and in one real system font from the Phase 1 probe, `__canvas_faceGlyph` equals
      `__canvas_glyphIndex`. Likewise `__canvas_faceAdvance` equals
      `__canvas_glyphAdvance` for every glyph ID. Run it as a `--debug` scratch
      program in the test.
      The sweep is `__canvas_faceCheck` (`MFB_CANVAS_FACE_CHECK`, a `--debug`-only hook in
      `measureText`), over every scalar 0..0x10FFFF except surrogates and every glyph id
      0..max(65535, highest mapped). Run on: the fixture; a format-4 fixture with delta,
      wrapping, `idRangeOffset` and shadowed segments; an unsorted, overlapping
      format-12 fixture (the painted path); and Arial (format 4, 657 runs) and Noto Sans
      Bamum (format 12). Each test also pins the exact runs where the shape is
      hand-computable. All report `glyphMismatches=0 advanceMismatches=0
      metricMismatches=0` (Arial: `mapped=2792 glyphIds=65536 runs=657`, 2.7 s). STHeiti
      is not in the test: its byte-reader side is a linear scan of 23,495 groups per
      codepoint, which is the cost the index removes, and ran more than 10 min.

Acceptance: the index answers exactly what the byte readers answer, for every input.
  Check: `cargo test --test rt_canvas_font face_index` → pass (est. 3 min; the
  full-range sweep is the only check that catches a run-merging off-by-one).
  Result: 6 passed, 0 failed, 76 s.
Commit: (with Phase 2, Correction 8)

### Phase 4 — switch the readers (blast radius last)

- [ ] `__canvas_shapeRun` (`helper_font.rs`): glyph and advance through the index. It
      no longer calls `__canvas_fontBlob`.
- [ ] `__canvas_measureText` (`func_measure_text.rs`): metrics through
      `__canvas_faceMetric`, and the `__canvas_fontBlob` metrics read added by bug-756
      is removed. The closed guard (`canvas::fontOpenHandle`) stays.
- [ ] `__canvas_glyphRunHeader` (`helper_geometry.rs`): ascent/descent through the index.
- [ ] (Correction 3) The other metrics readers through the index:
      `helper_hit.rs:__canvas_textBoxHeader`, `helper_particles.rs:__canvas_particleTextBox`,
      `helper_autopaint.rs:__canvas_autoTextBox` and `__canvas_geoRunReference` stop
      fetching the blob. `__canvas_textGlyphRun` and `__canvas_textEdges` keep theirs for
      outlines only and take `unitsPerEm` from the index.
- [ ] Delete `__canvas_glyphIndex`/`__canvas_cmapSubtable`'s per-call path if nothing
      calls them any more. They stay only as the index builder's readers, so they
      move into the builder's doc scope, and nothing is suppressed.
- [ ] `src/docs/spec/app/06_canvas.md`: add a subsection "A font is parsed once" that
      cites `[[src/codegen/builtins/canvas/helper_font.rs:__canvas_faceIndex]]` and
      `[[src/codegen/builtins/canvas/gen_font_table.rs:faceAt]]`, and states the
      in-place read rule.

Acceptance: pixels and metrics are byte-identical, and the only font-byte fetches left
are for outlines.
  Check: `cargo test --test rt_canvas_font --test rt_canvas_golden --test rt_canvas_geo_native`
  → pass, and `git status --short tests/golden/canvas` → empty (est. 6 min). Then the
  §1 grep lists only the outline readers.
Commit: —

## Validation Plan

- Tests: Phase 2's index/leak tests, Phase 3's exhaustive equivalence sweep, and the
  existing `rt_canvas_font` closed-font and zero-`head` cases.
- Coverage check: `tests/golden/canvas/transforms.png` draws `Text`
  (`grep -n "canvas::Text" tests/canvas/rt_canvas_golden.rs`), so text is in the PNG
  gate. The fixture font has one glyph, so the Phase 3 sweep over a real font is what
  actually covers multi-run cmaps.
- Runtime proof: dump one real-font frame before and after (`MFB_CANVAS_DUMP` +
  `MFB_CANVAS_SYNC=1`, `--debug` build) and `cmp` → identical.
- Doc sync: `06_canvas.md` (Phase 4). No man-page change, since no public surface
  moves. Gate: `cargo build && cargo test --bin mfb spec`.
- Final gate (once, after Phase 4): `cargo test` (full), `scripts/artifact-gate.sh`
  (expect only `app-mouse-surface` to move, then regenerate it), and
  `scripts/test-canvas-vulkan.sh` on the Vulkan box per `.ai/remote_systems.md`
  (est. 40 min total. Nothing smaller covers the GPU text path).

## Open Decisions

- `faceAt`'s answer for an out-of-range read — `0` with a once-per-run
  `fontRegistered` guard (recommended, matching `__canvas_fontBlob`'s `[]`) vs. raising
  (rejected: the renderer must never fail a frame on a closed font, §7).
- Cmap run encoding — `(first, last, delta)` triples (recommended, a binary search
  over the IDs and exact for format 12) vs. a dense per-codepoint array (rejected
  pending the Phase 1 size measurement: 1.1 M words per font worst case).

## Corrections

1. **Offset 40 is taken; the face pointer goes at 48.** The plan assumed `FONT_BYTES` =
   32 and a free word at 40. bug-755 added `owner@32` to the canonical header, so
   `RESOURCE_TAIL_START` = 40 = `FONT_BYTES`. `FONT_FACE` is
   `RESOURCE_TAIL_START + 8` = 48, inside the 96-byte envelope. Evidence:
   `error_constants.rs` `assert!(RESOURCE_TAIL_START == 40)`, `gen_font.rs`
   `FONT_BYTES = RESOURCE_TAIL_START`.
2. **Nothing frees a font's bytes, so nothing frees its index.** §4.3 and Phase 2 said
   "freed wherever `FONT_BYTES` is freed". That place does not exist:
   `lower_destroy_font` unregisters and sets the closed flag only, and §7 of
   `.ai/canvas-threading.md` makes "close never frees" the rule. The Phase 2 "memory
   stays flat" test could not pass even at HEAD, since each `loadFont` keeps its bytes
   and record. It is replaced by a stronger, checkable bound: each cycle leaves exactly
   the bytes, the record and the index, and nothing transient.
3. **The census is 8 font-byte fetches, not 3–4.** Besides `__canvas_shapeRun`,
   `__canvas_measureText` and the four geometry builders, three more metrics-only
   readers fetch the whole file: `helper_hit.rs:__canvas_textBoxHeader`,
   `helper_particles.rs:__canvas_particleTextBox`, and
   `helper_autopaint.rs:__canvas_autoTextBox`. The §1 grep covers their files, so §1's
   goal already requires them to move. Phase 4 gains a task for them, and for
   `__canvas_textGlyphRun`/`__canvas_textEdges`/`__canvas_geoRunReference`'s
   `unitsPerEm` reads: those two outline readers keep their blob for outlines but take
   `unitsPerEm` from the index, and `__canvas_geoRunReference` reads no outlines at
   all.
4. **Load time is measured on the real builder, not the probe.** The Phase 1 probe is
   Python, a scratch copy of §4.1's algorithm. It gives exact index sizes but no
   MFBASIC timing. The real `__canvas_faceIndex` load time over the largest face
   (STHeiti) is measured in Phase 2 and recorded there.
5. **The font table is a chain of 16-slot chunks, not one 16-slot table.** bug-756
   replaced the fixed table (`error_constants.rs:CANVAS_FONTS_SYMBOL` doc). The
   non-goal "16-slot capacity unchanged" now reads "chunk geometry unchanged except the
   slot stride (16 → 24 bytes)". Every scan walks `CANVAS_FONT_SLOT_BYTES`, so the
   stride change is one constant, and `CANVAS_FONT_CHUNK_NEXT` follows from it.

6. **The resource record is 112 bytes, not 96.** §2 quoted `RESOURCE_RECORD_SIZE`
   as 96 bytes. `error_constants.rs` has `RESOURCE_RECORD_SIZE: &str = "112"`. The
   live-bytes test budget uses 112, and 16-byte pool rounding (`mfb man debug pools`).
   The first draft of that test used 96 and 8-byte rounding, came out 32 bytes a cycle
   high, and the gap was traced to those two numbers, not a leak (`File` cycles flat,
   `Image` cycles their own blocks only, `main` and this branch differ by exactly the
   index block).
7. **`__canvas_cmap4` and `__canvas_cmap12` were refactored, not only reused.** The
   builder needs format 4's per-segment rule and format 12's group bound without the
   linear search around them, so `__canvas_cmap4Segment` and `__canvas_cmap12Groups`
   were lifted out and both the lookups and the builder call them. Behaviour is
   unchanged (the Phase 3 sweep holds the two together). A format-12 table whose
   groups are unsorted or overlap takes `__canvas_cmap12Painted`, an O(n log n) paint
   over the sorted group boundaries. `collections::sort` cannot be called from canvas
   helpers (`error: NIR call target '#collections_sort' does not resolve`), so the
   merge sort is `__canvas_sortedDistinct`.
8. **Phases 2 and 3 share one commit.** Phase 3's sweep needs Phase 2's dump and index
   in place, and the two touch the same functions in `helper_font.rs` and
   `gen_font_table.rs`. They were verified separately (Phase 2's acceptance, then
   Phase 3's) and committed together. The bug-756 test
   `canvas::tests::canvas_text_has_one_glyph_walk` was extended in the same commit:
   (1) written by bug-756; (2) it protects "only `__canvas_shapeRun` turns
   codepoints into glyph ids or advances"; (3) nothing else depends on it; (4) the
   new `__canvas_faceCheck` calls the byte readers only to compare them with the
   index. The rule is kept and strengthened: `__canvas_faceGlyph` and
   `__canvas_faceAdvance` are now guarded too, and the oracle is named as the one
   exemption.

## Summary

The risk is not the parsing. It reuses the existing readers and is proven equal over
every codepoint. The risk is the index block's lifetime across the worker/graphics
split, which must ride the exact path that frees the font bytes. Everything
user-visible is unchanged, and this letter is the base B, C, D and F build on.
