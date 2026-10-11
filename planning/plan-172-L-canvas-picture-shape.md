# plan-172-L: `Picture` clipped to a shape

Last updated: 2026-10-10
Effort: large (3h–1d)
Depends on: plan-172-K

`canvas::Picture` becomes `shape`, `clip`, `fit`, `image`. The image is drawn inside any
closed shape — a rounded rectangle, a circle, a polygon — fitted by `PictureFit` and
either upright or carried by the shape's transform, as `PictureClip` says.

Behavioral outcome: a program draws a photograph inside a circle with one item; the
shape's stroke outlines it; and a `Picture` whose shape is a plain rectangle draws
exactly what a `Picture` draws today.

References:

- `planning/new_canvas.md` — the `PictureClip` and `PictureFit` enums, the `Picture`
  listing and the six bullets under it.
- `planning/plan-172-A-canvas-shapes-premises.md` — Open Decision 3.
- `src/codegen/builtins/canvas/helper_geometry.rs` — `__canvas_pictureHeader`,
  `__canvas_clipHeader`, `__canvas_withClipEdges`.
- `src/codegen/builtins/canvas/helper_shapes.rs:__canvas_rrectRing` (172-E).
- `.ai/canvas-threading.md` §6 (image upload), §7 (closing an image), §13 (a group owns
  the images its items name).
- `src/docs/spec/diagnostics/02_error-codes.md` — the Constant Registry, a `build.rs`
  input.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-K complete and archived | `ls planning/plan-172-K-*` → no matches | NOT MET |
| `canvas::AutoShape` exists | `grep -c 'name: "AutoShape"' src/codegen/builtins/canvas/mod.rs` → 1 | NOT MET |
| Open Decision 3 of 172-A is answered | `grep -c '\*\*ANSWER:' planning/completed/plan-172-A-*.md` → 5 | NOT MET |
| The picture suites are green | `cargo test --test rt_canvas_picture --test rt_canvas_group_ownership` → pass | UNVERIFIED — run first |

If plan-172-K is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `canvas::Picture` is `id AS String`, `shape AS canvas::AutoShape`, `clip AS
  canvas::PictureClip`, `fit AS canvas::PictureFit`, `image AS RES canvas::Image`.
- `PictureClip` is `ClipOnly` (zero value), `ClipAndTransform`. `PictureFit` is
  `Stretch` (zero value), `Contain`, `Cover`.
- The image is visible only inside the shape. The shape's `paint` means what a
  picture's paint means today: `fill` tints, `transparency` fades, `blend` composites,
  `stroke` outlines the shape on top. The shape's interior is not filled separately.
- `Contain` and `Cover` centre the image in the bounds.
- `ClipAndTransform` fits to the shape's local bounding box and the shape's transform
  carries the image. `ClipOnly` fits to the axis-aligned bounding box of the transformed
  shape and draws the image upright.
- A `Picture` whose shape is a `Line`, `Arc`, `PolyLine` or `Text` draws nothing.
- When the shape is an `AutoPaint`, the state paint replaces the tint and outline.
- A `Picture` is hit as its shape is. It keeps its own `id`, which goes in front of its
  shape's in `overID` (172-I's id path): a picture `"avatar"` on a shape `"frame"`,
  inside a group `"card"`, reports `card`, `avatar`, `frame`, joined by the separator,
  with any empty part left out.

### Non-goals (explicit constraints)

- **No new geometry kind, no header slot, no item-block change, no shader change.** See
  §3.
- **Sampling stays nearest-pixel.**
- **The image is still held as `RES canvas::Image` directly on the `Picture`**, so
  `the_resource_naming_variants_hold_the_resource_itself` and the group ownership rules
  are unchanged.
- **`canvas::setBytes`, `destroyImage` and the closed-image rules are untouched.**
- **No second clip slot.** The shape outline and `paint.clip` are intersected into one
  edge set at expansion (Open Decision 3 of 172-A); the header keeps one clip.

## 2. Current State

After 172-E a `Picture` is `id`, `position`, `size`, `image`, `paint`, `transform`. Its
geometry is a rectangle record with the kind rewritten to picture (9); the sampling
rectangle is read back from the rectangle's own slots, and coverage is the rectangle's.
All three renderers treat kind 9 as a rectangle for distance.

`Paint.clip` already clips a picture on every renderer: an axis-aligned single ring on
an untransformed item becomes a clip rectangle in header slots 22–25; anything else
becomes edges appended to the geometry tail, with the count and rule in slots 52–53, and
coverage is multiplied by the clip's own antialiased coverage. The clip is polygonal.

`__canvas_appendDraw` already folds a picture's pixel-block address into its draw hash.

### Measured populations

`$D` is `src tests examples tools scripts benchmark`.

| What | Count | Command |
|---|---|---|
| `canvas::Picture[` sites | 71 in 24 files | `grep -rhoE 'canvas::Picture\[' $D \| wc -l` → 71 |
| Reads of a picture's placement fields in the companion source | 8 before 172-E | `grep -rhoE '\bpic\.(x\|y\|w\|h)\b' src/codegen/builtins/canvas \| wc -l` → 8; re-measure against `position` / `size` on entry |
| Picture sites that name a clip inline | at least 3 | `grep -rhoE 'canvas::Picture\[[^]]*' $D \| grep -c clip` → 3. The pattern stops at the first `]`, so this is a lower bound; these are the sites Open Decision 3 affects |
| Highest `7705xxxx` error code | 77050027 | `grep -rhoE '7705[0-9]{4}' src build.rs \| sort -u \| tail -1` → 77050027; re-measure on entry |

### Verified properties

- **A polygonal clip on a picture works today on all three renderers.** VERIFIED by
  reading `__canvas_clipHeader` and the picture branch of `__canvas_drawGeometry`, and
  by the description of `Picture` in the registry ("Stroke, clip, blend and transform
  work as they do on a `canvas::Rectangle`").
- **A picture's one clip slot cannot hold two ring sets.** VERIFIED from the header:
  one edge count and one rule (slots 52–53).
- **UNVERIFIED — the antialiased edge of a picture clipped to a flattened circle against
  a filled `Circle` of the same radius.** Phase 3 measures the worst pixel delta.

## 3. Design Overview

**A shaped picture is two draw entries, made at expansion.** `__canvas_appendDraw`'s
picture arm resolves the shape (through its `AutoPaint` state if it has one) and emits:

1. **The image entry** — a picture record over the fitted destination rectangle, with
   the shape's outline as its clip rings, and the shape's `fill`, `transparency` and
   `blend`.
2. **The outline entry** — the shape itself with a transparent fill and its own stroke,
   only when the stroke would draw.

Because both are ordinary entries, the software renderer, both GPU backends, damage and
the group matrix all handle them with no change.

**The plain-rectangle case is today's record.** A shape that is a `Rectangle` with four
square corners gets no clip rings: the destination rectangle is the shape, and the one
entry carries the stroke itself, exactly as today. Every ported `Picture` takes this
path, which is what keeps the picture goldens still.

**The outline.** `__canvas_shapeOutline(shape)` returns rings and a fill rule:
`Rectangle` through `__canvas_rrectRing`; `Circle` and `Ellipse` flattened to the same
0.05-pixel chord bound; `Polygon` as its own contours and rule. An open shape or `Text`
returns none and the picture draws nothing.

**Fit.** With bounds `B` and image size `I`: `Stretch` makes the destination `B`;
`Contain` scales `I` uniformly to fit inside `B` and centres it; `Cover` scales `I`
uniformly to cover `B` and centres it, the overflow removed by the clip. The destination
rectangle goes in the picture record's own rectangle slots, which is where the sampler
already reads it.

**Clip mode.** `ClipAndTransform`: `B` is the outline's local bounding box, the entry
carries the shape's placement transform, and the rings are local. `ClipOnly`: the rings
are mapped forward through the shape's placement, `B` is their axis-aligned bounding
box, and the entry carries no transform. Enclosing groups apply to both through the
draw entry's matrix.

**`paint.clip` on a shaped picture** (Open Decision 3 of 172-A, answered: intersect).
The image is visible where the shape outline **and** `paint.clip` both cover. The header
has one clip slot, so the two ring sets are intersected into one edge set at expansion:

- `__canvas_clipIntersect(ringsA, ruleA, ringsB, ruleB)` returns the boundary of
  `A ∩ B` as edges under the even-odd rule. It works by horizontal bands. The band
  boundaries are every vertex `y` of either set and every `y` at which two edges (of
  the same set or of different sets) cross. Inside one band no two edges cross, so each
  set's interior on any horizontal line through the band is a list of x-intervals
  bounded by the same edges, found by sorting the band's edges at its mid-line and
  applying that set's fill rule to the running winding count. The two interval lists
  are intersected. Each surviving interval contributes its left and right bounding
  edges, clipped to the band; and between two adjacent bands the horizontal boundary is
  the symmetric difference of the lower band's intervals at its top with the upper
  band's at its bottom. The result is the true outline of the intersection and nothing
  else — no edge interior to the region — which matters because clip coverage is taken
  from the distance to the nearest edge, and an interior edge would draw a seam.
- The edges are a soup, not ordered rings. The clip's inside test is a crossing count
  and its coverage a nearest-edge distance, and neither needs ring order; the even-odd
  rule is correct for a boundary each point of which separates inside from outside.
- The outline entry is the shape's stroke, and `paint.clip` clips it as it clips any
  shape's stroke: that entry carries `paint.clip` alone.
- An empty intersection draws no image entry.
- A plain-rectangle picture never calls it: its `paint.clip` goes in the slot as today.

**Gate class: new behaviour, with the plain-rectangle case pixel-neutral.**

### Rejected alternatives

- **A picture as a paint source on any shape's own distance field.** Rejected (172-A
  §3): exact, but it needs header slots that collide with the aux and cap slots closed
  shapes use, and a wider item block on both backends.
- **Silently ignoring `paint.clip` on a shaped picture.** Rejected: a clip that does
  nothing with no diagnostic is the failure mode this package's raises exist to prevent.
- **Raising when a shaped picture sets `paint.clip`.** This letter's original
  recommendation; rejected by the owner in favour of intersecting (172-A Open
  Decision 3).
- **A second clip slot, coverages multiplied.** Rejected: it grows the geometry header
  and the item block on both GPU backends, which this letter's first non-goal forbids.
- **Intersecting by trapezoids emitted as separate rings.** Rejected: adjacent
  trapezoids share edges inside the region, and nearest-edge coverage would dip to a
  half along every one of them.
- **Drawing the shape's fill behind the image.** Rejected by `planning/new_canvas.md`:
  `fill` on a picture is a tint.

## Compatibility / Format Impact

- **Changed:** `canvas::Picture`'s fields.
- **Added:** `canvas::PictureClip`, `canvas::PictureFit`. No error code (the raise is
  gone) and no `canvas::picture` constructor function (172-A Open Decisions 3 and 1).
- **Unchanged:** the picture geometry kind and record, `ITEM_BLOCK_SIZE`, both shaders,
  the image resource and its ownership rules, every pixel of a plain-rectangle picture.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Fit and outline, as pure functions

- [ ] Re-measure the two "on entry" rows and record them.
- [ ] `helper_shapes.rs`: `__canvas_pictureFit(bounds, imageSize, fit)` returning the
      destination rectangle; `__canvas_shapeOutline(shape)`.
- [ ] Unit cases: `Stretch`, `Contain` and `Cover` for a wide image in a tall box and a
      tall image in a wide box, each centred; the outline of a circle has every vertex
      within 0.05 pixels of the radius; an open shape returns no rings.
- [ ] `helper_shapes.rs`: `__canvas_clipIntersect(ringsA, ruleA, ringsB, ruleB)` per §3.
- [ ] Unit cases for it, each checked by area (the shoelace sum over the edge soup,
      which even-odd boundaries make well defined) and by point membership at fixed
      probes: two overlapping squares give the overlap square; disjoint squares give no
      edges; a square inside a circle outline gives the square; a five-point star under
      even-odd against a square differs from the same star under non-zero by exactly
      the star's inner pentagon; a ring with a hole against a bar crossing the hole
      gives two pieces; and no returned edge has both neighbours inside (the no-seam
      property), sampled 0.25 pixels either side of each edge's midpoint.

Acceptance: the fit and outline cases pass.
  Check: `cargo test --bin mfb canvas_picture_fit` → pass (est. 3 min).
Commit: —

### Phase 2 — The record and the port, plain rectangles only

- [ ] `mod.rs`: the two enums; `Picture` becomes `id` and the four fields; its
      description rewritten around the bullets in `planning/new_canvas.md`.
- [ ] `__canvas_appendDraw`: the picture arm, taking the plain-rectangle path for a
      plain rectangle and the two-entry path otherwise; `__canvas_pictureHeader` takes a
      destination rectangle, a paint and optional clip rings.
- [ ] The image-naming helpers (`__canvas_listNamesImage`, `__canvas_closeRetired`) and
      `canvas::setBytes`'s live-scene check read `Picture.image` as before; confirm with
      the ownership suite.
- [ ] `helper_hit.rs`: a picture is hit as its shape.
- [ ] `__canvas_appendDraw`'s picture arm joins the picture's `id` onto the path before
      its shape's, for both of a shaped picture's entries. Cases in
      `tests/canvas/rt_canvas_hit.rs`: two pictures built on one shape value report two
      paths; a named picture on an unnamed shape reports the picture's `id`; an unnamed
      picture on a named shape reports the shape's; a picture inside a named group
      reports group, picture, shape in that order.
- [x] ~~Add `canvas::picture(shape, image)` with a man example.~~ — moot: Open
      Decision 1 of 172-A was answered "no constructor functions"
      (`grep -c 'ANSWER: no constructor functions' planning/completed/plan-172-A-*.md`
      → 1). A `Picture` holds a `UNION` and a `RES`, so it has no default and is built
      with its five-field constructor.
- [ ] Port the 71 sites to a `Rectangle` shape with `ClipOnly` and `Stretch`. Each
      site's `id` stays on the `Picture`; the new `Rectangle`'s `id` is empty, so every
      site reports the `overID` it reported before.
- [ ] `FIELD_KIND_TABLE` and its `.tsv`; the IR-embedding goldens.

Acceptance: every picture test passes with no golden regenerated.
  Check: `cargo test --test rt_canvas_picture --test rt_canvas_group_ownership --test
  rt_canvas_graphics_thread --test rt_canvas_metal` → pass (est. 12 min); `git status
  --porcelain tests/golden/canvas` → empty.
Commit: —

### Phase 3 — Shaped pictures (largest blast radius)

- [ ] The two-entry path for `Circle`, `Ellipse`, `Polygon` and rounded `Rectangle`,
      both clip modes, all three fits.
- [x] ~~The raise for `paint.clip` on a shaped picture: allocate the next free `7705xxxx`
      code, add it to `src/docs/spec/diagnostics/02_error-codes.md`'s Constant Registry
      and `src/codegen/builtins/errorcode/mod.rs`, and raise it from the worker-side
      scene check in `func_present.rs` beside the group signature check.~~ — moot: Open
      Decision 3 of 172-A was answered "intersect the two clips"
      (`grep -c 'ANSWER: intersect the two clips' planning/completed/plan-172-A-*.md`
      → 1), so nothing is raised.
- [ ] The image entry's clip for a shaped picture that sets `paint.clip` is
      `__canvas_clipIntersect(outline, paint.clip)`; the outline entry carries
      `paint.clip` alone; an empty intersection emits no image entry.
- [ ] Scene `tests/canvas/scenes/picture_shapes_on_black.mfb` and golden
      `tests/golden/canvas/picture_shapes.png`: a picture in a circle, in a rounded
      rectangle, in a star polygon; `Contain` and `Cover` side by side; one shape
      rotated under each clip mode; one translucent picture; one inside a scaled group;
      one picture in a circle with a star `paint.clip` (the intersection).
- [ ] Runtime cases in `tests/canvas/rt_canvas_picture.rs`: a pixel just outside the
      circle is background; the stroke outlines the circle; a `Line` shape draws
      nothing; `setBytes` on the image repaints the shaped picture; an `AutoPaint` shape
      under a scripted hover changes the tint; a circle picture with a half-plane
      `paint.clip` shows image pixels only in the half inside both, is background in
      the half inside the circle alone, and has no seam (no pixel inside the
      intersection, more than 2 pixels from its boundary, differs from the unclipped
      picture's pixel there).
- [ ] Measure the clipped-circle edge against a filled circle and record the worst
      per-channel delta in Corrections.
- [x] ~~A `tests/rt-error/canvas/` fixture for the new raise (the directory is new;
      `ls tests/rt-error/canvas` → no such directory today).~~ — moot: there is no
      raise (Open Decision 3 of 172-A, as above).
- [ ] A Metal parity case; add the scene to `scripts/test-canvas-gpu-rows.sh`; run the
      Vulkan row on box 2226.
- [ ] `src/docs/spec/app/06_canvas.md` "Images are named, not embedded": the two-entry
      expansion, the fit rules, the two clip modes and the clip intersection, each cited.

Acceptance: the new golden renders identically twice; Metal and Vulkan match it within
`Tolerance::GPU_DEFAULT`; a shaped picture with a `paint.clip` shows the image exactly
where both cover, with no seam.
  Check: `cargo test --test rt_canvas_golden picture_shapes --test rt_canvas_picture
  --test rt_canvas_metal picture` → pass, and `scripts/test-canvas-vulkan.sh
  target/release/mfb --box 2226` → pass (est. 15 min).
Commit: —

## Validation Plan

- Tests: fit, outline and clip-intersection unit cases; six runtime cases; one new
  golden with Metal and Vulkan parity.
- Coverage check: the stats line for the new scene must show the GPU renderer on a GPU
  host — a shaped picture's clip edges count against the frame's edge cap, and a frame
  that silently declined would still match.
- Runtime proof: `examples/fractals-image` with its picture placed in a circle, run
  headless and dumped.
- Doc sync: `src/docs/spec/app/06_canvas.md`; `mfb man canvas types` rendered.
- Final gate: none in this letter; 172-M.

## Open Decisions

None beyond 172-A's Open Decision 3.

## Corrections

- **From 172-A, Open Decision 3: intersect, not raise.** §1's non-goal, §3, the
  rejected alternatives, Compatibility, Phase 1 (the intersection and its unit cases),
  Phase 3 (the raise, its error code and its `rt-error` fixture struck as moot; the
  intersection wired in; one more scene item and one more runtime case) and the
  Validation Plan are rewritten. `src/docs/spec/diagnostics/02_error-codes.md` is no
  longer touched by this letter.
- **From 172-A, Open Decision 1:** no `canvas::picture` constructor function.
- **Owner's decision, 2026-10-10: a `Picture` keeps its own `id`.** "Picture's id should
  prefix too… more than one picture/group can use the same shape." §1 (five fields; the
  id path in place of "its `id` is its shape's"), Phase 2 (the record, the walk and four
  hit cases, the port's rule) are rewritten, and `planning/new_canvas.md` with them. The
  path itself and its separator are 172-I's.

## Summary

A shaped picture is built from two things every renderer already draws — a picture
rectangle with a polygonal clip, and a stroked shape — so the feature costs no shader
and no layout. What it gives up is exactness at the clip edge, bounded at 0.05 pixels
and measured in Phase 3.
