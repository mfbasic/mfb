# bug-757: a composite TrueType glyph draws nothing — accented letters in many fonts are blank

Last updated: 2026-09-27
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (silent: missing ink, no error)

Status: Open
Regression Test: none yet — see Phase 1

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

- [ ] Extend `tests/canvas/rt_canvas_font.rs:truetype_fixture` with a composite glyph
      (offset-only component, and a scaled component). RED test: its pixels are lit.
- [ ] `/tmp` probe: count the composite glyphs, and the flags used, across
      `canvas::listSystemFonts()`. Record the counts here with the command.

Acceptance: the RED test fails for the documented reason, and the census is recorded.
Commit: —

### Phase 2 — the fix

- [ ] Affine threading through `__canvas_contourEdges`, identity-exact.
- [ ] The composite walk, with a depth cap.

Acceptance: the Phase 1 test passes, and `git status --short tests/golden/canvas` → empty.
Commit: —

### Phase 3 — full validation

- [ ] `cargo test` (full), `scripts/artifact-gate.sh` (`app-mouse-surface` moves), and
      `scripts/test-canvas-vulkan.sh`.
- [ ] Real font: render "Ä é ñ Å" in a system face with composites, and compare it
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
