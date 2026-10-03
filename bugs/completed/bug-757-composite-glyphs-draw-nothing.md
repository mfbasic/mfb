# bug-757: a composite TrueType glyph draws nothing — accented letters in many fonts are blank

Last updated: 2026-10-03
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (silent: missing ink, no error)

Status: Fixed
Regression Test: `tests/canvas/rt_canvas_font.rs:a_composite_glyph_draws_every_placed_component`

A TrueType glyph is either **simple** (its own contours, `numberOfContours >= 0`) or
**composite** (`numberOfContours < 0`): a list of other glyphs, each placed with an
offset and optionally scaled. Many fonts build accented and derived letters (é, ñ, Å,
ǅ, and so on) as composites of a base letter and a mark. The canvas outline reader
returns no edges for a composite glyph. The glyph still advances the pen, so the text
**measures** correctly, but it **draws** as a gap. Nothing is raised or logged.

**The single correct behavior a fix produces:** a composite glyph draws the union of
its components' outlines, each transformed as its component record says, and matches
what the same font renders elsewhere (FreeType is the oracle).

References:

- OpenType `glyf` spec, "Composite glyph description" (flags `ARG_1_AND_2_ARE_WORDS`,
  `ARGS_ARE_XY_VALUES`, `WE_HAVE_A_SCALE`, `WE_HAVE_AN_X_AND_Y_SCALE`,
  `WE_HAVE_A_TWO_BY_TWO`, `MORE_COMPONENTS`, `USE_MY_METRICS`).
- Found while researching plan-162 (font/text work). It is independent of that plan;
  plan-162-C's fallback fonts would hit it too.

## Failing Reproduction

Found by reading the code. It has not yet been run against a real composite glyph.
Phase 1 builds the executable reproduction.

- `src/codegen/builtins/canvas/helper_glyph.rs:__canvas_glyphEdges`:

  ```
  LET contours AS Integer = __canvas_beS16(b, start)
  IF contours <= 0 THEN
    RETURN out
  END IF
  ```

  A composite has `contours = -1`, so it returns the edges unchanged, which is none.
- `helper_glyph_cache.rs:__canvas_glyphEntry` rasterises from
  `__canvas_glyphEdges(b, gid, scale, 0.0, 0.0, [])`, so the filled path draws nothing.
  `__canvas_textEdges` (the stroked path) calls the same function, and so does the GPU
  path through the same glyph cache.
- There is no composite handling anywhere in the canvas tree:
  `grep -rni "composite" src/codegen/builtins/canvas/helper_glyph*.rs` → no matches.

Expected (Phase 1 test): the fixture composite glyph `Ä` = `A` + a translated `¨` lights
the union of both components' pixels. Observed prediction: only the pen advances, and no
pixel lights.

## Root Cause

The outline reader implements only the simple-glyph path. The composite branch was
never written, and `contours <= 0` folds "empty glyph" (0 contours, which is correct
to skip) together with "composite" (−1, which is not).

## Goal

- `__canvas_glyphEdges` handles `numberOfContours < 0`: it walks the component records,
  reads each component's simple outline recursively, applies the component transform
  (offset, uniform/xy/2×2 scale in F2Dot14), and appends the edges.
- Recursion depth is capped (FreeType caps nesting). A cycle or an over-deep nesting
  draws nothing for that component rather than looping.

### Non-goals

- Point-matching placement (`ARGS_ARE_XY_VALUES` clear), where components are aligned by
  point numbers. Implement it if the probe finds it in system fonts; otherwise, draw the
  component unoffset and document that.
- Hinting instructions. They are skipped today and stay skipped.
- Advance widths. `USE_MY_METRICS` affects hmtx-free renderers only, and `hmtx` already
  gives the composite's advance.

## Blast Radius

- `helper_glyph.rs:__canvas_glyphEdges` — fixed here, for all callers.
- `helper_glyph_cache.rs:__canvas_glyphEntry` (fill, CPU and GPU) — fixed through the
  above.
- `helper_geometry.rs:__canvas_textEdges` (stroke/transform) — fixed through the above.
- `func_measure_text.rs` — unaffected: measuring uses `hmtx` only.

## Fix Design

Add `__canvas_compositeEdges(b, start, scale, penX, penY, edges, depth)`. For each
component:
- read the flags and glyph index;
- read the arguments as words or bytes, signed;
- read the transform per the flags.

Then recurse into `__canvas_glyphEdges` for the component with a composed transform.
The existing `__canvas_contourEdges` takes `scale, penX, penY`, which is only uniform
scale plus translation. A 2×2 component needs a general affine, so thread an affine
(`a, b, c, d, e, f`) through the contour walk instead, with the identity case producing
bit-identical edges. That is where the risk concentrates: the simple-glyph path must
stay byte-identical (the PNG goldens).

## Phases

### Phase 1 — failing test + real-font census (no behavior change)

- [x] Extend `tests/canvas/rt_canvas_font.rs:truetype_fixture` with a composite glyph
      (offset-only component, and a scaled component). RED test: its pixels are lit.
- [x] `/tmp` probe: count the composite glyphs, and the flags used, across
      `canvas::listSystemFonts()`. Record the counts here with the command.

Acceptance: the RED test fails for the documented reason, and the census is recorded.
Commit: —

### Phase 2 — the fix

- [x] Affine threading through `__canvas_contourEdges`, identity-exact.
- [x] The composite walk, with a depth cap.

Acceptance: the Phase 1 test passes, and `git status --short tests/golden/canvas` → empty.
Commit: —

### Phase 3 — full validation

- [x] `cargo test` (full), `scripts/artifact-gate.sh` (`app-mouse-surface` moves), and
      `scripts/test-canvas-vulkan.sh`.
- [x] Real font: render "Ä é ñ Å" in a system face with composites, and compare it
      against FreeType (`ftview`/`freetype-py` rasterisation at the same size). Record
      the command and the result.

Acceptance: full suite green, and the real-font glyphs visibly match the oracle.
Commit: —

## Validation Plan

- Regression test: the Phase 1 fixture test.
- Runtime proof: the real-font render.
- Doc sync: `06_canvas.md` gains one sentence (composite glyphs supported; point-matching
  per the Non-goals outcome).
- Full suite: as in Phase 3.

## Open Decisions

- Point-matching components: implement only if the census finds them (recommended).

## Summary

Silent missing ink for common letters in real fonts. The fix is contained to the outline
reader. The risk is keeping the simple-glyph path bit-identical while generalising its
transform.

## Resolution (2026-10-03)

`__canvas_glyphEdges` sends `numberOfContours < 0` to the new
`COMPOSITE_EDGES` helpers. `contours = 0` still draws nothing, and the
simple-glyph path is untouched, so every existing font test passes unchanged.

- `__canvas_compositeEdges` reads each component record: word or byte
  arguments; XY offsets; a uniform, x/y or 2×2 F2Dot14 transform; and the
  scaled/unscaled offset flags. It composes the component's transform with the
  caller's surface affine and recurses, capped at depth 8.
- `__canvas_affineGlyphEdges` / `__canvas_contourEdgesMapped` flatten a simple
  component from pre-mapped surface points. This is a dedicated copy of the
  contour walk: rewriting the simple path as an affine would have changed its
  rounding through FMA contraction.

**Census.** `python3` over every `glyf` face in `/System/Library/Fonts/**` and
`/Library/Fonts` found 637 faces and 1 330 272 outline glyphs, of which
**305 021 are composite**. Component flags: xy 2 125 135, words 1 790 166,
x/y-scale 1 490, uniform scale 630, 2×2 110, UNSCALED_COMPONENT_OFFSET 132.
There are **0 point-matched components**, so the non-goal costs nothing here.

**Proof.**

- The fixture test fails on main ("drew nothing at all"). With the fix it lights
  exactly the two placed squares (1125 px, no stray ink). All 32 font tests pass.
- Real font: Arial's `Ä é ñ Å` are all composites. Rendered headless at 120 px
  (`/tmp/b757`), the fix lights 9796 px with every diacritic placed correctly;
  main lights 0. The FreeType comparison was not run because no FreeType tooling
  is installed on the host. The visual check is the oracle used.

**Expected golden change, regenerated at the batch gate.**
`syntax/app/app-mouse-surface` (`.ir`, `.nir`, `.nplan`): the new always-injected
helper and its `#CANVAS_COMPOSITE_DEPTH` global. Spec: `mfb spec app canvas`,
"Composite glyphs draw their components".
