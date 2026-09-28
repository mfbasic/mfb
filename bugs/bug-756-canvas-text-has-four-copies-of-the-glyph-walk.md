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
`__canvas_shapeRun(fonts AS List OF Integer, text AS String)`, turns a string plus an
ordered list of font **handles** into glyphs. Every text consumer reads its result,
and none calls `__canvas_glyphIndex` or `__canvas_glyphAdvance` on a string itself. The
helper takes handles, not font bytes. Rendered output, measured metrics, raised errors
and geometry records are bit-identical to HEAD.

Design decisions this bug is shaped for (settled 2026-09-27, implemented by later
plans, **not** by this bug):

- **Fallback is a list of fonts, and list order is fallback order.** There is no
  separate `fallbacks` field or argument. `canvas::measureText` gains an overload
  taking `List OF RES canvas::Font`, and `canvas::Text` names its fonts as that same
  kind of list. For each codepoint, the first font in the list that maps it wins.
- **Font bytes are not passed around unless truly needed.** The `canvas::Font`
  resource is opaque, so it owns a parse-once cache of its tables. Text code asks the
  handle for glyph IDs and advances. Only the outline and rasterise steps, which read
  `glyf` contours, actually need the bytes.

So `__canvas_shapeRun` takes a list of handles from the start, and returns which font
each glyph came from. The fallback overload and the font cache then change only the
helper's body, never its four callers.

References:

- `canvas::measureText` DESC (`func_measure_text.rs:DESC`) — the "same glyph walk" contract.
- `canvas::loadFont` DESC — "no kerning, ligatures or complex shaping", the limits
  this refactor preserves.
- `.ai/testing-gates.md` § "Canvas reference images: exact for software, tolerance
  for GPU" — the PNG golden rule, and the `app-mouse-surface` change-sentinel note.
- plan-162 (`planning/plan-162-A-font-face-index.md` … `-F-`) — the follow-on work this
  bug is shaped for; this bug is its first prerequisite.
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

The font bytes come from two different places:
- `measureText` goes through `canvas::fontBytes(font)`. This is a guarded copy: its
  lowering (`func_measure_text.rs:lower_font_bytes`) raises `ErrResourceClosed` for a
  closed font.
- The draw paths go through `__canvas_fontBlob(canvas::fontHandle(t.font))`, which reads
  the process-global font table. `loadFont` registers the font there
  (`func_load_font.rs` → `gen_font_table::emit_register_font`), and `destroyFont`
  unregisters it. For an unregistered handle it answers `[]` and never raises.

Both return an owned copy of the same file (`canvas::fontBlobUnchecked`: "The
published font file for a handle, as an owned copy"). So switching `measureText` to
read by handle gives the same bytes. The difference is the closed-font behaviour:
`canvas::fontHandle` doesn't raise (`func_handle_bridge.rs`, empty `errors` list), so a
handle-based `measureText` has to check for a closed font explicitly.

## Goal

- `__canvas_shapeRun(fonts AS List OF Integer, text AS String) AS List OF Integer`
  exists in `helper_font.rs`. `fonts` holds font handles in fallback order. It returns
  a flat list of `(fontIndex, glyphId, advanceUnits)` triples, one per codepoint of
  `text`, in order. `fontIndex` is the position in `fonts` of the first font whose
  `cmap` maps the codepoint to a glyph other than 0. If none does, the triple is
  glyph 0 of `fonts[0]` (that font's `.notdef`), which is exactly what HEAD draws for
  an unmapped codepoint today. The helper reads each font's bytes itself, by handle.
  Callers never pass bytes to it.
- `__canvas_measureText`, `__canvas_textGlyphRun`, `__canvas_textEdges` and
  `__canvas_geoRunReference` each call `__canvas_shapeRun` once, with a one-element
  list, and contain no `__canvas_glyphIndex`/`__canvas_glyphAdvance` call.
- `__canvas_measureText` stops calling `canvas::fontBytes`, which is removed. It gets
  the handle from `canvas::fontOpenHandle`, which raises `ErrResourceClosed` for a
  closed font as HEAD does. Its only remaining byte read is the `head`/`hhea` metrics,
  and that goes away with the font-cache plan.
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
- **No kerning, letter-spacing or ligatures, and no public fallback.** Nothing new is
  read from the font. For a one-element list, `__canvas_shapeRun` is exactly today's
  `cmap` + `hmtx` walk. The fallback *rule* lives in the helper, but no public
  surface can pass a second font until the `measureText` list overload and the
  `Text` font list land. That plan adds the multi-font tests; this bug proves only
  that the one-font case is unchanged.
- **No font cache.** The helper still reads bytes through `__canvas_fontBlob` on each
  call. The parse-once cache on the `canvas::Font` resource is a later plan, and it
  changes only the helper's body.
- **`ErrResourceClosed` from `measureText` on a closed font.** It still raises, as the
  man page says ("measuring it raises `ErrResourceClosed`"). **Tempting wrong fix:**
  letting the closed font fall through `__canvas_fontBlob`'s `[]` and return all-zero
  metrics. That turns a documented error into silent zeros.
- **Glyph-cache pinning order in `__canvas_textGlyphRun`.** Pass one must still
  rasterise every glyph and pin it in `__CANVAS_GLYPH_PINS` before pass two reads entries
  back (see that function's comments on eviction renumbering). Only the glyph-ID source
  changes: both passes read IDs from the one shaped run instead of calling
  `__canvas_glyphIndex` twice.
- **The outline and rasterise steps keep their bytes.** `__canvas_glyphEntry` (in
  `__canvas_textGlyphRun`) and `__canvas_glyphEdges` (in `__canvas_textEdges`) read
  `glyf` contours, which is the case where bytes are truly needed. Those callers keep
  fetching `__canvas_fontBlob` for the font a triple's `fontIndex` names. With a
  one-element list, that is `fonts[0]`, the same font they read today.
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
- `func_measure_text.rs:canvas::fontBytes` (`internal_only: true`) — its only caller
  is `__canvas_measureText` (`grep -rn "canvas::fontBytes" src/` → that one call plus
  the registration). This bug replaces it with `canvas::fontOpenHandle`, which keeps the
  closed guard and drops the copy (Fix Design step 2), so the old member is removed in
  the same commit. Its doc comment (why it copies instead of aliasing, including the
  measured exit-139 segfault) is a durable lesson: move it to the byte-read that
  remains, `gen_font_table.rs:canvas::fontBlobUnchecked`, and don't drop it.
- The per-call full-file copy **remains** in `__canvas_fontBlob`. After this bug:
  - `measureText` pays **two** copies per call, up from one today: one inside the
    helper, and one for the `head`/`hhea` metrics (Fix Design step 3). That makes this
    bug a measurable per-call regression for `measureText` until the font-cache plan
    lands, so that plan should follow directly after this one.
  - A draw path also pays **two** copies per geometry build: one inside the helper, and one
    for the outline/raster bytes.

  The draw cost is paid only when a `Text` item's content hash changes, not every frame,
  because geometry is cached on the content hash (`canvas::present` DESC). It is removed
  entirely by the font-cache plan, where the helper stops reading bytes. It is recorded
  here so that a benchmark move on text-heavy scenes isn't mistaken for noise.

## Fix Design

1. Add `__canvas_shapeRun` to `helper_font.rs`, beside the readers it composes:

   ```
   FUNC __canvas_shapeRun(fonts AS List OF Integer, text AS String) AS List OF Integer
     MUT blobs AS List OF List OF Byte = []
     FOR EACH f IN fonts
       blobs = collections::append(blobs, __canvas_fontBlob(f))
     NEXT
     MUT run AS List OF Integer = []
     FOR EACH cp IN encoding::utf32Encode(text)
       ' First font in fallback order that maps cp; none does -> fonts[0]'s .notdef.
       MUT which AS Integer = 0
       MUT gid AS Integer = 0
       MUT i AS Integer = 0
       WHILE i < len(blobs) AND gid = 0
         gid = __canvas_glyphIndex(collections::getOr(blobs, i, []), cp)
         IF gid <> 0 THEN
           which = i
         END IF
         i = i + 1
       END WHILE
       run = collections::append(run, which)
       run = collections::append(run, gid)
       run = collections::append(run, __canvas_glyphAdvance(collections::getOr(blobs, which, []), gid))
     NEXT
     RETURN run
   END FUNC
   ```

   This is illustrative. Match the file's idiom when writing it. Each font's bytes are
   fetched **once per call**, not once per codepoint. The helper works in font units
   and never scales. Its doc comment states that it is the **only** place text becomes
   glyphs, that list order is fallback order, and that kerning, spacing and the
   parse-once font cache change this body and nothing else.

   Check that an unmapped codepoint stays byte-identical. HEAD calls
   `__canvas_glyphIndex(b, cp)`, which answers 0 for an unmapped codepoint, then uses
   glyph 0's advance from that same font. With one font, the helper does exactly the
   same: `which = 0`, `gid = 0`. A font with no usable `cmap` also answers 0, and the
   result is the same.
2. **A closed-font guard that returns the handle.** Replace the internal
   `canvas::fontBytes` with `canvas::fontOpenHandle(font AS RES canvas::Font) AS Integer`
   in `func_measure_text.rs`. It keeps `lower_font_bytes`' `emit_closed_guard` →
   `raise_error_bare("ErrResourceClosed")` branch and returns the handle instead of
   copying the file. Its registry `errors` become `["ErrResourceClosed"]`, because it no
   longer allocates. Keep the "It copies, and the first version did not" lesson by moving
   it to `canvas::fontBlobUnchecked` (see Blast Radius).
3. `__canvas_measureText`: `LET run = __canvas_shapeRun([canvas::fontOpenHandle(font)], text)`.
   `advance` becomes the Integer sum of each triple's third element, so the arithmetic is
   unchanged (an Integer sum, then `toFloat(advance) * scale`). `unitsPerEm`, ascent,
   descent and lineGap need bytes today: read them from
   `__canvas_fontBlob(handle)`. That is the second copy per call, until the font-cache
   plan turns these into handle reads too. If the handle has no published blob, return
   the all-zero `TextMetrics`, exactly as HEAD does for a font with no `head` table.
4. `__canvas_textGlyphRun`: shape once, with `[canvas::fontHandle(t.font)]`, before pass
   one. Pass one takes `gid` from the run. Pass two reads `gid` and `adv` from the run
   and keeps `pen = pen + toFloat(adv) * scale`. `chars` becomes `len(run) / 3`. The
   raster step (`__canvas_glyphEntry`) still takes the bytes and handle of the font
   `fontIndex` names. With one font, that is the same `b` and handle it uses today.
5. `__canvas_textEdges` and `__canvas_geoRunReference`: the same substitution.
   `__canvas_glyphEdges` keeps its bytes argument (it needs outlines). In
   `__canvas_geoRunReference`, the stored-entry offset stays
   `offset + __CANVAS_GEO_HEADER + c * 3` with `c` the glyph ordinal, and the glyph key
   still uses the handle and `gid` from the run.

Where the risk concentrates: `__canvas_textGlyphRun`'s two-pass pinning. The shaped run
is a plain local List of Integers, built **before** pass one, and it holds no cache
indices. So eviction during pass one cannot invalidate it, which is the hazard that
function's comments describe for entry indices. That reasoning should go in a comment
at the call. There is a second, smaller risk: `measureText`'s closed-font error and
its zero-metrics fallback now come from different places (the guard and the blob),
so both need a guard test.

Rejected alternatives:
- **Pass font bytes to the helper.** Rejected (decided 2026-09-27): bytes are passed only
  where outlines are read. A bytes signature would also have to change a second time
  when the font cache lands.
- **A separate fallbacks argument or field.** Rejected (decided 2026-09-27): the font
  list *is* the fallback chain, in order.
- **Return pixel positions from the helper.** Rejected: it breaks bit-identity (see
  Non-goals).
- **Return a `List OF` a record type (`ShapedGlyph[font, gid, advance]`).** Rejected for
  now. Every neighbouring helper uses flat numeric lists, and the geometry record is
  flat. A record type would be the only one in the text path.
- **Keep `canvas::fontBytes` for the metrics and add the guard beside it.** Rejected:
  that leaves `measureText` holding font bytes it doesn't need, which is exactly what
  the decision above rules out. The metric reads use `__canvas_fontBlob` only until the
  cache lands, and they are marked as such.

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
column of the triple list, or broke fractional pen accumulation would pass every
existing test. So the guards come first, and they must be GREEN at HEAD.

- [ ] Extend the fixture builder, or add a second fixture, so it provides **two
      mapped glyphs with different advances** that are not whole pixels at the test
      size, plus an **unmapped codepoint** (so the run includes glyph 0).
- [ ] Guard `measureText`: exact `width` for a mixed string such as `"AB?A"` with `?`
      unmapped, compared against the hand-computed Integer-sum-then-scale value.
- [ ] Guard the closed-font contract: `measureText` on a destroyed font raises
      `ErrResourceClosed`. Check that an existing test in `rt_canvas_font.rs` already
      asserts this before adding one.
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

- [ ] Add `__canvas_shapeRun(fonts, text)` to `helper_font.rs` with its "only place /
      list order is fallback order" doc comment.
- [ ] Replace internal `canvas::fontBytes` with `canvas::fontOpenHandle`
      (`func_measure_text.rs`). Move the copy-not-alias lesson to
      `canvas::fontBlobUnchecked`.
- [ ] Route `__canvas_measureText` through the helper, reading the handle via
      `canvas::fontOpenHandle` (`func_measure_text.rs`).
- [ ] Route `__canvas_textGlyphRun`, `__canvas_textEdges` and
      `__canvas_geoRunReference` through it (`helper_geometry.rs`). Add the pinning
      comment at the `__canvas_textGlyphRun` call.

Acceptance: the structural test passes, `grep -rn "fontBytes" src/` reports no
member or caller (only the relocated history comment, if it names it), all Phase 1 guards
still pass, and
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

- Where the structural test lives — in-crate `#[cfg(test)]` over the source constants
  (recommended, no file-path coupling) vs. an integration test that `include_str!`s
  the `.rs` files.
- Landing order against the font-cache plan. This bug adds a second font copy to
  `measureText` (see Blast Radius). Recommended: land the cache plan directly after
  this one. The alternative is to land the cache first and have this bug's helper read
  cached tables from day one, which makes this bug depend on that plan.

Settled 2026-09-27 (do not re-open here): the helper takes a list of font **handles**,
list order is fallback order, there is no separate fallbacks argument or field, and
font bytes are passed only where outlines are read. The output is flat
`(fontIndex, glyphId, advanceUnits)` triples.

## Summary

The helper is small. The engineering risk lies in three places:
- keeping the four paths' deliberately different pen arithmetic bit-identical;
- not disturbing `__canvas_textGlyphRun`'s eviction-safe two-pass pinning;
- keeping `measureText`'s `ErrResourceClosed` once its bytes stop coming from the
  guarded `fontBytes`.

Today's tests are too weak to catch a pen mistake (one glyph, whole-pixel advance), so
Phase 1's guards are the real work. Left for later plans: the parse-once font cache,
the public font-list API (`measureText` overload and the `Text` font list), `cmap`
subtable lookup cost, and the hash key. Every pixel is unchanged.
