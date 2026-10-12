# plan-172-L: `Picture` clipped to a shape

Last updated: 2026-10-11
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
| plan-172-K complete and archived | `ls planning/plan-172-K-*` → no matches | MET (2026-10-11: "no matches found") |
| `canvas::AutoShape` exists | `grep -c 'name: "AutoShape"' src/codegen/builtins/canvas/mod.rs` → 1 | MET (2026-10-11: 1) |
| Open Decision 3 of 172-A is answered | `grep -c '\*\*ANSWER:' planning/completed/plan-172-A-*.md` → 5 | MET (2026-10-11: 5) |
| The picture suites are green | `cargo test --test rt_canvas_picture --test rt_canvas_group_ownership` → pass | MET (2026-10-11, in 172-K's closing round: "11 passed; 0 failed" each) |

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
  inside a group `"card"`, reports `"card::avatar::frame"`, with any empty part left
  out. When the shape is an `AutoPaint`, the part is the wrapped shape's `id`.

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

- [x] Re-measure the two "on entry" rows and record them. `canvas::Picture[` sites: 72
      in 25 files (the §2 command; 71 in 24 when the plan was written — 172-K's scene
      and tests did not add one, `func_set_bytes.rs`'s man example was miscounted).
      Reads of a picture's placement and paint in the companion source: 41
      (`grep -rhoE '\bpic\.(position|size|transform|paint)\b'
      src/codegen/builtins/canvas | wc -l`; the 8 was `pic.x|y|w|h`, before 172-E).
      Highest `7705xxxx`: still 77050027, and this letter no longer takes one.
- [x] `helper_shapes.rs`: `__canvas_pictureFit(bounds, imageSize, fit)` returning the
      destination rectangle; `__canvas_shapeOutline(shape)`. In a new file,
      `helper_picture.rs`, as the chunk `PICTURE_GEOMETRY`, gated on the program
      constructing a `Picture` — `helper_shapes.rs` is compiled into every canvas
      program and this is not for every one (Corrections). `__canvas_pictureFit`
      takes the box, the image size and the fit as numbers; the outline's curved case
      is `__canvas_ellipseRing`, and the per-shape choice between it,
      `__canvas_rrectRing` and a polygon's own contours is Phase 2's, where the record
      is.
- [x] Unit cases: `Stretch`, `Contain` and `Cover` for a wide image in a tall box and a
      tall image in a wide box, each centred; the outline of a circle has every vertex
      within 0.05 pixels of the radius; an open shape returns no rings. Four tests. The
      circle's vertices are on it to `1e-4` px and its chords within 0.05; "an open
      shape returns no rings" is Phase 3's `Line` case, since the shapes are not in this
      chunk, and here a zero radius returns none.
- [x] `helper_shapes.rs`: `__canvas_clipIntersect(ringsA, ruleA, ringsB, ruleB)` per §3.
      In `PICTURE_GEOMETRY`, by bands as §3 says, with two things §3 does not say: a
      source edge that bounds the region through consecutive bands is emitted as ONE
      edge, and the edges are oriented, so their shoelace sum is the area.
- [x] Unit cases for it, each checked by area (the shoelace sum over the edge soup,
      which even-odd boundaries make well defined) and by point membership at fixed
      probes: two overlapping squares give the overlap square; disjoint squares give no
      edges; a square inside a circle outline gives the square; a five-point star under
      even-odd against a square differs from the same star under non-zero by exactly
      the star's inner pentagon; a ring with a hole against a bar crossing the hole
      gives two pieces; and no returned edge has both neighbours inside (the no-seam
      property), sampled 0.25 pixels either side of each edge's midpoint. Five tests,
      and the seam count is asserted zero in every one of them. The run-joining is its
      own case: a 32-point circle cut by a half-plane comes back as 17 edges.

Acceptance: the fit and outline cases pass.
  Check: `cargo test --test rt_canvas_picture_fit` → pass (est. 3 min; the plan wrote
  `cargo test --bin mfb canvas_picture_fit`, see Corrections).
  Observed 2026-10-11: "9 passed; 0 failed".
Commit: a32506096

### Phase 2 — The record and the port, plain rectangles only

- [x] `mod.rs`: the two enums; `Picture` becomes `id` and the four fields; its
      description rewritten around the bullets in `planning/new_canvas.md`. Rendered:
      `mfb man canvas types` shows `canvas::PictureClip`, `canvas::PictureFit` and the
      five-field record.
- [x] `__canvas_appendDraw`: the picture arm, taking the plain-rectangle path for a
      plain rectangle and the two-entry path otherwise; `__canvas_pictureHeader` takes a
      destination rectangle, a paint and optional clip rings. As
      `helper_picture.rs:PICTURE` (Corrections): `__canvas_pictureAppend`,
      `__canvas_picturePlainOffset` / `__canvas_picturePlain` for the plain record,
      `__canvas_pictureImageOf` for a shaped picture's image, `__canvas_pictureStamp`
      for what both share.
- [x] The image-naming helpers (`__canvas_listNamesImage`, `__canvas_closeRetired`) and
      `canvas::setBytes`'s live-scene check read `Picture.image` as before; confirm with
      the ownership suite. `cargo test --test rt_canvas_group_ownership` → "11 passed; 0
      failed". One thing is new: a `Picture` on a `Text` names that text's FONT, so the
      two font helpers count it (`__canvas_pictureNamesFont`,
      `__canvas_pictureCloseFont`).
- [x] `helper_hit.rs`: a picture is hit as its shape. `__canvas_hitAppend`'s `Picture`
      arm goes round again with `__canvas_pictureHitItem`.
- [x] `__canvas_appendDraw`'s picture arm joins the picture's `id` onto the path before
      its shape's, for both of a shaped picture's entries. Cases in
      `tests/canvas/rt_canvas_hit.rs`: two pictures built on one shape value report two
      paths; a named picture on an unnamed shape reports the picture's `id`; an unnamed
      picture on a named shape reports the shape's; a picture inside a named group
      reports group, picture, shape in that order.
      `id_path_a_picture_goes_in_front_of_its_shape`: those four, and three more in the
      same scene — a point inside a rounded rectangle's box and outside its corner is
      over nothing, a point inside a circle's box and outside the circle likewise, and a
      picture on a `Line` is not there to be hit. `cargo test --test rt_canvas_hit` →
      "13 passed; 0 failed". (The walk is `__canvas_hitAppend`, as 172-I's hand-off
      note says; `__canvas_appendDraw` carries no id path.)
- [x] ~~Add `canvas::picture(shape, image)` with a man example.~~ — moot: Open
      Decision 1 of 172-A was answered "no constructor functions"
      (`grep -c 'ANSWER: no constructor functions' planning/completed/plan-172-A-*.md`
      → 1). A `Picture` holds a `UNION` and a `RES`, so it has no default and is built
      with its five-field constructor.
- [x] Port the 71 sites to a `Rectangle` shape with `ClipOnly` and `Stretch`. Each
      site's `id` stays on the `Picture`; the new `Rectangle`'s `id` is empty, so every
      site reports the `overID` it reported before. 70 literals in 23 files, by
      `/tmp/plan-172-A/port_pictures.py`; the other two of the 72 counted on entry are
      a comment in `mod.rs` about the old `imageRef` form and a golden `build.log` that
      echoes a ported source line. Three sites are `ClipAndTransform`, not `ClipOnly`
      (Corrections).
- [x] `FIELD_KIND_TABLE` and its `.tsv`; the IR-embedding goldens. The table and the
      `.tsv` are unchanged and right: `canvas.Picture` keeps its row (it is still a
      record that cannot be built by default), and the two new types are enums, which
      have none — `cargo test --test inplace_self_update_census` → "2 passed", `cargo
      test --bin mfb field_kind_census` → "1 passed". The goldens that embed the
      companion source were resynced after the whole round: `scripts/artifact-gate.sh
      target/release/mfb all` reported 8 diffs, all in `syntax/app/app-mouse-surface`
      and `syntax/resources/loop-body-resource-reference-valid`, and after
      `sync-goldens.sh` and `regen-native-goldens.sh` reports "2322 golden(s) checked, 0
      diff(s)". Two `build.log` goldens moved with them, each for a stated reason
      (Corrections).

Acceptance: every picture test passes with no golden regenerated.
  Check: `cargo test --test rt_canvas_picture --test rt_canvas_group_ownership --test
  rt_canvas_graphics_thread --test rt_canvas_metal` → pass (est. 12 min); `git status
  --porcelain tests/golden/canvas` → empty.
  Observed 2026-10-11, before any shaped case existed: "11 passed", "11 passed", "17
  passed; 0 failed; 2 ignored", "50 passed", and the status empty. In the closing round:
  16, 11, 17 and 52 passed, and the status is one line, `?? tests/golden/canvas/
  picture_shapes.png` — Phase 3's new reference; no existing one moved.
Commit: b83b09637

### Phase 3 — Shaped pictures (largest blast radius)

- [x] The two-entry path for `Circle`, `Ellipse`, `Polygon` and rounded `Rectangle`,
      both clip modes, all three fits. `__canvas_pictureBlocksOf` and
      `__canvas_pictureImage`; a `Rectangle` with square corners takes it too whenever
      it is not the plain case (a fit other than `Stretch`, or `ClipOnly` under a
      transform).
- [x] ~~The raise for `paint.clip` on a shaped picture: allocate the next free `7705xxxx`
      code, add it to `src/docs/spec/diagnostics/02_error-codes.md`'s Constant Registry
      and `src/codegen/builtins/errorcode/mod.rs`, and raise it from the worker-side
      scene check in `func_present.rs` beside the group signature check.~~ — moot: Open
      Decision 3 of 172-A was answered "intersect the two clips"
      (`grep -c 'ANSWER: intersect the two clips' planning/completed/plan-172-A-*.md`
      → 1), so nothing is raised.
- [x] The image entry's clip for a shaped picture that sets `paint.clip` is
      `__canvas_clipIntersect(outline, paint.clip)`; the outline entry carries
      `paint.clip` alone; an empty intersection emits no image entry. The empty record
      is cached under the image's key and the entry left out wherever a record's kind is
      NONE, so the two walks decide it the same way.
- [x] Scene `tests/canvas/scenes/picture_shapes_on_black.mfb` and golden
      `tests/golden/canvas/picture_shapes.png`: a picture in a circle, in a rounded
      rectangle, in a star polygon; `Contain` and `Cover` side by side; one shape
      rotated under each clip mode; one translucent picture; one inside a scaled group;
      one picture in a circle with a star `paint.clip` (the intersection). And four
      more: a tinted triangle, a rectangle with four different corners under `Cover`, a
      picture whose shape is an `AutoPaint`, and a picture on a `Line`.
- [x] Runtime cases in `tests/canvas/rt_canvas_picture.rs`: a pixel just outside the
      circle is background; the stroke outlines the circle; a `Line` shape draws
      nothing; `setBytes` on the image repaints the shaped picture; an `AutoPaint` shape
      under a scripted hover changes the tint; a circle picture with a half-plane
      `paint.clip` shows image pixels only in the half inside both, is background in
      the half inside the circle alone, and has no seam (no pixel inside the
      intersection, more than 2 pixels from its boundary, differs from the unclipped
      picture's pixel there). Five tests there —
      `a_shaped_picture_is_its_image_inside_the_shape_and_its_stroke_on_top`,
      `contain_fits_inside_the_shape_and_cover_is_cut_to_it`,
      `clip_only_keeps_the_image_upright_and_clip_and_transform_turns_it`,
      `set_bytes_repaints_a_shaped_picture`,
      `a_shaped_picture_with_a_clip_shows_only_where_both_cover` (every pixel more than 2 px
      inside the intersection is compared, and there must be more than 3,000) — and the `AutoPaint`
      one in `rt_canvas_autopaint.rs`, where the scripted-pointer runner is:
      `a_picture_on_an_autopaint_shows_the_state_paint_as_its_tint_and_outline`, in
      the scene and in a group drawn twice. "16 passed" and "18 passed".
- [x] Measure the clipped-circle edge against a filled circle and record the worst
      per-channel delta in Corrections. 61 of 255; the entry below says where and why.
- [x] ~~A `tests/rt-error/canvas/` fixture for the new raise (the directory is new;
      `ls tests/rt-error/canvas` → no such directory today).~~ — moot: there is no
      raise (Open Decision 3 of 172-A, as above).
- [x] A Metal parity case; add the scene to `scripts/test-canvas-gpu-rows.sh`; run the
      Vulkan row on box 2226. Two Metal cases
      (`picture_shapes_match_the_software_oracle_on_metal`,
      `a_hovered_autopaint_picture_matches_the_software_oracle_on_metal`) and two rows
      (`pictureshapes`, `pictureauto`). On box 2226: "ok: pictureshapes: Vulkan matches
      the software oracle (ok worst=1 differing=0.9155%, 168562 lit)", "ok: pictureauto:
      … (ok worst=1 differing=0.9144%, 169198 lit)", `geoVerifyMismatches=0` on all
      four runs. The same two rows on box 2227 (x86_64, musl, lavapipe): the same two
      lines. On box 206 (Windows, Intel UHD 630): "worst=1 differing=1.2231%" and
      "worst=1 differing=1.2191%" — after the scene was changed for bug-1079
      (Corrections).
- [x] `src/docs/spec/app/06_canvas.md` "Images are named, not embedded": the two-entry
      expansion, the fit rules, the two clip modes and the clip intersection, each cited.
      A new subsection there, "A picture is drawn inside a shape", and with it: "The
      item unions" (the hash that folds a union field), "Rendering conventions" (what a
      picture RECORD is), "The walk, and the id path" (four rows and the hit rule), and
      "AutoPaint — One frame" (an instance that is a picture's shape).
      `cargo test --bin mfb spec` → "46 passed".

Acceptance: the new golden renders identically twice; Metal and Vulkan match it within
`Tolerance::GPU_DEFAULT`; a shaped picture with a `paint.clip` shows the image exactly
where both cover, with no seam.
  Check: `cargo test --test rt_canvas_golden picture_shapes --test rt_canvas_picture
  --test rt_canvas_metal picture` → pass, and `scripts/test-canvas-vulkan.sh
  target/release/mfb --box 2226 --target linux-aarch64` → pass (est. 15 min; the
  `--target` is needed, the script's default is x86_64).
  Observed 2026-10-11: "1 passed" (`picture_shapes_match_their_reference_exactly`, which
  renders twice and compares), "16 passed", "3 passed"; "canvas Vulkan runtime tests
  passed". The whole-letter round (`/tmp/plan-172-A/round_l.sh`: 39 test targets, the
  in-binary `canvas`, `spec`, wider and census subsets): every target passed, 0 FAILED
  lines.
Commit: b83b09637

## Validation Plan

- Tests: fit, outline and clip-intersection unit cases; six runtime cases; one new
  golden with Metal and Vulkan parity.
- Coverage check: the stats line for the new scene must show the GPU renderer on a GPU
  host — a shaped picture's clip edges count against the frame's edge cap, and a frame
  that silently declined would still match.
- Runtime proof: `examples/fractals-image` with its picture placed in a circle, run
  headless and dumped. Done 2026-10-11 on a scratch copy
  (`/tmp/plan-172-A/fractal_circle.sh`: the `Rectangle` swapped for a `Circle` with a 4
  px outline, `Cover`): 25 s on the GPU path wrote 1,145 stats lines, every one
  `gpuFrames` equal to `frames` and `blocks=5` — the picture's two records and three
  runs of text — and the dumped frame is the fractal inside the circle under its
  outline.
- Doc sync: `src/docs/spec/app/06_canvas.md`; `mfb man canvas types` rendered.
- Final gate: none in this letter; 172-M.

## Open Decisions

None beyond 172-A's Open Decision 3.

## Corrections

- **Phases 2 and 3 are one change (2026-10-11).** The record cannot hold a shape
  without something drawing a shape that is not a plain rectangle, so the two-entry path
  was written with the record and the two phases share a commit. Phase 2's acceptance
  was run first, on the port alone, before any shaped case existed; Phase 3's after.
- **The picture code is its own gated chunk, and `__canvas_geometryFor` no longer builds
  a picture.** The plan put a three-argument `__canvas_pictureHeader` in the
  unconditional source. As built, `helper_picture.rs:PICTURE` is compiled only into a
  program that constructs a `Picture`, with an `ABSENT` twin of eight one-line hooks for
  every other (172-K's pattern); the walks hand a picture to it before
  `__canvas_geometryFor`, whose picture arms are gone. Measured on macOS AArch64
  (`__text`, `otool -l`, against the compiler built at `88005fb60`): a canvas program
  with no picture is 16,904 bytes SMALLER (2,734,204 → 2,717,300); the tilemap, all
  plain pictures, is 226,012 larger (2,823,996 → 3,050,008); an `AutoPaint` program
  with no picture 10,392 larger. The tilemap's per-frame cost is unchanged within what
  a loaded machine can show: `tools/canvas-bench` `pics 4000 16 0 0`, alternated with
  the baseline, `phaseOffsetsMs` 15.6 / 19.9 / 19.1 / 17.1 before and 15.7 / 21.8 /
  18.5 / 17.6 after — once the plain picture had its own path
  (`__canvas_picturePlainOffset`); the first cut, which went through the shaped path's
  probes first, read 21.6 to 28.3.
- **Three ported sites are `ClipAndTransform`.** The plan says every site becomes
  `ClipOnly`. A picture with a transform used to turn its image with it, and under
  `ClipOnly` it would not, so the three sites whose transform is not the zero literal
  (`rt_canvas_picture.rs`'s rotated strip, `rt_canvas_metal.rs`'s `rotated`,
  `test-canvas-vulkan.sh`'s `picScaled`) are `ClipAndTransform` — which is the plain
  path, and what keeps their pixels.
- **The outline entry is there for more than a stroke.** The plan: "only when the
  stroke would draw". Also under `Contain`, where the image may stop short of the
  shape, and always for an `AutoPaint` shape — in both, the outline is what makes the
  whole shape, not only the image, what the pointer is over, and for an `AutoPaint` it
  is the entry a state paint's stroke replaces.
- **A picture on an `AutoPaint` is always two entries, and its host is kept by the
  picture chunk.** `__canvas_autoNote` takes the instance and a number;
  `__canvas_pictureAutoBlocks` turns the number back into the picture. The first cut
  kept a `List OF DrawItem` of hosts in the `AutoPaint` chunk, which cost every
  `AutoPaint` program 19,324 bytes for a picture it may not have.
- **The image is fitted to the shape's EXACT box.** §3 says "`B` is the outline's
  bounding box". For a circle that is the true box only to the accuracy of this
  package's sine, about 4e-5 px, and a destination that far off a whole number moves a
  texel boundary off a pixel centre: Metal and the oracle then picked different texels
  along a whole row (`picture_shapes_match_the_software_oracle_on_metal`, first run:
  "364 of 576000 pixels differ (max channel delta 220)", 44 of them beyond tolerance).
  A rectangle's, a circle's and an ellipse's box is now computed, not measured.
- **One antialiased edge, not two.** Not in the plan. A record's coverage is its
  rectangle's times its clip's, so where the image's edge lies along the outline a
  half-covered pixel came out a quarter covered (measured: 137 against a filled
  circle's 188 at the circle's leftmost pixel). The image record's radius slot is set
  to 1 with its box whole, which moves the rectangle's own edge a pixel out and the
  image not at all — all three renderers take a rectangle's distance as box less
  radius and sample a picture from the box — and under `Contain` the outline is
  intersected with the image's rectangle. A `Contain` picture whose paint also has a
  clip keeps the rectangle's edge: that would be a second intersection, and a record
  has room for one.
- **The clipped-circle edge against a filled circle (the Phase 3 measurement).**
  `/tmp/plan-172-A/edge_probe.mfb`: a one-white-texel picture in a circle beside a
  filled white circle of the same radius, at three sizes and places (radius 50 on whole
  pixels, 37.25 at a fractional centre, 140 at a half-pixel centre). 1,456 of the
  74,508 pixels either covers differ; the worst is 61 of 255, at (63, 242), which the
  circle covers 4.7% and the flattened outline does not reach. That is the 0.05 px
  chord bound and nothing more: the frame is sRGB-encoded, and 0.047 of linear
  coverage is 61 code values at the dark end. The histogram falls away fast — 1,143 of
  the 1,456 differ by 10 or less. Where the image's edge and the outline coincide the
  two are now equal (188 and 188).
- **A top-level picture of two entries is −2 in `__CANVAS_TOP_OFFSETS`, and the GPU walk
  takes its blocks from the frame's entries.** Not in the plan, which says "both are
  ordinary entries, [so] both GPU backends … handle them with no change". The native
  layout lays one block per scene index, so an index that is two entries must send the
  frame to the MFBASIC draw walk; and that walk must not ask the picture again — the
  first cut did, and the runtime proof's last frame, drawn as the program closed its
  image, reported one block for a picture whose two entries the same frame had drawn.
- **The native item hash folds a picture's shape; `canvas::sceneIdBits` reads its
  `id`.** Not in the plan. A picture on a `Rectangle` — a tilemap — would otherwise
  have dropped to the MFBASIC hash and to a walk of the scene for every `pollMouse`
  build. The walk folds the picture's other fields, then branches back to its own
  dispatch with the shape's record (`emit_item_hash`).
- **Two `build.log` goldens moved, each for a stated reason.**
  `canvas-setgroup-consumes-items` echoes a ported source line. `canvas-drawitem-
  thread-plane-invalid` names the first resource the type walk finds in a `DrawItem`,
  and a picture's first is now its shape's — a `Text`'s font — where it was its image:
  "`RES canvas.Font`" for "`RES canvas.Image`". The fixture pins that a scene cannot
  cross the data plane (plan-116-I, `a274f147b`), which it still does, with the same
  code; `mfb spec app canvas` says "A `Picture` or a `Text` carries a resource" and
  "Move the `Image` or `Font`", either.
- **bug-1079, found here and not this letter's.** The scene's first cut failed on box
  206's real GPU alone: "worst=161 … first-beyond-tolerance=(470, 262, …)". Two of its
  shapes put a texel boundary exactly on a pixel centre, where the texel a renderer
  takes hangs on a float32 division being exact, and Intel's is not. A PLAIN picture
  does the same there with the compiler built at `88005fb60`
  (`bugs/repro/bug-1079/texel_edge.mfb`: "worst=220 differing=0.0278%"), so it is filed
  and the scene's sizes were changed to keep every boundary off a pixel centre. The fix
  is one expression in each GPU shader and a SPIR-V regeneration.
- **The `--debug` geometry check does not rebuild a picture.** A picture's records have
  one builder — the native one declines a picture — so there is nothing to hold one
  against, and `__canvas_geoReference` answers the record under test. A shaped
  picture's outline is checked where it is built, as the item it is.
- **`examples/canvas-demo` was ported, not rewritten.** Its nine `Picture` literals got
  the mechanical port every site got, so it builds; the showcase rewrite the owner
  asked for is still to be authorised.

- **Phase 1's cases are an integration test, and its functions are in their own gated
  file (2026-10-11).** As in 172-K Phase 2: a `--bin mfb` test cannot run MFBASIC, so
  `tests/canvas/rt_canvas_picture_fit.rs` reads `PICTURE_GEOMETRY` out of
  `helper_picture.rs` (and the float and trigonometry chunks of `helper_shapes.rs` it
  calls), compiles it behind a driver and runs it. The functions are not in
  `helper_shapes.rs` because that file is compiled into every canvas program, and
  172-K measured what unconditional code costs; the chunk is gated on the program
  constructing a `Picture` (`HelperGate::WhenUsedOrConstructed`).
- **`toString` of a `Float` keeps six significant digits (2026-10-11).** The first cut
  of the driver printed areas with it and could not tell the star's pentagon (34.6893)
  from 34.69. Areas and distances are printed as whole millionths.
- **The outline is as exact as the canvas's own sine and cosine (2026-10-11).**
  `__canvas_ellipseRing` uses `__canvas_sin` / `__canvas_cos`, which are deterministic
  across platforms and good to `4.6e-7`, not `math::sin`. A vertex is on the circle to
  `1e-4` px at radius 100, which is 500 times inside the 0.05 bound the chord has.
- **The owner confirmed `canvas::AutoPaint`'s field names (2026-10-11):** "Field names
  are fine." The bullet under "From 172-K as built" that says "pending the owner's
  word" is settled.

- **From 172-K as built (2026-10-11): what a `Picture` that holds an `AutoShape` has to
  fit into.** Read `.ai/canvas-threading.md` section 16 and `mfb spec app canvas`,
  "AutoPaint", first. The parts this letter touches:
  - **An `AutoPaint` is found by the WALK, and only by the walk.** Its geometry is never
    cached under the item's own hash, so the native first pass always leaves it for
    `__canvas_appendDraw`, whose `AutoPaint` arm (`__canvas_autoAppend`) notes the
    instance. An `AutoPaint` reached through a `Picture`'s `shape` is an instance only
    if the picture's walk calls that same hook, with a `path` and `index` that name its
    place (`__canvas_autoPath`) — and the GPU walk (`__canvas_memoGroup`,
    `__canvas_autoBlock`) must derive the same key.
  - **A state paint is a different RECORD, swapped in after the hit test.** For a
    picture whose shape is an `AutoPaint`, "the state paints change the tint and
    outline" means the picture's record is rebuilt under the state's paint and cached
    under a stepped hash, as `__canvas_autoGeometry` does for a shape.
  - **The item hash and the shape converter are behind a gate.** `__canvas_autoItem`,
    `__canvas_autoHash` and ten more names exist twice — the real chunk, compiled into a
    program that constructs an `AutoPaint`, and `ABSENT` for every other. A new hook
    goes into BOTH, with one signature
    (`canvas_auto_hooks_are_declared_alike_in_both_chunks`). They are there because an
    always-compiled arm that copies a record costs kilobytes in every canvas program:
    measure `__text` (`otool -l`) before leaving one unconditional.
  - **`canvas::AutoPaint`'s paint fields are `enterPaint`, `exitPaint`, `hoverPaint`,
    `pressedPaint`** (`EXIT` is a keyword), pending the owner's word on the names.
  - **The `--debug` geometry check is handed the ITEM.** `__canvas_geoReference` needs
    an arm for whatever a `Picture` becomes, and only the remote rows
    (`scripts/test-canvas-gpu-rows.sh`) set `MFB_CANVAS_GEO_VERIFY`.
  - **Damage is off by default** and a test must never time anything from a headless
    program's first frame — both in section 11 and section 4 of the threading notes.
- **From 172-I as built (2026-10-11): the id path is not carried by
  `__canvas_appendDraw`.** Phase tasks that say "`__canvas_appendDraw`'s picture arm
  joins the picture's `id` onto the path" mean `helper_hit.rs:__canvas_hitAppend`, the
  worker's own walk; `__canvas_appendDraw` has no path. And a `Picture` on the worker is
  not the renderer's record: `__canvas_hitGeometry` builds it as its rectangle
  (`__canvas_rectHeader`), with no pixel block. "A picture is hit as its shape" changes
  that arm. `canvas::sceneIdBits` reads `id` as each variant's FIRST field and fails the
  build otherwise, which `Picture`'s new field order (`id` first) keeps true.

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
