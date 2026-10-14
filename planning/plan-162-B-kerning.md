# plan-162-B: kerning — GPOS pair adjustment and the legacy `kern` table

Last updated: 2026-09-27
Effort: large (3h–1d)
Depends on: plan-162-A (prerequisites: see plan-162-A §Prerequisites)

Today every glyph advances by its `hmtx` width alone. The `canvas::measureText` man page
says so ("This build does no kerning"). This letter reads a font's pair kerning once,
into the face index, and applies it in `__canvas_shapeRun`. Because every text path
goes through that helper (bug-756), measuring and drawing both pick it up together.

**The single behavioral outcome:** for a font with kerning, the pen after a kerned pair
moves by `advance(left) + kern(left, right)` font units, identically in
`measureText`'s `width` and in the drawn position of `right`. A font with no kerning
draws byte-identically to HEAD.

References:

- plan-162-A (face index layout, §4.1; version word; `canvas::faceAt`).
- OpenType spec: `GPOS` header, ScriptList/FeatureList/LookupList, lookup type 2
  (PairPos formats 1 and 2), lookup type 9 (Extension), Coverage formats 1 and 2,
  ClassDef formats 1 and 2, ValueRecord/ValueFormat. Also the TrueType `kern` table,
  version 0, subtable format 0.
- `func_measure_text.rs:DESC` and `func_load_font.rs` DESC. Both state "no kerning"
  today and must be updated.

## 1. Goal

- The face index gains a **kerning section** (layout version 2) built at load.
- `__canvas_shapeRun` adds each adjacent same-font pair's adjustment to the left
  glyph's advance.
- A kerned pair in a fixture font measures and draws at the kerned distance, on both
  the fill and stroke paths, and on the GPU.
- The man pages for `measureText` and `loadFont` describe kerning as done.

### Non-goals

- **No contextual or positional adjustments.** No mark attachment (types 4–6), cursive
  attachment (type 3), chained or contextual positioning (types 7–8), or single
  adjustment (type 1). Only pair `XAdvance` is applied.
- **No vertical kerning. No `YAdvance`, `XPlacement` or `YPlacement`.** They are read
  only to skip them correctly (ValueRecord sizing).
- **No per-script or per-language feature selection.** The `kern` feature is taken
  from the `DFLT` script's default LangSys if present, else from the `latn` script's
  default LangSys, else from the first script. This is recorded as an Open Decision.
- **No kerning across a font change.** In letter C, a pair whose glyphs come from two
  different fonts is not kerned.
- **No change for fonts without kerning.** Their pixels are byte-identical.

## 2. Current State

- `__canvas_shapeRun` (bug-756, body per plan-162-A) returns
  `(fontIndex, glyphId, advanceUnits)` triples with no pair term.
- No `GPOS` or `kern` reader exists:
  `grep -rn "GPOS\|\"kern\"" src/codegen/builtins/canvas/` → no matches (measured
  2026-09-27 as `grep -rhoE "\b(kern|GPOS)\b" src/codegen/builtins/canvas/` → nothing).
- `__canvas_faceTable(b, dir, tag)` (`helper_font.rs`) locates any table by tag. That
  is the entry point for both readers.
- Fixture font: `tests/canvas/rt_canvas_font.rs:truetype_fixture` has one outline
  glyph and no `GPOS`/`kern` table. Kerning needs a new fixture.

### Measured populations

| What | Count | Command |
|---|---|---|
| System fonts carrying `GPOS` / `kern` | Of 638 TrueType faces: 355 have `GPOS`, 250 of them a `kern` feature (DFLT → latn → first script); 267 have a `kern` table, 133 of those with no `GPOS` `kern`. `kern` versions: 211 version 0, 56 Apple version 1. Every version-0 subtable is format 0 with coverage low byte 1 (horizontal only): 231 subtables. `kern`-feature lookup shapes (type, format, via Extension) → faces: (2,1) 181, (2,2) 124, (2,1,ext) 16, (2,2,ext) 15, plus non-pair types the letter ignores ((8,·) 50, (4,1) 9, (7,2) 3, (1,1) 2). 158 faces use more than one pair subtable | `python3 -I /tmp/p162/probe.py` over `/System/Library/Fonts`, `/Library/Fonts`, `~/Library/Fonts`, `/System/Library/AssetsV2`, summarised from `/tmp/p162/probe.json` (2026-10-14) |
| Pair-table sizes (format 1 pairs, format 2 class matrix cells) | max 46,098 format-1 pairs in one face; max 1,129,208 format-2 class-matrix cells in one face | same probe |

## 3. Design Overview

- **Build (at load, MFBASIC, `helper_kern.rs` new).** `__canvas_kernSection(bytes)`
  returns a list of **pair subtables** in lookup order. Each is flattened to one of two
  shapes:
  - **Pairs (GPOS format 1, and `kern` format 0):** the covered first glyphs, each with
    a sorted list of `(secondGid, xAdvanceAdjust)`.
  - **Classes (GPOS format 2):** a first-glyph coverage, two ClassDefs flattened to
    `(firstGid, lastGid, class)` runs, and a `class1Count × class2Count` matrix of
    `xAdvance` values.
- **Apply (in `__canvas_shapeRun`).** For each adjacent pair from the same font, walk
  the subtables in order. The **first** subtable whose coverage contains the left glyph
  and that has an entry for the right glyph wins. This is the OpenType rule for a
  single lookup's subtables. Across multiple `kern` lookups, adjustments **add**.
- **Precedence.** If the font has a `GPOS` `kern` feature, use it and ignore `kern`.
  Otherwise use `kern` version 0 format 0 subtables whose coverage says horizontal and
  not cross-stream. This follows what FreeType and HarfBuzz do for legacy fonts.

**Gate:** behavior changes by design. Correctness is checked by pixel/metric tests on
a kerning fixture plus a real-font oracle.
- **Expected to change:** any real-font scene with a kerned pair.
- **Must not change:** `tests/golden/canvas/*.png`, because the fixture font there has no
  kerning. A diff there is a bug.
- `app-mouse-surface` sentinel goldens move (helper change).

**Design uncertainty (first):** real-world table shapes, meaning how many fonts use
Extension lookups, format 2, or several subtables. The Phase 1 probe settles it before
the parser is written.

**Correctness risk:** ValueRecord sizing. Skipping a field wrongly desynchronizes every
later record silently, so each ValueFormat bit gets its own test.

Rejected alternatives:
- **Expand format 2 to explicit pairs.** Rejected: a class matrix covering thousands of
  glyphs expands quadratically.
- **Apply kerning in the callers.** Rejected: that recreates the four-copy problem
  bug-756 removed.
- **Read `GPOS` at shape time from bytes.** Rejected: plan-162-A's rule is no font bytes
  outside outlines.

## 4. Detailed Design

### 4.1 Face index v2

- Header word 9: kerning section offset (0 = none).
- Section layout: `[subtableCount, off_0, …]`. Each subtable starts with a kind word:
  - kind 1 (pairs): `[1, coverageRunCount, runs…, pairSets…]`;
  - kind 2 (classes): `[2, coverageRuns…, class1Runs…, class2Runs…, c1, c2, matrix…]`.
- All glyph-set lookups use sorted `(first, last, value)` runs and binary search, the
  same primitive as the cmap runs.
- Version word = 2.

### 4.2 ValueRecord sizing

The size of a ValueRecord in bytes is `2 × popcount(valueFormat & 0x00FF)`. `XAdvance`
is bit `0x0004`, and its offset within the record is `2 × popcount(valueFormat & 0x0003)`.
Device-table offsets (bits `0x0010`–`0x0080`) are counted in the size but never followed.

### 4.3 `kern` table

- Version 0 header: `nTables`. Each subtable has a version, length and coverage word.
- Accept only format 0 with the horizontal bit set and the minimum/cross-stream bits
  clear.
- Pairs are `(left, right, FWORD value)`, already sorted by the spec. Flatten them into
  the kind-1 shape.
- Apple `kern` version 1 (32-bit header) is skipped entirely. This is recorded in the
  man page.

### 4.4 Shaping

In `__canvas_shapeRun`, after computing triple `i` and before appending triple `i + 1`:
if both come from the same font and that font has a kerning section, add
`__canvas_faceKern(font, gidLeft, gidRight)` to triple `i`'s advance. The adjustment is
in font units, so every caller's pixel arithmetic (bug-756) is unchanged.

## Phases

> **NOTE — keep the checkboxes current as you go** (rules as in plan-162-A).

### Phase 1 — measure real fonts

- [x] `/tmp` probe: for each `canvas::listSystemFonts()` face, print whether it has
      `GPOS`, whether `GPOS` has a `kern` feature, the lookup types and subtable
      formats it uses (including Extension-wrapped ones), and whether it has a `kern`
      table and which versions/formats. Record the counts in §2 with the command.
      Done by the shared plan-162 probe (`/tmp/p162/probe.py`); counts in §2.
- [x] Pick the real-font oracle face for Phase 4: one with format 2 through an
      Extension lookup, if any is installed. Record its name and file hash.
      `/System/Library/Fonts/SFNS.ttf` (SF Pro, TrueType outlines, upem 2048): `kern`
      through Extension lookups, formats 1 and 2, 11 pair subtables, 43,926 format-1
      pairs and 24,742 class cells. SHA-256
      `1c81ead87ca36eee7d51dbe816c3202ba8052fc88228a46de930339c4ec1df22`.

Acceptance: §2's UNMEASURED rows hold numbers and commands.
  Check: `grep -c UNMEASURED planning/plan-162-B-kerning.md` → 0 outside this task list
  (est. 1 min).
  Result: → 1, the line above.
Commit: — (with plan-162-A's Phase 4 hash, this commit)

### Phase 2 — fixtures

- [ ] `tests/canvas/rt_canvas_font.rs`: extend the fixture builder with two outline
      glyphs (`A`, `V`) with distinct advances, plus:
  - (a) a `GPOS` with a format 1 pair `A,V = −120` behind the `kern` feature;
  - (b) a `GPOS` with a format 2 class pair, wrapped in an Extension lookup;
  - (c) only a `kern` v0 f0 table with `A,V = −80`;
  - (d) both `GPOS` (−120) and `kern` (−80), to test precedence;
  - (e) a ValueFormat carrying `XPlacement` + `XAdvance` + `YAdvance` + a device
    offset.
- [ ] RED tests: `measureText("AV")` width = `(adv(A) − 120 + adv(V)) × scale` for
      (a), (b) and (e); −80 for (c); −120 for (d). `"VA"` is unkerned in all five.

Acceptance: the fixtures load and the RED tests fail today with the unkerned width.
  Check: `cargo test --test rt_canvas_font kern` → the five width tests fail on the
  kerning term only (est. 3 min).
Commit: —

### Phase 3 — build the kerning section

- [ ] New `src/codegen/builtins/canvas/helper_kern.rs`, registered with the canvas
      helpers: the `GPOS` walk (script/LangSys/feature selection per the Non-goals
      rule), types 2 and 9, Coverage 1/2, ClassDef 1/2, ValueRecord sizing (§4.2), and
      the `kern` v0 f0 reader.
- [ ] `__canvas_faceIndex` appends the section and writes version 2.
- [ ] `__canvas_faceKern(font, left, right) AS Integer` over `canvas::faceAt`.

Acceptance: `__canvas_faceKern` returns the fixture values, and 0 for unkerned pairs.
  Check: `cargo test --test rt_canvas_font face_kern` → pass (est. 3 min).
Commit: —

### Phase 4 — apply in `__canvas_shapeRun`, docs

- [ ] `__canvas_shapeRun`: add the pair term per §4.4.
- [ ] Phase 2 tests go GREEN. Add pixel tests showing the second glyph's lit column
      moves by the kerned amount on the fill path and on the stroke path.
- [ ] Real-font oracle: in the Phase 1 face, `measureText("AVAWATo")` equals the width
      HarfBuzz reports with only `kern` enabled (`hb-shape --features=-liga,-clig,-calt,kern`,
      summed advances, scaled). The expected value is recorded in the test with the
      command that produced it. Skip with a logged reason if the face isn't installed.
- [ ] Man pages: `canvas::measureText` DESC and `canvas::loadFont` DESC. Replace "no
      kerning" with what is applied: pair kerning from `GPOS` `kern`, or the legacy
      `kern` table. Say plainly that complex shaping is still not done. Check with
      `mfb man canvas measureText` and `scripts/man-census.sh --memory-scope`
      → 0 unclassified.
- [ ] Spec `06_canvas.md`: a "Kerning" subsection covering the feature/script selection
      and precedence, citing `helper_kern.rs` symbols.
- [ ] GPU: `scripts/test-canvas-vulkan.sh` on the Vulkan box, plus the Metal
      `rt_canvas_metal` suite locally. Glyph placement is on the CPU, so this confirms
      only that nothing else moved.

Acceptance: kerned pairs measure and draw kerned, unkerned fonts are byte-identical,
and the docs are true.
  Check: `cargo test --test rt_canvas_font --test rt_canvas_golden` → pass and
  `git status --short tests/golden/canvas` → empty (est. 6 min).
Commit: —

## Validation Plan

- Tests: fixtures (a)–(e), the pixel tests, and the HarfBuzz oracle.
- Coverage: the PNG goldens use the kerning-free fixture, so they show only that nothing
  else changed. The new fixtures are the real coverage.
- Runtime proof: the `examples/emoji` caption, rendered before and after with a system
  font that has kerning. The caption width changes by the sum of its pairs'
  adjustments. Record both widths.
- Doc sync: the man pages and the spec, as above.
- Final gate: `cargo test` (full) and `scripts/artifact-gate.sh` (`app-mouse-surface`
  only) (est. 35 min).

## Open Decisions

- Feature source when `DFLT` has no `kern`: `latn`, then the first script (recommended,
  matching HarfBuzz for Latin text) vs. a union of all scripts (rejected: can apply
  kerning meant for another script).
- Apple `kern` v1: skip (recommended, and documented) vs. implement (few fonts that
  carry it lack `GPOS`, to be measured in Phase 1).

## Corrections

1. **The kerning section has lookups that contain subtables, not one flat subtable
   list.** §3 states the rule correctly: "the first subtable … wins" inside a lookup,
   and adjustments add across lookups. A flat `[subtableCount, off…]` list cannot say
   where one lookup ends, so §4.1's layout is corrected to
   `[lookupCount, lookupOff…]`, each lookup `[subtableCount, subtableOff…]`.
2. **A pair adjusts both glyphs, and a second-glyph value record moves the walk on.**
   §4.4 added one term to the left glyph. HarfBuzz, the oracle Phase 4 is held to,
   applies `value2.XAdvance` to the right glyph, and when `valueFormat2 ≠ 0` it skips
   the right glyph as the next pair's left (`PairPosFormat1/2::apply`, `if (len2)
   skippy_iter.idx++`). So each pair entry stores `(second, xAdvance1, xAdvance2)`. A
   class matrix stores the second column only when `valueFormat2` has `XAdvance`. Each
   subtable carries a "skips the second glyph" flag, and the walk is per lookup:
   `i += 2` after such a pair, `i += 1` otherwise. With `valueFormat2 = 0`, which is
   every pair in §2's census bar a handful, this is exactly §4.4.
3. **Format 2 stops its lookup even at a zero value.** Once the left glyph is covered
   and both classes are in range, the subtable applies whatever the matrix holds, 0
   included, and no later subtable of that lookup is tried. Format 1 applies only when
   the right glyph is in the left glyph's pair set. The text in §3, "has an entry for
   the right glyph", reads that way for both formats once a class-0 cell counts as an
   entry.
4. **`__canvas_faceKern` is reached through the `--debug` face check.** No program can
   call an `internal_only` helper. Phase 3's check reads it through
   `MFB_CANVAS_FACE_CHECK`, whose line gains `kern=left:right:x1:x2,…`: every non-zero
   pair among the first 64 glyph ids, which covers a fixture font whole.

## Summary

The format reading is mechanical. The risk is ValueRecord sizing and subtable
precedence, which are silent when wrong, so each has a dedicated fixture. This letter
touches no public signature and changes pixels only for fonts that kern.
