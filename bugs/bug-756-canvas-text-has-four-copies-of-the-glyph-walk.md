# bug-756: canvas text has four independent copies of the glyph walk; "measure = draw" holds only by coincidence

Last updated: 2026-09-27
Effort: medium (1h–2h)
Severity: LOW
Class: Footgun (duplicated invariant — no wrong output today)

Status: Open
Regression Test: `canvas_text_has_one_glyph_walk` (structural, RED today) +
the Phase 1 guard tests in `tests/canvas/rt_canvas_font.rs` (GREEN today, must stay GREEN)

The `canvas::measureText` documentation promises that "measuring and drawing use the
same glyph walk" (`src/codegen/builtins/canvas/func_measure_text.rs:DESC`). There is no
single walk. Four MFBASIC functions each re-derive the codepoint → glyph → advance
sequence independently: one for measuring, one for filled glyph runs, one for
stroked/transformed text edges, and one for the geometry-verify reference. Today they
agree because the logic in each is identical: decode UTF-32, look up the `cmap` glyph
with no fallback, and add the `hmtx` advance with no kerning. Nothing forces that agreement.

The failure mode is in the future, and it is silent. Every planned text improvement
changes the pen: kerning (`GPOS`/`kern`), font fallback, letter-spacing and ligatures.
Each would have to be added four times, identically. Missing one copy doesn't
fail any build. It produces text whose measured width disagrees with its drawn width.
It can also produce a stroked `Text` item whose glyphs sit somewhere other than the
filled one's, or a verify reference that disagrees with the run it checks. The first
of those breaks every layout built on `measureText`, which is exactly the consumer these
features are for.

**The single correct behavior a fix produces:** exactly one MFBASIC function,
`__canvas_shapeRun`, turns a string plus a font into positioned glyphs. Every text
consumer reads its result and none calls `__canvas_glyphIndex` or
`__canvas_glyphAdvance` on a string itself. Rendered output, measured metrics and
geometry records are bit-identical to HEAD.

References:

- `canvas::measureText` DESC (`func_measure_text.rs:DESC`) — the "same glyph walk" contract.
- `canvas::loadFont` DESC — "no kerning, ligatures or complex shaping", the limits
  this refactor preserves.
- `.ai/testing-gates.md` § "Canvas reference images: exact for software, tolerance
  for GPU" — the PNG golden rule, and the `app-mouse-surface` change-sentinel note.
- Found while discussing HTML/CSS-style layout on the canvas. This is step 0 of the
  text work: kerning, fallback fonts, `measureRun`/`caretStops`, and letter-spacing
  all build on this one helper.

## Failing Reproduction

This is a structural defect with no wrong pixel today, so the reproduction is the count
of independent walks. The command lists every call to the per-glyph primitives outside
their own definitions:

```
grep -rn "__canvas_glyphAdvance(\|__canvas_glyphIndex(" src/codegen/builtins/canvas/ \
  | grep -v "FUNC __canvas_glyph"
```

- Observed: call sites in four functions: `__canvas_measureText`
  (`func_measure_text.rs`), plus `__canvas_textGlyphRun`, `__canvas_textEdges` and
  `__canvas_geoRunReference` (`helper_geometry.rs`). `__canvas_textGlyphRun` calls
  `__canvas_glyphIndex` twice per codepoint, once in each of its two passes.
- Expected: every such call is inside `__canvas_shapeRun`.

Contrast: `__canvas_hashText` (`helper_geometry.rs`) also iterates
`encoding::utf32Encode`, but it hashes **codepoints**, never glyphs or advances. It is
not a walk and must not become one (see Blast Radius).

## Root Cause

Each text path was written against the `helper_font.rs` table readers
(`__canvas_glyphIndex`, `__canvas_glyphAdvance`, `__canvas_fontUnitsPerEm`) directly,
with its own loop, because each needs a different output from the same walk:

| Function | Font bytes from | Output per glyph | Pen arithmetic |
| --- | --- | --- | --- |
| `func_measure_text.rs:__canvas_measureText` | `canvas::fontBytes(font)` (a copy) | nothing; sums the width | `advance` is an **Integer** sum in font units, scaled once at the end |
| `helper_geometry.rs:__canvas_textGlyphRun` | `__canvas_fontBlob(canvas::fontHandle(t.font))` | glyph-cache entry, `toInt(pen + 0.5)`, `toInt(t.y + 0.5)` | Float `pen = pen + toFloat(adv) * scale`, per glyph |
| `helper_geometry.rs:__canvas_textEdges` | same as above | polygon edges via `__canvas_glyphEdges(b, gid, scale, pen, t.y, …)` | same Float accumulation, **unsnapped** pen |
| `helper_geometry.rs:__canvas_geoRunReference` | same as above | the stored entry if its key matches, else -2; snapped pen | same Float accumulation |

The shared step is "codepoint → glyph ID + advance in font units". What differs is
what each function does with the glyph and how it turns units into pixels. Nobody
factored out the shared step because, while it was only `cmap` + `hmtx`, it was two
calls. The duplication costs nothing until the step grows, and every planned text
feature grows exactly that step.

The pen arithmetic is **not** uniform across the four, and a fix has to keep it that
way to stay bit-identical. `__canvas_measureText` sums Integers and multiplies once.
The other three accumulate a scaled Float per glyph. `__canvas_textGlyphRun` and
`__canvas_geoRunReference` snap each glyph to a whole pixel, while
`__canvas_textEdges` does not. These are existing, deliberate differences (glyph-cache
bitmaps are placed on integer pixels, and edges are exact geometry). They are out of
scope here, but they are why the helper must return **font units** and leave
unit→pixel conversion to each caller.

## Goal

- `__canvas_shapeRun(b AS List OF Byte, text AS String) AS List OF Integer` exists
  in `helper_font.rs`. It returns a flat list of `(glyphId, advanceUnits)` pairs, one per
  codepoint of `text`, in order. That is the same flat-list idiom the glyph run
  already uses.
- `__canvas_measureText`, `__canvas_textGlyphRun`, `__canvas_textEdges` and
  `__canvas_geoRunReference` each call `__canvas_shapeRun` once and contain no
  `__canvas_glyphIndex`/`__canvas_glyphAdvance` call.
- The structural test `canvas_text_has_one_glyph_walk` passes. It asserts that, across
  the canvas MFBASIC helper sources, `__canvas_glyphIndex(` and
  `__canvas_glyphAdvance(` are called only inside `__canvas_shapeRun`.
- `tests/golden/canvas/*.png` are byte-identical, and every `rt_canvas_*` suite
  passes unchanged. The Phase 1 guards pass before and after the change.

### Non-goals (must NOT change)

- **Rendered pixels, measured metrics, geometry records.** This is a pure refactor.
  In particular:
  - `measureText` keeps its Integer sum then single scale.
  - The draw paths keep their per-glyph Float accumulation.
  - The fill and verify paths keep per-glyph `toInt(pen + 0.5)` snapping.
  - The edges path stays unsnapped.

  **Tempting wrong fix:** making the helper return pixel pen positions so all four
  "share the arithmetic". That silently changes `measureText` widths by ULPs and
  would have to pick one snapping rule for all paths, which changes pixels. It is
  forbidden here. If unifying the arithmetic is ever wanted, it is a separate bug with
  its own golden proof.
- **No kerning, fallback, letter-spacing or ligatures.** Nothing new is read from the
  font. `__canvas_shapeRun` is exactly today's `cmap` + `hmtx` walk. The later
  features change only its body.
- **Glyph-cache pinning order in `__canvas_textGlyphRun`.** Pass one must still
  rasterise every glyph and pin it in `__CANVAS_GLYPH_PINS` before pass two reads entries
  back (see that function's comments on eviction renumbering). Only the glyph-ID source
  changes: both passes read IDs from the one shaped run instead of calling
  `__canvas_glyphIndex` twice.
- **Font byte source.** `measureText` keeps `canvas::fontBytes` and draw keeps
  `__canvas_fontBlob`. Removing the per-call font copy is a separate step. It is
  latent below, not part of this bug.
- **Public API and man pages.** No registry, `mfb man` or `mfb spec` change. The
  `measureText` DESC sentence becomes literally true and stays as written.
- **`__canvas_hashText`.** It keeps hashing codepoints. It must NOT start hashing
  shaped glyphs, because the geometry cache key has to be computable without
  reading the font.

## Blast Radius

Found with `grep -rn "__canvas_glyphAdvance(\|__canvas_glyphIndex(\|utf32Encode" src/codegen/builtins/canvas/`
and `grep -rln "hmtx\|glyphAdvance\|numberOfHMetrics" src/`. The second command returns
only `helper_font.rs`, `func_measure_text.rs` and `helper_geometry.rs`, so there is no
native (Rust/assembly) text walk. `func_geo_build.rs` declines `Text` and leaves it to the
MFBASIC builders (see its module doc: "`Text` — glyph runs").

- `func_measure_text.rs:__canvas_measureText` — fixed by this bug.
- `helper_geometry.rs:__canvas_textGlyphRun` — fixed by this bug. It also stops calling
  `__canvas_glyphIndex` twice per codepoint.
- `helper_geometry.rs:__canvas_textEdges` — fixed by this bug.
- `helper_geometry.rs:__canvas_geoRunReference` — fixed by this bug. It is the
  `MFB_CANVAS_GEO_VERIFY` reference for `__canvas_textGlyphRun`, so the two drifting
  apart is exactly the failure the verifier exists to catch. Sharing one walk removes
  that risk by construction.
- `helper_geometry.rs:__canvas_hashText` — unaffected. It walks codepoints for a hash
  key and never looks at glyphs. Later features that add `Text` fields (`fallbacks`,
  `letterSpacing`) must extend the **item** hash (`__canvas_hashItem`'s `Text` arm), not
  this function.
- `helper_font.rs:__canvas_faceTable` (`utf32Encode(tag)`) — unaffected. It encodes a
  4-byte table tag.
- `helper_font.rs:__canvas_glyphIndex` re-runs `__canvas_cmapSubtable`, a linear
  scan of the `cmap` records, on every call. That makes it latent per-glyph overhead
  (a performance concern, not correctness). Out of scope because it doesn't change what
  is drawn. Once `__canvas_shapeRun` exists, the natural fix is to look up the subtable
  once per run, which is a one-line follow-up inside the helper.
- `func_measure_text.rs:__canvas_measureText` copies the whole font file per call via
  `canvas::fontBytes` (documented on `fontBytes`: "It copies, and the first version did
  not"). This is latent performance for a layout engine that measures per word. Out of
  scope: it is step 2 of the text work (parse font tables once at `loadFont`), and it
  changes the byte source, which this bug holds fixed.

## Fix Design

1. Add `__canvas_shapeRun` to `helper_font.rs`, beside the readers it composes:

   ```
   FUNC __canvas_shapeRun(b AS List OF Byte, text AS String) AS List OF Integer
     MUT run AS List OF Integer = []
     FOR EACH cp IN encoding::utf32Encode(text)
       LET gid AS Integer = __canvas_glyphIndex(b, cp)
       run = collections::append(run, gid)
       run = collections::append(run, __canvas_glyphAdvance(b, gid))
     NEXT
     RETURN run
   END FUNC
   ```

   It works in font units and never scales. Its doc comment states that it is the
   **only** place text becomes glyphs, and that kerning, fallback and spacing are added
   here and nowhere else.
2. `__canvas_measureText`: `advance` becomes the Integer sum of the odd elements. The
   arithmetic is unchanged (an Integer sum, then `toFloat(advance) * scale`).
3. `__canvas_textGlyphRun`: shape once before pass one. Pass one takes `gid` from the
   run, and so does pass two. Pass two keeps
   `pen = pen + toFloat(adv) * scale` with `adv` from the run. `chars` becomes
   `len(run) / 2`.
4. `__canvas_textEdges` and `__canvas_geoRunReference`: the same substitution. In
   `__canvas_geoRunReference`, the stored-entry offset stays
   `offset + __CANVAS_GEO_HEADER + c * 3` with `c` the glyph index, and the glyph key
   still uses the run's `gid`.

Where the risk concentrates: `__canvas_textGlyphRun`'s two-pass pinning. The shaped run
is a plain local List of Integers, built **before** pass one, and it holds no cache
indices. So eviction during pass one cannot invalidate it, which is the hazard that
function's comments describe for entry indices. That reasoning should go in a comment
at the call.

Rejected alternatives:
- **Return pixel positions from the helper.** Rejected: it breaks bit-identity (see
  Non-goals).
- **Return a `List OF` a record type (`ShapedGlyph[gid, advance]`).** Rejected for
  now. Every neighbouring helper uses flat numeric lists, and the geometry record is
  flat. A record type would be the only one in the text path. If kerning later needs a
  third column (`xPlacement`), make it a triple. That is a local change, because
  every consumer indexes through the helper's stride.
- **Leave `__canvas_measureText` separate "because it has a different byte source".**
  Rejected: the byte source is a parameter, and `b` is the same font file either way.

Expected shift in generated outputs: `tests/syntax/app/app-mouse-surface/golden/*`
(`.ir`, `.nir`, `.nplan`, per-target `.ncodesum`) **will** move. It builds a canvas
program, and per `.ai/testing-gates.md` any change to a canvas MFBASIC helper moves it.
It is a change sentinel, so the move is expected, and it is regenerated after the full
suite runs. No other byte-identity golden builds canvas
(`grep -rln "IMPORT canvas\|IMPORT app" tests/byte-identity/` → none, per the same
doc). **Any** diff in `tests/golden/canvas/*.png` is a bug in the refactor, not a
regeneration.

## Phases

### Phase 1 — failing test + guards + audit (no behavior change)

Coverage today is thin. The only fixture font (`truetype_fixture` in
`tests/canvas/rt_canvas_font.rs`) has one glyph with one advance, and the only
pen test (`a_string_advances_the_pen_between_glyphs`) draws `"AA"` at an advance that
is a whole number of pixels. A refactor that swapped glyph order, read the wrong
column of the pair list, or broke fractional pen accumulation would pass every
existing test. So the guards come first, and they must be GREEN at HEAD.

- [ ] Extend the fixture builder, or add a second fixture, so it provides **two
      mapped glyphs with different advances** that are not whole pixels at the test
      size, plus an **unmapped codepoint** (so the run includes glyph 0).
- [ ] Guard `measureText`: exact `width` for a mixed string such as `"AB?A"` with `?`
      unmapped, compared against the hand-computed Integer-sum-then-scale value.
- [ ] Guard the filled path: the lit-column extents of the same string, pinning every
      glyph's snapped pen.
- [ ] Guard the stroked path (`strokeWidth > 0`, which routes through
      `__canvas_textEdges`): the lit extents of the same string.
- [ ] Guard verify-mode agreement: render the filled string with
      `MFB_CANVAS_GEO_VERIFY=1` and assert zero mismatches, reusing whatever stat
      `rt_canvas_geo_native.rs` already reads for `__CANVAS_GEO_MISMATCHES`.
- [ ] Add the RED structural test `canvas_text_has_one_glyph_walk` as an in-crate
      `#[cfg(test)]` test in the canvas module. It scans the canvas MFBASIC helper
      source constants and asserts that `__canvas_glyphIndex(` and
      `__canvas_glyphAdvance(` appear only inside the body of `__canvas_shapeRun`.
      It fails today with the four functions named in the reproduction.
- [ ] Confirm the audit verdicts above still hold by re-running both grep commands.

Acceptance: the four guards pass at HEAD, and the structural test fails, naming exactly
`__canvas_measureText`, `__canvas_textGlyphRun`, `__canvas_textEdges` and
`__canvas_geoRunReference`.
Commit: —

### Phase 2 — the refactor

- [ ] Add `__canvas_shapeRun` to `helper_font.rs` with its "only place" doc comment.
- [ ] Route `__canvas_measureText` through it (`func_measure_text.rs`).
- [ ] Route `__canvas_textGlyphRun`, `__canvas_textEdges` and
      `__canvas_geoRunReference` through it (`helper_geometry.rs`). Add the pinning
      comment at the `__canvas_textGlyphRun` call.

Acceptance: the structural test passes, all Phase 1 guards still pass, and
`cargo test --test rt_canvas_font` / `rt_canvas_golden` / `rt_canvas_geo_native` pass.
Nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate the sentinel + full validation

- [ ] Run the full suite first, never one module.
- [ ] Confirm `tests/golden/canvas/*.png` are untouched (`git status tests/golden/canvas`
      shows nothing).
- [ ] Regenerate only `tests/syntax/app/app-mouse-surface/golden/*`, following
      `.ai/testing-gates.md` (`sync-goldens.sh` takes the **mfb binary** as its first
      argument). Inspect the `.ir` diff: it must be confined to the four rewritten
      functions and the new helper.
- [ ] `scripts/test-canvas-vulkan.sh` on a Vulkan box (see `.ai/remote_systems.md`) and
      the Metal path locally. GPU text uses the same glyph run, so a regression shows up
      as a tolerance failure.
- [ ] Render one real-font scene before and after (a scratch program in `/tmp` using
      `canvas::systemFonts()`, dumped with `MFB_CANVAS_DUMP` + `MFB_CANVAS_SYNC=1`
      from a `--debug` build) and `cmp` the frames. The fixture font can't show kerning-
      or width-sensitive drift, and a real font can.

Acceptance: full suite green, PNG goldens byte-identical, the sentinel diff confined to
the canvas text helpers, and the real-font frames byte-identical.
Commit: —

## Validation Plan

- Regression test(s): `canvas_text_has_one_glyph_walk` (structural, canvas module), plus
  the Phase 1 guards in `tests/canvas/rt_canvas_font.rs`.
- Runtime proof: before/after `cmp` of a real-font frame dump (Phase 3), and
  `MFB_CANVAS_GEO_VERIFY=1` mismatch count 0.
- Doc sync: none expected. `measureText`'s "same glyph walk" sentence becomes true as
  written, and no spec topic cites the four functions. Confirm with
  `grep -rn "textGlyphRun\|textEdges\|geoRunReference\|glyphAdvance" src/docs/spec/`
  and, if any hit exists, re-point it at `__canvas_shapeRun`.
- Full suite: `cargo test` (full), `scripts/artifact-gate.sh`, and
  `scripts/test-canvas-vulkan.sh`.

## Open Decisions

- Helper output shape — flat `(gid, advanceUnits)` pairs (recommended, matches the
  flat-list idiom) vs. a record list. (§Fix Design)
- Where the structural test lives — in-crate `#[cfg(test)]` over the source constants
  (recommended, no file-path coupling) vs. an integration test that `include_str!`s
  the `.rs` files.

## Summary

The engineering risk is not the helper, which is five lines. It lies in two places:
keeping the four paths' deliberately different pen arithmetic bit-identical, and not
disturbing `__canvas_textGlyphRun`'s eviction-safe two-pass pinning. Today's tests are
too weak to catch a mistake in either (one glyph, whole-pixel advance), so Phase 1's
guards are the real work. Left untouched: font byte sourcing, `cmap` subtable lookup
cost, the hash key, every public API, and every pixel.
