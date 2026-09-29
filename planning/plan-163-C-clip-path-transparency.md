# plan-163-C: path clipping, `transparency`, and the picture tint

Last updated: 2026-09-28
Effort: large (3h–1d)
Depends on: plan-163-B (stroke styling; see Prerequisites)

The canvas `Paint` has three gaps compared with SVG content.

- **Clipping** is one axis-aligned rectangle in surface pixels (`Paint.clip AS
  canvas::Bounds`). It ignores the item's transform and a group's translation, so it
  cannot express an SVG `clip-path`: an arbitrary path with holes, a `clip-rule`, and
  its coordinates in the element's own space.
- **There is no whole-item opacity.** A fade has to be written into the fill colour,
  the stroke colour and every gradient stop.
- **A `Picture` overloads its fill colour.** The fill is a multiply tint, and its alpha
  is the picture's opacity. So the zero `Paint` draws a picture as nothing, the one
  place where an unset paint is not a no-op.

This letter makes four changes:

- `Paint.clip` becomes rings (`List OF List OF Point`, the same shape as plan-163-A's
  `Polygon.contours`) with a `clipRule`, and `canvas::Bounds` is removed;
- the clip is given in the item's own coordinates, so `paint.transform` and a group's
  translation apply to it exactly as they apply to the item;
- a new `transparency` field (0.0 = opaque, the zero value) fades the whole item;
- a picture's fill colour becomes **only** a tint whose alpha is the tint strength,
  with its opacity coming from `transparency`.

**No backwards compatibility** (user decision, 2026-09-28): the clip's coordinate
space, the removed `Bounds` type and the picture tint rule change outright. Every
in-tree site is migrated. There is no shim, and no old behavior is kept behind a
flag.

**The single behavioral outcome:**

- An item with `paint.clip := [ring, …]` draws only where the rings enclose the pixel,
  under `clipRule`, with the rings mapped by `paint.transform` and moved by the group
  offset. The clip edge is antialiased like a shape edge.
- An item with `transparency := t` draws at `(1 − t)` of its alpha.
- A picture draws `texel.rgb` blended toward `texel.rgb × fill.rgb / 255` by
  `fill.a / 255`, at alpha `texel.a × (1 − transparency)`. The zero `Paint` therefore
  draws the image unchanged and opaque.
- All three renderers agree: the software renderer exactly, the GPUs within
  `Tolerance::GPU_DEFAULT`.

References:

- plan-163-A (the rings and `FillRule`, and the software/GPU fill test and its
  NonZero count, which the clip reuses) and plan-163-B (the Paint fields and the item
  block and header growth this letter extends).
- `src/docs/spec/app/06_canvas.md`:
  - the Paint zero-value rule (:67–87);
  - the double-float clip distance (:161–215, "the clip's rectangle distance too");
  - the transform rule (:378–399);
  - the Picture rule (:420–436, the tint at :431–434).
- `.ai/canvas-threading.md`:
  - :517–574, a per-item payload is a frame region plus an index, and caps are frame
    sums;
  - :1003–1040, "Growing `ITEM_BLOCK_SIZE` moves six things, and three are silent"
    (**this letter grows it**);
  - :1042+, shared layout constants need pin tests.
- `.ai/testing-gates.md` :235 and :309 (exact software goldens; the GPUs are compared
  against the oracle, so the oracle needs hand-computed checks).
- `planning/completed/plan-116-B-*` (the rectangle clip) and `plan-116-G` G5 (the
  decision that a clip is **not** moved by a group offset). This letter reverses G5 on
  purpose (§3).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-163-B (stroke styling) complete and archived | `ls planning/completed/plan-163-B-*` → match | NOT MET (2026-09-28: B not started) |

plan-163-A is complete by transitivity, because B requires it. Everything below is
written against the world where A and B have landed:

- `Polygon.contours` and `Polygon.rule` exist, and the software and GPU crossing tests
  support NonZero;
- `Paint` has B's `cap`/`join`/`miterLimit`/`dashes`/`dashOffset` fields and B's header and item-block
  growth.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> the command before you continue and before you stop, and report it if you stop.

## 1. Goal

- `canvas::Paint.clip` is `List OF List OF canvas::Point`, and the empty list means
  unclipped. The new `canvas::Paint.clipRule` is a `canvas::FillRule`.
- The record `canvas::Bounds` no longer exists.
- The clip rings are in the item's own coordinates. The spec and the `clip` field
  description say: **`paint.transform` is applied to the clip path as well as to the
  item, and so is a group's translation.** The clip stays a `Paint` field. It is not
  part of the `Transform` record.
- The new `canvas::Paint.transform`-independent field `transparency AS Float` has 0.0
  as opaque (the zero value) and 1.0 as invisible. Values outside 0..1 are clamped. It
  multiplies the item's final alpha by `1 − transparency`: fill, stroke, gradient,
  glyphs and pictures alike.
- **Picture tint:**
  - `tinted = texel.rgb × fill.rgb / 255`;
  - `out.rgb = texel.rgb + (tinted − texel.rgb) × fill.a / 255`;
  - `out.a = texel.a × (1 − transparency)`.
  - `fill.a` no longer affects the picture's alpha. The integer rounding is fixed in
    §6 and is the same in all three renderers.
- All three renderers implement all of this: the software renderer exact against
  hand-computed pixels, Metal and Vulkan within `GPU_DEFAULT` of it.

### Non-goals (explicit constraints)

- **Not SVG element or group `opacity`.** `transparency` fades an item's fill and
  stroke separately, so where they overlap the fill shows through a faded stroke. That
  matches SVG's `fill-opacity` and `stroke-opacity`, not element `opacity`. True group
  opacity needs an offscreen layer composite, which does not exist (layers are a cache
  boundary, spec :36–44) and is a later letter.
- **No clip masks** (SVG `<mask>`, a luminance or alpha image) and no clip by another
  item's shape. A clip is rings of points only.
- **No change to fill, stroke, join or cap geometry**, or to `DrawItem`'s variant set.
- **No backwards compatibility** (above).

## 2. Current State

Paths are under `src/codegen/builtins/canvas/` unless written in full. Line numbers are
at HEAD `48dbc21a3` (pre-A/B; A and B do not touch the clip, tint or alpha code, so
re-verify only the lines B's layout growth shifts).

**Types.**

- `Bounds` is declared at `mod.rs:433`.
- Its only use as a field type is `Paint.clip` (`mod.rs:852`, `ty:
  ParameterType::named("Bounds")`). It also appears in a type-name list at
  `mod.rs:1791`.
- `grep -nE 'named\("Bounds"\)' src/codegen/builtins/canvas/*.rs` → 1 hit, `mod.rs:852`.
- The default is `helper_paint_defaults.rs:__canvas_noClip()` (:166).

**Clip in the header.** `__canvas_paintHeader` resolves the clip to `x0, y0, x1, y1`
in header slots 22–25. A zero-area or negative-extent clip means unclipped, which is
what an unset clip is. The native twin is `func_geo_build.rs:emit_paint_header`
(:500–584).

**Software clip.**

- `helper_items.rs:__canvas_hasClip` (:81) gates the per-pixel test.
- `__canvas_clipCoverage` (:143) is the antialiased coverage of the surface rectangle
  at a pixel centre.
- It is applied as `(coverage * clipCov) / 255` at :299 (glyph runs), :333 (glyph
  blit) and :555 (shapes).
- The pixel range is narrowed to the clip rectangle at :448–483.
- **The clip is deliberately not offset by a group** (:475–478, "plan-116-G / G5 … a
  group's translation moves the shape through the clip rather than carrying the clip
  with it").

**GPU clip.**

- The `ItemBlock` holds `clip` (x0,y0,x1,y1 in 16.16) and `clipLo` (their residual
  words) (`mfb_canvas.frag:26`, `:37`).
- `clipCoverage(vec2 p)` (`mfb_canvas.frag:825`) evaluates the rectangle in double
  float. It is called with `gl_FragCoord.xy`, the surface point (:914–918, "`Paint.clip`
  is a surface rectangle"), and multiplied into the glyph (:928), fill (:953) and
  stroke (:963) coverage.
- Metal's `clipCoverage` is its line-for-line twin in `metal.rs:METAL_SHADER_BODY`.

**The polygon machinery the path clip reuses** (after A):

- The edges go into the frame edge region (`emit_edge_buffer` in Metal,
  `emit_edge_upload` in Vulkan).
- A per-polygon **band index** limits each pixel row to the edges that can reach it.
  `polygonBand` (`mfb_canvas.frag:385–401`) reads the band header from `arcCaps` =
  (top, height, band count, table offset), and the table lives in the edge region at
  `BAND_BASE`. It is written by `emit_band_index` (`vulkan.rs:4233`, `metal.rs:5052`).
- `edgeDistance(base, count, list, x, y, reach)` returns the signed distance under
  A's rule argument.
- The frame-sum edge cap is 262,144 (spec :124–146).

**Picture tint.**

- Software: `helper_items.rs` near :593 (`The texel is then TINTED by …`).
- Vulkan: `mfb_canvas.frag` near :672 ("tinted by `fill` integer-wise, alpha
  included — the fill's alpha is the …").
- Metal: `metal.rs` near :923 (the twin of the `isPicture` arm).
- The rule is `(texel × fill) / 255` per channel, alpha included (spec :431–434).

**Item alpha.** Fill and stroke colours are in header slots 8–15 and in the item
block's `fill`/`stroke` words. Gradient stops carry their own colours (the stop tail).
Glyph runs are drawn with `item.fill` (`mfb_canvas.frag:928`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `canvas::Bounds` references (code, tests, scripts, examples) | 22 | `grep -rn "canvas::Bounds" --exclude-dir=target --exclude-dir=.git --exclude-dir=planning --exclude-dir=bugs . \| wc -l` |
| `clip :=` sites | 23 | same with `"clip :="` |
| files holding either | 11 (`scenes/transformed_shapes_on_black.mfb` 8, `rt_canvas_rasteriser.rs` 7, `rt_canvas_metal.rs` 3, `scenes/canvas_geo_native_matrix.mfb` 2, `rt_canvas_golden.rs` 2, `mod.rs` 2, `scripts/test-canvas-vulkan.sh` 2, and 1 each in `field_kinds.tsv`, `scenes/clips_at_a_step_on_black.mfb`, `rt_canvas_picture.rs`, `func_fill_stroke.rs`) | same, `\| cut -d: -f1 \| sort \| uniq -c` |
| `canvas::Picture[` sites | 54 (top: `rt_canvas_picture.rs` 14, `rt_canvas_metal.rs` 8, `rt_canvas_group_ownership.rs` 5, `test-canvas-vulkan.sh` 4) | `grep -rn "canvas::Picture\[" --exclude-dir=target --exclude-dir=.git --exclude-dir=planning --exclude-dir=bugs . \| wc -l` |
| Picture sites whose fill alpha is below 255 (the ones whose pixels change) | UNMEASURED | Phase 1's first task |
| Clip sites that also set a transform or sit in a translated group (the ones whose pixels change) | UNMEASURED | Phase 1's first task |

### Verified properties

- **`Bounds` has exactly one field use.** Verified by the `named("Bounds")` grep.
- **The clip is surface-space today, on every backend, and not moved by a group.**
  Verified by reading `helper_items.rs:475–478` and `mfb_canvas.frag:914–918`.
- **The path clip can reuse `edgeDistance` and the band index as-is.** The function
  takes its edge base, count and band list as arguments (`mfb_canvas.frag:330`) rather
  than reading them from the item, so it can be called on a second edge set. Verified
  by reading its signature and the `kind == 4` dispatch (:632).
- UNVERIFIED: whether the clip's double-float residuals (`clipLo`) are still needed
  for the rectangle fast path once it is evaluated in shape space. Phase 3's GPU rows
  measure it against `GPU_DEFAULT`.

## 3. Design Overview

The work comes in four pieces:

1. **Types and migration.** Add the new `Paint` fields, remove `Bounds`, and change the
   tint rule. Migrate every site.
2. **Transparency.** Fold it into the fill, stroke and gradient-stop alpha at geometry
   build, so the renderers see only colours they already handle. Only the picture path
   reads it at draw time.
3. **Path clip, software.** The clip rings become a **clip edge tail** evaluated at the
   shape-space point with `edgeDistance` under `clipRule`. An axis-aligned one-ring
   rectangle keeps a rectangle fast path.
4. **Path clip, GPU.** The clip edges go into the frame edge region with their own band
   index. The item block grows by two `ivec4`s for the clip edge set and its band
   header.

**Decisions this letter takes:**

- **The clip lives in the item's own space and follows both `paint.transform` and the
  group offset.** This reverses plan-116-G5, where the clip was surface-space and a
  group moved the shape through it. That was right for a surface-rectangle clip; it is
  wrong for an SVG `clip-path`, which moves with its element. The user asked for the
  transform to apply to the clip. The group offset is a translation of the same
  coordinate space, so applying one and not the other would make a clipped item inside
  a translated group clip somewhere else.
- **Transparency is folded at build time, not per pixel.** The effective alpha of each
  colour is `(a × (255 − T) + 127) / 255` in integers, where `T = round(clamp(t,0,1) ×
  255)`. It is computed once in both geometry builders and written into header slots
  8–15 and into the stop tail. So the fill, stroke, gradient and glyph paths need **no
  shader change** and cannot disagree between renderers.
  - The picture is the exception. Its alpha comes from the texel, so it gets `T` in one
    item-block word (§6).
- **The rectangle fast path.** When `clip` is one 4-point ring that is an axis-aligned
  rectangle **and** the item has no transform, header slots 22–25 hold it
  (group-offset-adjusted at draw time, as the shape is). The existing rectangle
  `clipCoverage` is used, and no clip edges are emitted. Every other clip takes the
  path. A fast-path rectangle and the same rectangle through the path give the same
  coverage within rounding, and a Phase 2 test pins that they agree exactly in
  software.

**Correctness risk concentrates in the item-block growth** (six coupled sites, three
of them silent). It lands in Phase 3 behind the four named guard tests.

**Design uncertainty concentrates in** the clip coverage when the clip is transformed
with the item. The clip's distance is evaluated at the same inverse-mapped point and
scaled by the same direction-aware divisor as the shape (spec :390–399). So a sheared
clip is antialiased as a sheared shape is. Phase 2 tests a sheared and a rotated clip
first.

**Correctness gate: behavioral.**

- **Byte-identical sub-gate:** every software golden PNG and hand-computed test for an
  item with **no clip, `transparency` 0.0, and not a Picture** stays exact. Those
  items' headers are unchanged, because slots 22–25 are still zero and the alpha fold
  with `T = 0` is the identity. A failure there is a builder bug: dump ONE header with
  `MFB_CANVAS_DUMP` and fix it.
- **Expected to change** (the plan working):
  - pictures whose fill alpha was below 255;
  - clipped items that also have a transform or sit in a translated group;
  - `tests/syntax/app/app-mouse-surface/golden/*` (the type surface);
  - the item-hash values.
  Each changed test or golden is re-derived by hand per AGENTS "Never edit a test to
  pass". The proof it is wrong is the user's 2026-09-28 decision recorded above, cited
  in the commit.

**Rejected alternatives** (do not re-litigate):

- *A single ring (`List OF Point`).* It cannot express a clip with a hole or several
  subpaths, and SVG `clip-path` needs both.
- *The clip inside `Transform`, or its own transform.* The user's decision: it stays a
  `Paint` field, and the item's transform applies to it.
- *`opacity` with 1.0 = opaque.* The zero value must be opaque (user decision), so the
  field is named for what its zero does.
- *Transparency as a per-pixel shader multiply.* Folding it at build time is exact,
  identical across backends, and needs no shader change except for pictures.
- *A colourise tint (keep the luminance, replace the hue).* The user chose multiply. It
  also keeps full-alpha pictures pixel-identical.
- *A CPU-computed clip mask texture.* That is per-frame CPU work proportional to the
  clip area, which is what the user asked to avoid. Edges plus a band index keep the
  work on the GPU.

## 4. The new `Paint` fields

```
clip          AS List OF List OF canvas::Point   ' was: AS canvas::Bounds
clipRule      AS canvas::FillRule
transparency  AS Float
```

- These are declared in this order, with `clip` in its current position,
  `clipRule` right after it, and `transparency` at the end after B's fields.
- `canvas::fill`, `canvas::stroke` and `canvas::fillStroke` set `clip := []`,
  `clipRule := EvenOdd` and `transparency := 0.0` (`func_fill_stroke.rs` literal,
  `helper_paint_defaults.rs:__canvas_noClip` → `[]`).
- A clip ring with fewer than 2 points contributes nothing, as in plan-163-A. A
  non-empty clip whose rings all contribute nothing clips **everything** away: it
  encloses no area. It is not treated as "unclipped". Only the empty list means
  unclipped, which keeps the zero value inert without making a degenerate clip
  silently draw.

## 5. Clip geometry and evaluation

- **Header**: three new slots, appended after the last slot B adds. Their numbers are
  `__CANVAS_GEO_HEADER`-at-B + 0, 1, 2, and Phase 1 records the actual numbers here:
  - `CLIP_MODE`: 0 none, 1 rectangle, 2 path;
  - `CLIP_EDGE_COUNT`;
  - `CLIP_RULE`.
  - Slots 22–25 hold the rectangle (mode 1), or the **surface bounds of the
    transformed clip path** (mode 2), which is used to narrow the pixel range as today.
  - Both builders change together. Both pin tests pin the new slots.
- **Tail order**: edges | joins | dashes | **clip edges** | stops. Stops stay last. A
  new `__canvas_clipEdgeBase` computed from the header counts is the only way readers
  find the clip edges (the `__canvas_gradientStopBase` rule).
- **The clip edges** are in the item's own coordinates, before the transform: the same
  space as the shape's edges, built by A's `__canvas_polygonEdges` from the rings.
- **Software** (`helper_items.rs`): for mode 2, `clipCov = __canvas_coverage(dClip /
  dScale)`, where `dClip` is `__canvas_edgeDistance` over the clip edges at the
  shape-space point under `CLIP_RULE`, and `dScale` is the shape's direction-aware
  divisor at that point. Mode 1 keeps `__canvas_clipCoverage` with the rectangle offset
  by the group translation. Mode 0 is unclipped. The pixel-range narrowing at :448–483
  now uses the (group-offset) slots 22–25 in modes 1 and 2.
- **GPU**:
  - `clipCoverage` takes the shape-space point (not `gl_FragCoord`) plus the divisor.
  - Mode 1 uses the rectangle test, offset by the group.
  - Mode 2 calls `edgeDistance(clipEdgeBase, clipEdgeCount, clipBandList, …)` with the
    clip's own band list and `CLIP_RULE`.
  - The clip edges share the frame edge region and its 262,144 frame-sum cap (the
    Metal and Vulkan predicates in `helper_render.rs` add `CLIP_EDGE_COUNT` to the
    polygon edge sum). The band table is built by the same `emit_band_index`,
    parameterised by which edge set it indexes.

## 6. Transparency and the picture tint

- **Fold** (both builders, in `__canvas_paintHeader` and `emit_paint_header`):
  - `T = round(clamp(transparency, 0, 1) × 255)`.
  - Each of the fill, stroke and gradient-stop alphas becomes `(a × (255 − T) + 127)
    / 255` in integers.
  - With `T = 0` this is the identity: `(a×255 + 127)/255 = a` for a in 0..255.
  - The picture's fill colour is **not** folded, because it is a tint.
  - A new header slot is not needed for fill/stroke/stops. The picture needs `T`
    (below).
- **Picture** (software near `helper_items.rs:593`, frag near :672, Metal near :923),
  in integers:
  - `tint_c = (texel_c × fill_c + 127) / 255`;
  - `out_c = texel_c + ((tint_c − texel_c) × fill_a + (sign-aware) 127) / 255`, using
    a round-half-away form the three renderers write identically;
  - `out_a = (texel_a × (255 − T) + 127) / 255`.
  - `T` travels in a fourth new header slot, `PICTURE_TRANSPARENCY`, appended after
    the three clip slots (so C grows the header by 4 in total), and in an item-block
    word (the new `clipPath`
    `ivec4`'s spare `w`, §7).
  - The exact rounding expression is fixed by the Phase 1 software implementation and
    copied line for line into both shaders. The spec quotes it.

## 7. Item block growth

- The block grows from 288 (after B) to 320 bytes, with two new `ivec4`s:
  - `clipBand` = (top, height, band count, table offset), the same shape as
    `arcCaps`'s polygon band header;
  - `clipPath` = (clipEdgeBase, clipEdgeCount, clipMode | clipRule << 2, picture `T`).
- The existing `clip`/`clipLo` keep the rectangle (mode 1) or the path's bounds
  (mode 2).
- Follow `.ai/canvas-threading.md:1003–1040` item by item. That covers both shader
  struct copies and `mfb_canvas.vert`, `runtime/canvas/mod.rs`'s `ITEM_*` constants and
  `ITEM_BLOCK_SIZE`, Metal's stack frame, the region bases, and both `.spv`
  regenerated (`scripts/regen-spirv.sh`, box 2228).
- The guard tests are `the_draw_frame_slots_do_not_overlap`,
  `the_metal_shader_region_bases_match_the_buffer_layout`,
  `the_pipeline_specializes_every_region_base` and
  `the_shared_buffer_holds_every_region`.

## Compatibility / Format Impact

- **Source-breaking, no shim** (user decision):
  - `canvas::Bounds` is removed. `Paint.clip` changes type.
  - `Paint` gains `clipRule` and `transparency`. The one `Paint[...]` literal is the
    constructor helper; `WITH` refinements that do not name `clip` compile unchanged.
- **Pixel-changing, no shim:**
  - A clip now follows `paint.transform` and the group offset.
  - A picture's fill alpha is a tint strength, not its opacity. The zero `Paint` now
    draws a picture unchanged instead of drawing nothing.
- **Unchanged:** every item with no clip, `transparency = 0.0` and not a Picture draws
  byte-identically in software, and within tolerance on the GPUs.
- **Internal layout (no external contract):** the header grows by 4 slots and the item
  block to 320 bytes.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work. Use `- [~]` for partial work, with what remains. Mark a moot task
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` the moment a phase lands. **An
> unticked box means NOT DONE.**

### Phase 1 — Types, transparency fold, picture tint, migration (software)

This phase delivers the new surface and the two alpha rules in software. The clip is
still rectangle-only here (the fast path), so nothing about the path clip is
half-built.

- [ ] Measure the two UNMEASURED rows of §2:
      - Picture sites whose fill alpha is below 255 (read each of the 54 sites'
        `paint`);
      - clip sites that also set a transform or sit in a translated group.
      Record both counts and file lists here. They are the tests whose expectations
      change.
- [ ] `mod.rs`: remove the `Bounds` record and its `:1791` list entry. `Paint.clip` →
      `list_of(list_of(named("Point")))`. Add `clipRule` and `transparency` per §4, with
      descriptions. The `clip` description states that **`paint.transform` and a group's
      translation apply to the clip path as well as to the item**. The Picture
      description states the tint and `transparency` rule.
- [ ] `helper_paint_defaults.rs:__canvas_noClip` → `[]`. Update the constructor
      literal in `func_fill_stroke.rs` and the "all N fields" sentences
      (`func_fill.rs:24`, `func_stroke.rs:24`, `func_fill_stroke.rs:24`).
- [ ] Both builders:
      - clip mode 0/1 per §5, where mode 1 is only an axis-aligned 4-point ring with no
        transform. Anything else is refused by `emit_paint_supported` until Phase 2
        (it falls back to MFBASIC, which in Phase 1 also only accepts modes 0/1: any
        other clip draws as unclipped until Phase 2, and no in-tree site uses one yet);
      - the alpha fold per §6;
      - `PICTURE_TRANSPARENCY`;
      - the header slots added per §5/§6, with both pin tests extended.
- [ ] `__canvas_hashPaint`: fold `clip` (rings), `clipRule` and `transparency`.
- [ ] Software picture tint per §6 (`helper_items.rs` near :593). Software mode-1 clip
      offset by the group (replacing the :475–478 G5 behavior).
- [ ] Migrate the 23 `clip :=` / 22 `canvas::Bounds` sites to rings (a rectangle
      `Bounds[x, y, w, h]` becomes a 4-point ring). Migrate the Picture sites whose
      fill alpha was below 255 to `transparency`.
      For each test whose expected pixels change, re-derive the expectation by hand and
      cite this plan's decision in the commit. This includes plan-116-G5's
      group-and-clip test, whose premise the user's decision replaces.
      Check: `grep -rn "canvas::Bounds" --exclude-dir=target --exclude-dir=.git
      --exclude-dir=planning --exclude-dir=bugs .` → 0.
- [ ] Tests (`rt_canvas_rasteriser.rs`, `rt_canvas_picture.rs`, hand-computed):
      - `transparency := 0.5` on a red fill gives alpha 128 (`(255×128+127)/255 = 128`);
      - on a gradient, each stop's alpha is folded;
      - on stroked text, the glyph alpha is folded;
      - `transparency` values −1 and 2 clamp;
      - Picture with the zero `Paint`: the image unchanged and opaque;
      - Picture with white fill at alpha 255: unchanged;
      - Picture with red fill at alpha 255: the channel multiply (today's pixels);
      - Picture with red fill at alpha 128: the half blend;
      - Picture with `transparency := 0.5`: the texel alpha halved;
      - a mode-1 clip inside a translated group is clipped at the group's offset.
- [ ] Regenerate `tests/syntax/app/app-mouse-surface/golden/*`
      (`scripts/regen-native-goldens.sh`, expected). Inspect the `.ir` diff: it should
      be the type surface only.

Acceptance: the new rules hold in software, and unaffected items are byte-identical.
  Check: `cargo test --test rt_canvas_rasteriser --test rt_canvas_golden --test
  rt_canvas_picture --test rt_canvas_geo_native` → pass. `git diff --stat
  tests/golden/canvas` must list only PNGs whose scenes are in the Phase 1 measured
  lists (est. 12 min, guess; these four binaries cover clip, alpha, picture and both
  builders).
Commit: —

### Phase 2 — Path clip in software (the design's uncertain part, tested first)

- [ ] Premise tests first (hand-computed, `rt_canvas_rasteriser.rs`):
      - a triangle clip on a rectangle;
      - a ring-with-hole clip under EvenOdd and under NonZero with a same-direction
        inner ring;
      - a clip under a 30° rotation;
      - a clip under an X shear;
      - a path clip inside a translated group.
      If the sheared clip's edge coverage disagrees with a sheared shape edge of the
      same geometry, fix the divisor use and record it in Corrections.
- [ ] Both builders: mode 2, the clip edge tail per §5, and
      `__canvas_clipEdgeBase`. `emit_paint_supported` accepts mode 2 once native emits
      it. `MFB_CANVAS_GEO_VERIFY=1` under `--debug` reports no mismatch on the premise
      scenes.
- [ ] Software mode-2 coverage per §5.
- [ ] Test: an axis-aligned rectangle clip with no transform gives exactly the same
      pixels through mode 1 and forced through mode 2. This pins the fast path to the
      general path.
- [ ] Test: a non-empty clip whose rings are all shorter than 2 points draws nothing.
      `clip := []` draws unclipped.

Acceptance: path clips draw per §5 in software, and modes 0 and 1 are unchanged from
Phase 1.
  Check: `cargo test --test rt_canvas_rasteriser --test rt_canvas_golden --test
  rt_canvas_geo_native` → pass, goldens unmodified since Phase 1 (est. 10 min, guess).
Commit: —

### Phase 3 — Item block growth, and the path clip and tint on the GPU (largest blast radius)

- [ ] Item block 288 → 320 per §7, with every guard test passing.
- [ ] Emitters (`metal.rs`, `vulkan.rs`):
      - clip edges into the edge region, and a clip band index via a parameterised
        `emit_band_index`;
      - `clipBand`/`clipPath` words and picture `T`;
      - the frame-sum predicate in `helper_render.rs` counts clip edges.
- [ ] Shaders (`METAL_SHADER_BODY`, `mfb_canvas.frag`, line for line): `clipCoverage`
      on the shape-space point per §5, and the picture tint per §6. Regenerate both
      `.spv` on box 2228.
- [ ] Tests: the Phase 1 picture and group-clip scenes and the Phase 2 premise scenes as
      `rt_canvas_metal.rs` rows.

Acceptance: both GPUs match the software renderer within tolerance, and the layout
guards pass.
  Check: `cargo test --bin mfb canvas` → pass (est. 3 min, guess);
  `cargo test --test rt_canvas_metal` → pass with `gpuFrames≠0` on the new rows (est.
  5 min, guess); `scripts/test-canvas-vulkan.sh <path-clip scene>` and `<picture-tint
  scene>` on box 2228 → within `GPU_DEFAULT` (est. 15 min, guess; 2228 is the Linux
  Vulkan lane, `.ai/remote_systems.md:112–137`);
  `scripts/test-canvas-gpu-rows.sh <path-clip scene> --target windows-x86_64` on box 206
  (the only real GPU) → within tolerance (est. 10 min, guess).
Commit: —

### Phase 4 — Docs and spec

- [ ] `06_canvas.md`:
      - the clip: rings, `clipRule`, and the clip in the item's space moved by the
        transform and the group offset (rewrite the plan-116-B/G5 text);
      - the fast path and its equality with the general path;
      - `transparency` and the fold formula;
      - the Picture rule (:431–434 rewritten with the tint formula);
      - the header and item-block growth where the layout is cited.
- [ ] `.ai/canvas-threading.md`: the `clipBand`/`clipPath` block words, the clip edges
      sharing the edge region, and the tail order edges | joins | dashes | clip edges | stops.
- [ ] Man prose on `Paint` (`clip`, `clipRule`, `transparency`), `Picture` and
      `FillRule` (`.ai/man-content.md` bans apply).

Acceptance: man and spec render and cite.
  Check: `mfb man canvas types` shows no `Bounds` and the new fields;
  `bash scripts/man-census.sh --memory-scope` → 0 unclassified;
  `bash scripts/man-run-examples.sh canvas --run` → pass;
  `bash scripts/spec-census.sh --citations` and `--links` → 0 dangling;
  `cargo test --bin mfb spec` → pass (est. 8 min, guess).
Commit: —

## Validation Plan

- Tests: per phase, with hand-computed software pixels. Negative cases: `transparency`
  out of range, a degenerate non-empty clip, `clip := []`, and a zero-`Paint` picture.
- Coverage check: the software paths run in
  `rt_canvas_rasteriser`/`rt_canvas_picture`/`rt_canvas_golden`. The shader paths run
  only on GPU lanes: `rt_canvas_metal` (confirm `gpuFrames≠0`) and the Vulkan scripts
  on 2228 and 206. The default `cargo test` covers none of the Vulkan shader.
- Runtime proof: an `--app` program on macOS with `MFB_CANVAS_GPU=1`, screenshotted.
  It shows:
  - a rotating item clipped by a star path that rotates with it;
  - an "O"-shaped clip;
  - a picture fading out via `transparency`;
  - a picture tinted red at 50% strength.
- Doc sync: Phase 4.
- Final gate (run ONCE, after Phase 4): `cargo test --no-fail-fast` (est. 40 min,
  from plan-159-D's recorded run) and `bash scripts/artifact-gate.sh target/release/mfb
  all` → 0 diffs outside `tests/syntax/app/app-mouse-surface` (est. 20 min).

## Open Decisions

- **Degenerate non-empty clip**: clips everything (recommended, §4) vs. counts as
  unclipped. Only `[]` is the inert zero value; a clip the program wrote but that
  encloses nothing should be visible as such.
- **Clip bounds for mode 2**: the transformed path's exact surface AABB, computed from
  the transformed ring points (recommended) vs. the transformed AABB of the
  untransformed bounds, which is looser and cheaper. The exact one narrows more pixels
  and is computed once per content change.

## Corrections

(none yet)

## Summary

The engineering risk is in three places:

- evaluating a transformed clip path with the same distance, divisor and antialiasing
  as the shape it clips, which Phase 2 tests first in software;
- the item-block growth to 320 bytes, which Phase 3 runs behind the four guards;
- the deliberate, uncompensated pixel changes to transformed or group-clipped items and
  to pictures with a sub-opaque fill, which are measured first in Phase 1 and
  re-derived by hand.

Untouched: fill, stroke, cap, join and dash geometry, the `DrawItem` variant set, and
every item with no clip, no transparency and not a Picture.
