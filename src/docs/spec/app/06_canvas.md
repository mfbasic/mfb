# Canvas

The 2D drawing surface `app::setMode(Mode.Canvas)` presents, and the model the
`canvas` package draws on it. This topic specifies the *contract* — what a scene
is, what installing one guarantees, and how images live; the per-call API is
`./mfb man canvas`, and the mode itself is `./mfb spec app presentation-mode`.

## Coordinates are `vector::Float2`

Every coordinate pair in the package — a gradient's `startPoint`, `endPoint` and
`focalOffset`, a `Paint.clip` ring, a `Polygon`'s contours, a `PolyLine`'s points and
`MouseEvent.position` — is a `vector::Float2`, the two-`Float` record `x`, `y` owned by
`vector`. The package's own `Point` record is gone; naming it is `SYMBOL_UNKNOWN_IDENTIFIER`.
[[src/codegen/builtins/vector/mod.rs:FLOAT2_TYPE_ID]]

`canvas` imports `vector` itself, and `vector`'s source is injected by its own late
pass, so a program that imports only `canvas` still builds.
[[src/codegen/builtins/vector/mod.rs:augmented_project]] Imports are not transitive,
though: a program that constructs a pair, or reads `.x` / `.y` from one, must write
`IMPORT vector` (`./mfb spec language modules-and-packages`). The import brings in the
whole of `vector`'s source; for a program that presents an empty scene that is 330,236
bytes of executable on macOS aarch64.

## Retained, not immediate

`canvas` is a **retained scene**: a program builds a `List OF DrawItem` and
installs it with `canvas::present`, and the runtime redraws that scene when a
present changes it, on resize, and on OS damage — until the next `present`
replaces it. Time is a trigger in one bounded case — a `canvas::AutoPaint` inside its
enter or exit window (below, "AutoPaint") — and no backend waits for the display: a frame
is drawn when one is owed, never on a display-refresh tick.
[[src/codegen/builtins/canvas/mod.rs:CANVAS]]

This is a deliberate divergence from `term::`, which is ambient mutation plus a
present-diff. The consequences are the point:

- **`present` is not a per-frame call.** A static picture is presented once and
  costs nothing thereafter. A program that never changes its content never calls
  `present` again.
- **Re-presenting identical content is free of publication.** `present` compares
  the incoming scene against the installed one and returns without republishing
  when they match, so an animation loop that redraws an unchanged frame does not
  make the renderer redraw.
- **Per-item geometry can be cached on content**, because an item's identity is
  its contents rather than a call ordering. Re-presenting an unchanged item
  re-uses its cached geometry.

The comparison is exact rather than hashed: both sides are shrink-to-fit copies,
so equal content is equal bytes and there are no collisions to reason about.

### Flat or layered

A scene is installed in one of two shapes. `canvas::present` takes a flat
`List OF DrawItem`; `canvas::presentLayers` takes a `List OF DrawLayer` that
composite in order.

Layers exist to **separate what changes from what does not** — a static backdrop
under a moving sprite, fixed axes under a live series — so that a layer whose
contents did not change reuses its cached geometry wholesale and only the layer
that moved costs anything.

A scene is exactly one shape at a time: installing one replaces the other, so
switching shapes always publishes. Both calls share one implementation and
therefore one set of guarantees; they differ only in what they copy.

### The item unions

What a scene holds is three nested unions, each a registry union that `INCLUDES` the
one before it (`./mfb spec language types`):

| Union | Members | What it is for |
|---|---|---|
| `canvas::Shape` | `Rectangle`, `Line`, `Polygon`, `Circle`, `Arc`, `Text`, `Ellipse`, `PolyLine` | the eight things drawn with a paint of their own; what an `AutoPaint` wraps |
| `canvas::AutoShape` | `Shape`'s eight, then `AutoPaint` | a shape, plain or decorated |
| `canvas::DrawItem` | `AutoShape`'s nine, then `Picture`, `Group` | what `present` takes |

An included union's members come first, in its own order, so the three share tags: a
`Rectangle` is tag 0 in all of them, an `AutoPaint` tag 8 in the two that have it. The
order is pinned by three tests, one per union.
[[src/codegen/builtins/canvas/mod.rs:draw_item_variant_set_is_frozen]]
`INCLUDES` is not subtyping: a `Shape` value is not a `DrawItem`, but each member record
is a member of every union that reaches it, which is why `canvas::Rectangle[...]` is
accepted where either is expected.
[[src/codegen/registry/mod.rs:union_has_member]]

Nothing may depend on a tag's number. The native walks that branch on a variant ask the
type model for its tag by name, and each keeps a list of the variants it handles and the
variants it leaves to the MFBASIC path; a variant in neither fails the build.
[[src/codegen/builtins/canvas/func_item_hash.rs:NOT_HASHED]]

A `canvas::AutoPaint` holds a `Shape` and draws it. It has no `id`, no `position`, no
`paint` and no `transform` of its own — the shape's are its — so wherever a shape is
what is asked about (what is drawn, where it is hit, which `id` is reported, which font
is named), every `MATCH` over `DrawItem` answers its `AutoPaint` arm with the shape
(`__canvas_autoItem`). [[src/codegen/builtins/canvas/helper_autopaint.rs:RESOLVE]]
Its `shape` field is a union, so the record has no default value: an `AutoPaint` is
written as a full literal, not built up from `MUT a AS canvas::AutoPaint`. It is hashed
in MFBASIC — its shape's hash, its four flags and its four paints — because the native
hash does not fold a union field, and it is never carried from the previous scene's
hashes, because the shape it wraps may be a `Text`, whose hash folds its font's id.
[[src/codegen/builtins/canvas/func_carried_hashes.rs:lower_carried_hashes]]

## A published scene points at nothing caller-owned

`present` **deep-copies the scene transitively** into runtime-owned storage:
item fields, a `Polygon`'s rings, a `Text`'s string, the `Paint` values.
After it returns, nothing in the installed scene references anything the caller
owns.

This is not a convenience — it is what makes the model sound. The renderer reads
the installed scene at arbitrary times after `present` returns, with no further
involvement from the program, so a scene pointing at caller storage would be read
after that storage was reused. The copy is one operation rather than a walk,
because an MFBASIC collection is already a self-contained flat block: strings,
records and nested collections are inlined into it, not referenced from it.

The copy's cost is charged to the calling program's frame budget, by design.
Animated content calls `present` every frame and pays for it there; the renderer's
own per-frame cost stays constant.

## Paint is a value, not ambient state

Every drawn item carries a `Paint`. There is no "current colour" to set, no state
stack, and no drawing context.

Ambient state interacts badly with a retained scene: the question "which fill was
current when item 47 was appended?" has no good answer when the scene is a value
the program can build in any order, reorder, or reuse. Threading the paint through
each item makes an item's appearance a property of the item.

`Paint` is designed so that **each field's zero value is that field's no-op** —
transparent fill and stroke, zero stroke width, `Normal` blend, the identity
transform, an empty clip and no transparency. One consequence is worth stating
explicitly, because the natural reading is the opposite: **the all-zero
`Transform` means the identity**, not the degenerate matrix that collapses every
point to the origin. Defining it the other way would make an unset transform erase
the drawing.

MFBASIC named construction requires every field of a record, so a `Paint` is built
with `canvas::fill`, `canvas::stroke` or `canvas::fillStroke` and refined with
`WITH`.

## Coordinates and angles

Coordinates are pixels with a **top-left origin and Y increasing downward**.
Angles are **radians, measured clockwise from +X** — which, under Y-down, is the
direction that makes a `0`..`PI` arc sweep *below* its centre.

The convention is stated because an arc is the one primitive where getting it
wrong is silent: a smile renders as a frown rather than failing.

An `Ellipse` has no angle of its own. `radiusX` lies along the X axis and `radiusY`
along the Y axis, and it is turned, like every other item, by its `transform` (below);
header slots 39 and 40, which carried the cosine and sine of the old `angle` field,
are written as the identity rotation, 1 and 0.
[[src/codegen/builtins/canvas/helper_geometry.rs:GEO_HEADER]] `radiusX = radiusY`
renders identically to the `Circle` of that radius, so squashing a circle does not mean
changing item type.

### Placement

Every shape is placed by `position`: the centre for a `Circle`, an `Ellipse` and an
`Arc`, the left end of the baseline for a `Text`, the top-left corner for a `Picture`,
for a `Rectangle` the point its `bind` names — its top-left corner (`BindTo.TopLeft`,
the zero value) or its centre (`BindTo.Center`) — and for a `Line`, a `Polygon` and a
`PolyLine` the point their points are measured from. Their geometry is **local** — it
has its origin at that reference point — and a local point `p` reaches the surface at

    position + transform(p)

before any enclosing group's translation. So the transform **pivots about the reference
point**: a rotation turns a circle, an ellipse or an arc about its centre, a text about
the start of its baseline, a `Center` rectangle in place and a `TopLeft` one about that
corner; and `transform.tx` / `ty` add to `position`.

**The point-list shapes hold offsets.** A `Line`'s `startAt` and `endAt`, every vertex
of a `Polygon`'s `contours` and every point of a `PolyLine`'s `points` is an offset from
the item's `position`. `position` is a reference point of the program's choosing and
need not be a vertex: a polygon whose points surround `(0, 0)` turns about its middle,
and one whose `position` is the surface origin has its points in surface pixels. A
`Line` names three points for that reason — its `position` is the pivot, and it can be
anywhere along the line or off it.

`paint.clip` rings and a gradient's `startPoint` and `endPoint` are in the same local
space, so a shape moves whole when only its `position` changes — its clip and its ramp
go with it. `focalOffset` was always an offset from `startPoint` and is unchanged.

The builders do not apply that formula uniformly, and the difference is not observable:

* **A transform that is not in effect** — all-zero, or with no area (`|a·d − b·c| <
  1e-12`), the two cases that are the identity — builds the shape **at** `position`.
  The geometry record holds surface coordinates, the clip rings and gradient points
  have `position` added as the record is built, and `hasTransform` stays 0. This is the
  record an item at those surface coordinates has always had, so it keeps the native
  builder, the rectangular clip and direct antialiasing.
  [[src/codegen/builtins/canvas/helper_paint_defaults.rs:TRANSFORM_ACTIVE]]
* **A transform in effect** builds the shape at the local origin, leaves the clip and
  gradient points local, and carries all of it by the affine with the transform's linear
  part and the translation `position + (tx, ty)`.
  [[src/codegen/builtins/canvas/helper_paint_defaults.rs:PLACEMENT]]

Building every positioned shape the second way would send each one down the transformed
path to draw the same pixels. The native builder takes only items whose transform is
all-zero, so for it the offset is always `position`; it adds that to every clip point
and to every point of a `Line`, `Polygon` or `PolyLine` as it reads it, as the MFBASIC
builder does, including when it is zero.
[[src/codegen/builtins/canvas/helper_paint_defaults.rs:PLACED_PAINT]]
[[src/codegen/builtins/canvas/func_geo_build.rs:placed_point]]

For a point-list shape built at its position, the record is the one the same shape has
with `position` at the origin and the position added to each point: each point is moved
first, and the edge vectors, the bounds, the joins and the dashes are computed from the
moved points. `PolyLine`'s test for a segment with length is made on the moved points
too. So moving such a shape by its `position` and moving it by rewriting its points
draw the same frame, byte for byte, when the sums are exact.
[[src/codegen/builtins/canvas/helper_paint_defaults.rs:PLACED_PAINT]]
[[src/codegen/builtins/canvas/helper_geometry.rs:GEO_TAIL]]
[[src/codegen/builtins/canvas/func_geo_build.rs:fill_polygon]]
[[src/codegen/builtins/canvas/func_geo_build.rs:emit_polyline_scan]]
[[tests/canvas/rt_canvas_rasteriser.rs:path_placement_a_clipped_polygon_moves_whole_with_its_position]]

### A rectangle's corners

A `Rectangle` has a radius for each corner, and one value has two meanings chosen by
its size:

| Radius `r` | Corner |
|---|---|
| `r <= 0` | square |
| `0 < r <= 1` | a fraction of the half-size: an ellipse quadrant `r * width / 2` across and `r * height / 2` down |
| `r > 1` | pixels: a circular corner of radius `min(r, width / 2, height / 2)` |

The jump at 1 is intended: `1.0` is the full pill and `1.01` a one-pixel corner, so a
radius animated in pixels starts above 1. A circular corner under one pixel cannot be
written. [[src/codegen/builtins/canvas/helper_shapes.rs:CORNERS]]

How the outline is built depends only on what the four corners resolve to, and is one
of three paths:

1. **Four square corners** — the rectangle record (kind 0, radius 0).
2. **Four corners that are one circular radius** — the same record with that radius in
   slot 6: a box distance less the radius, exact on all three renderers. This is every
   rectangle that was a `RoundedRect`, byte for byte. Four equal fractional radii on a
   square rectangle are this path too, since the ellipse they describe is a circle.
3. **Anything else** — per-corner radii, or an elliptical corner. The outline is
   flattened to one clockwise ring and the item is a polygon record (kind 4), so it
   strokes, joins, dashes and clips as a polygon does, with no new geometry kind and no
   shader change. The ring starts on the top edge where it leaves the top-left corner —
   where a rounded rectangle's dashes have always started — so a dashed outline begins
   at the same place on every path. Each rounded corner is a quadrant cut into the
   fewest segments, between 4 and 32, for which a chord of a circle of the corner's
   larger radius stays within 0.05 px of the arc; the bound holds up to a radius of
   about 166 px and grows with the radius beyond it (0.3 px at 1,000). Two corners
   whose radii along a shared side sum to more than the side are scaled back together.
   The error is in local pixels: a transform or group that scales the shape scales it.
   [[src/codegen/builtins/canvas/helper_shapes.rs:RRECT_RING]]

The native builder draws paths 1 and 2 only when the four radii are equal and are
either all `<= 0` or all `> 1`; every other rectangle is built by the MFBASIC builder.
[[src/codegen/builtins/canvas/func_geo_build.rs:emit_rect_inputs]]

A `Picture` binds top-left and has no radii: it always takes path 1, which is what
keeps its sampling rectangle readable from the record's own slots.

`canvas::Size` — a rectangle's and a picture's `size`, and what `canvas::getSize`
returns — is two `Float`s. A shape's extent may be a fraction of a pixel; the
surface's and an image's are whole numbers held as Floats.

## Rendering conventions

These are the observable contract a backend must meet, not implementation notes.
The software rasteriser fixes them; the GPU backends match them within a
documented tolerance rather than exactly, because rasterisation rules and blend
precision differ legitimately between drivers.

**Which renderer draws is chosen once, at the first present.** A program in a
real window draws on the GPU (Metal on macOS, Vulkan on Linux and Windows); a headless run,
meaning any of `MFB_MACAPP_HEADLESS`, `MFB_GTKAPP_HEADLESS` or
`MFB_WINAPP_HEADLESS`, draws in software. `MFB_CANVAS_GPU` overrides both ways:
`0` forces software, any other value forces the GPU. The software rasteriser is
the exact-match oracle, and every test that renders runs headless, so the goldens
keep measuring it. A GPU frame is used only when a pipeline exists and the scene
is one the backend draws correctly; otherwise that frame falls back to software.
A machine with no Vulkan loader or driver builds no pipeline and draws in software.
[[src/codegen/builtins/canvas/helper_render.rs:ENSURE_GRAPHICS]]
[[src/codegen/runtime/canvas/vulkan.rs:has_vulkan_backend]]

**What a GPU backend declines is a frame too large for its buffers, never an ordinary
shape.** Every per-frame payload travels in buffers whose regions are fixed in size, so
each backend's predicate declines a frame that would overflow one of them. Metal and
Vulkan have the same caps: more than 65,536
item quads (a glyph is a quad of its own), more than 262,144 polygon edges summed over
the frame, more than 131,072 gradient stops summed over the frame, more than 1,048,576
dash words summed over the frame (a dashed item's dash tail plus one), or more than 8M
picture-and-glyph texels (each distinct image counted once). A `PolyLine`'s segments
count against the edge cap, one edge each, and a stroke's join records two each. There is no per-polygon
edge limit: a polygon's edges are indexed by horizontal band when the frame is built,
so a pixel tests only the edges that can reach it, and the result is identical to
testing them all. The caps are defined once, in Rust, and generated into the MFBASIC
predicates. So are the region bases the shaders read: Metal formats them into its
shader source when it compiles it, and Vulkan's checked-in SPIR-V declares them as
specialization constants that the pipeline sets from the same constants.
[[src/codegen/builtins/canvas/helper_render.rs:RENDER_METAL]]
[[src/codegen/builtins/canvas/helper_render.rs:__canvas_metalRenderable]]
[[src/codegen/builtins/canvas/helper_render.rs:__canvas_vulkanRenderable]]
[[src/codegen/runtime/canvas/mod.rs:METAL_MAX_FRAME_ITEMS]]
[[src/codegen/runtime/canvas/mod.rs:VULKAN_MAX_FRAME_ITEMS]]
[[src/codegen/runtime/canvas/mod.rs:VULKAN_SPEC_CONSTANTS]]
[[src/target/macos_aarch64/app/metal.rs:emit_band_index]]
[[src/codegen/runtime/canvas/vulkan.rs:emit_band_index]]

**A `PolyLine` on either GPU reuses the polygon's edges and the gradient's stops.** Its
segments upload through the polygon edge path and are banded the same way. The band
reach is measured from the larger of the stroke half-width and the radius slot: a
polyline is stroke-as-fill, so its half-width is in the radius slot. Its per-point colours
are its gradient stops, uploaded by the gradient path unchanged. The shader walks the
segments as the oracle does and looks the colour up at the path parameter instead of an
axis parameter. The first and last live segments travel in the block's gradient-axis
words as whole numbers rather than 16.16, because 16.16 holds an index only up to 32,767.
[[src/target/macos_aarch64/app/metal.rs:emit_edge_buffer]]
[[src/target/macos_aarch64/app/metal.rs:METAL_SHADER_BODY]]
[[src/codegen/runtime/canvas/vulkan.rs:emit_edge_upload]]
[[src/codegen/runtime/canvas/shaders/mfb_canvas.frag:polylineScan]]
[[src/codegen/runtime/canvas/mod.rs:HEADER_POLYLINE_FIRST]]

**Every shape's coverage on either GPU is computed in double-float from inputs at nearly
the oracle's precision.** fp32 is not enough: where the oracle's f64 distance lies within
~1e-5 px of a coverage quantisation boundary, fp32 rounds to the neighbouring step, and
over a dark background one step is 3-12 sRGB levels. Rounding an input to 16.16 alone
moves a distance by up to ~1.5e-5 px, so every value a distance reads travels as its
16.16 word plus a residual word, `round((x·65536 − word)·2^31)`, in units of 2^-47 px:

* each polygon or polyline edge endpoint, in a region parallel to the edge words;
* in the item block's four residual `ivec4`s: the shape parameters (centres,
  half-extents, radii, line endpoints, ellipse axes), the radius and the stroke
  half-width, an arc's two sweep endpoints, an ellipse's rotation cosine and sine, and
  the clip rectangle (a path clip's edges ride the edge region with the shape's);
* the group offset of each draw, as words 5 and 6 of its draw-list entry and the last
  two of the four 32-bit words bound to both shader stages.

The shader rebuilds each value as a double-float, a pair of floats with about 48 bits,
and evaluates the oracle's distance for every kind in it, operation for operation: the
rectangle, circle, segment (round, or butt with its two end planes), arc, ellipse,
polygon and polyline distances, the polygon's signed crossing test (multiplied out, no
division) summed under its fill rule, the inverse transform, the central-difference gradient, the stroke's
half-width and the coverage rounding; the clip's rectangle or path distance too. The
quantisation is the same `int(clamp(0.5 − d, 0, 1)·255 + 0.5)`. An arc's sweep test takes
its direction vectors from its cap endpoints less its centre, a positive multiple of the
oracle's Taylor `(cos, sin)`, so no GPU trigonometry is involved; whether the sweep is
reflex is decided by the emitter in f64 and carried as a flag, in the item-block word a
polygon uses for its fill rule instead (`arc.y`; only an arc reads it as the flag). An
ellipse runs the
oracle's 24-step bisection with every step in double-float: in fp32 a step whose test
value lies within fp32's error of zero takes the other branch. The double-float helpers
need their operations kept in written order: Metal's fast math reassociates them away,
so each MSL helper disables reassociation for its body, and the GLSL marks them
`precise`.

The double-float work is confined to pixels fp32 cannot decide, without changing the
result. For polygon and polyline edges, an error bound on each edge's fp32 distance
(2^-18 of the coordinates' magnitudes plus 2^-12 px) brackets its double-float distance.
A first fp32 pass over the pixel's band finds the least upper bracket. Only edges whose
lower bracket does not exceed it are evaluated in double-float, and only crossings too
close to call against the same bound. Every other kind has an fp32 bound of the same
kind: the rectangle, circle and line distance within the same bracket, the arc's ring
and cap distances with the sweep ignored (out of sweep the oracle answers 1e6, never
less), and for an ellipse `rmin·|s − 1|`, where `s` is the point's scale relative to the
ellipse and `rmin` the smaller radius, a true lower bound on the distance. A pixel whose
bound is past `0.5 +` the stroke half-width is 0 or 255 by its sign whatever the exact
distance, so it skips double-float. Under a transform only the centre sample may skip,
against `half + 0.5·max(F, 1)`, `F` the inverse matrix's Frobenius norm, which bounds the
gradient the coverage divides by; an arc, whose distance is not Lipschitz, never skips
under a transform. On Metal, frames rendered with every one of these skips disabled were
byte-identical to frames rendered with them, for every bug-735 and bug-687 oracle row
(bug-735, "Phase 2 results").

The emitters decide whether a blended item that fills and strokes becomes two records
on the stroke half-width's word and residual, which is exactly the header's `half > 0`
that `__canvas_blockInstances` counts on: a half-width under 2^-48 px, which would
otherwise narrow to zero, is given a residual of 1.
[[src/codegen/runtime/canvas/mod.rs:EDGE_RESIDUAL_SCALE]]
[[src/codegen/runtime/canvas/mod.rs:METAL_EDGE_RESIDUAL_BASE_WORDS]]
[[src/codegen/runtime/canvas/mod.rs:VULKAN_EDGE_RESIDUAL_BASE_WORDS]]
[[src/codegen/runtime/canvas/mod.rs:ITEM_OFFSET_SHAPE_LO]]
[[src/codegen/runtime/canvas/mod.rs:ITEM_RESIDUAL_FIELDS]]
[[src/codegen/runtime/canvas/mod.rs:ITEM_ARC_REFLEX]]
[[src/codegen/runtime/canvas/mod.rs:emit_item_polygon_rule]]
[[src/codegen/runtime/canvas/mod.rs:emit_item_stroke_sign]]
[[src/codegen/builtins/canvas/helper_render.rs:__canvas_pushOneDraw]]
[[src/target/macos_aarch64/app/metal.rs:emit_edge_buffer]]
[[src/codegen/runtime/canvas/vulkan.rs:emit_edge_upload]]
[[src/codegen/runtime/canvas/shaders/mfb_canvas.frag:shapeCoverage]]
[[src/codegen/runtime/canvas/shaders/mfb_canvas.frag:dfShapeDistance]]
[[src/codegen/runtime/canvas/shaders/mfb_canvas.frag:farBound]]
[[src/codegen/runtime/canvas/shaders/mfb_canvas.frag:edgeSlack]]

**In a real window a Metal frame goes straight to the window's `CAMetalLayer`.** The
frame is rendered offscreen, GPU-copied into the layer's next drawable and presented;
nothing is read back to the CPU. The frame is read back into a CPU surface and blitted
instead when something needs its pixels: a headless run (there is no window layer),
damage mode (`MFB_CANVAS_DAMAGE`, which keeps the previous frame), a `--debug` build
writing `MFB_CANVAS_DUMP`, or a frame with no drawable available. Software frames
always take the CPU surface, and the Metal layer is hidden while they show. The
offscreen target is created with `allowGPUOptimizedContents` off, so it is stored
linearly and the readback is a copy: a GPU-compressed target is decompressed on the
CPU inside `getBytes:`, which cost more than a millisecond per 900×640 frame.
[[src/codegen/builtins/canvas/func_metal_present.rs:lower_metal_present_scene]]
[[src/target/macos_aarch64/app/metal.rs:MTL_GPU_OPTIMIZED_CONTENTS]]

**On Windows a Vulkan frame in a real window goes to a swapchain.** The instance takes
`VK_KHR_surface` and `VK_KHR_win32_surface` and the device `VK_KHR_swapchain` when the
driver has them (and is built without them otherwise). The frame is recorded into the
acquired swapchain image through a render pass that differs from the offscreen one
only in its final layout, `PRESENT_SRC_KHR` — so every pipeline built against the
offscreen pass is compatible with it — and presented with FIFO. It takes the readback
path under the same conditions as Metal's, and also when the window's surface is not
exactly the canvas size, when no image is acquired within 100 ms, when the driver lacks
the extensions or the `B8G8R8A8_SRGB` surface format, and on every target but Windows.
Surface, swapchain, views and framebuffers are built together and rebuilt together, on a
size change, an out-of-date or suboptimal image, or a frame that acquired an image and
then could not submit it (which also replaces the two semaphores).
The swapchain presents to a hidden `STATIC` child window of the canvas window, not to the
window itself: once a swapchain presents to a window, GDI painting of it no longer
reaches the screen, so a readback frame's blit and the Console transcript would stay
covered by the last presented image. After each presented frame the graphics thread posts
`WM_APP_CANVAS_SHOW` and the window procedure shows the child, but only while still in
canvas mode; a readback frame's `WM_APP_BLIT` and every reconcile out of canvas mode hide
it. Both messages are posted in frame order, so the path the latest frame took is the
one on screen. The child answers `WM_NCHITTEST` with `HTTRANSPARENT` and never takes
focus, so input still reaches the canvas window. Entering canvas mode fits the child to
the client area and publishes that size as the canvas size, and `WM_SIZE` keeps both
current.
[[src/codegen/runtime/canvas/vulkan.rs:emit_vulkan_draw_scene]]
[[src/codegen/runtime/canvas/vulkan_present.rs:emit_ensure_swapchain]]
[[src/codegen/runtime/canvas/vulkan_present.rs:emit_submit_and_present]]
[[src/target/win_x86_64/app/mod.rs:emit_canvas_child_fit]]

**Compositing happens in linear light.** A colour's channels are sRGB-encoded
bytes, so they are decoded to linear before blending and re-encoded on store. The
transfer function is the standard one — `c / 12.92` below `0.04045`, else
`((c + 0.055) / 1.055) ^ 2.4` — evaluated over a 256-entry table so the result
cannot depend on a platform's `pow`. Blending is `dst + (src - dst) * alpha / 255`
on the linear values, rounded to nearest.

That transfer is not canvas's own: it is the shared `color::toLinear` /
`color::fromLinear` pair, and canvas blends through it (plan-122-B). A program can
therefore reach the exact seam the rasteriser uses — `color::mix` and
`color::brighten` operate in the same linear space and on the same table, so a
colour computed in a program and a pixel composited by the surface cannot disagree.
`./mfb spec stdlib color` specifies the pair; the linear range is `0`..`65535`.

Half-opaque white over red is therefore `(255, 188, 188)`, not `(255, 128, 128)`.
The second is what blending the encoded bytes directly would give, and it is the
single most common way a compositor is wrong while still looking plausible.

**Antialiasing is exact coverage.** A pixel's coverage is
`clamp(0.5 - d, 0, 1)` on the shape's signed distance `d`, sampled at the pixel
centre — the fraction of the pixel inside a locally straight edge. Coverage folds
into the source alpha rather than being applied separately, because the two mean
the same thing to the compositing equation.

The coverage form is specified rather than left to the backend because it is what
makes the software path's output *reproducible*: it uses only `+ - * /` and
`sqrt`, all exactly specified by IEEE-754, so the same scene renders to the same
bytes on every target. A `smoothstep`/`fwidth` formulation — the usual shader
idiom — depends on a derivative estimate and would not.

**The surface is opaque.** It is a window's whole content, with nothing behind it,
so alpha is written back as `255` and an unpainted pixel is black rather than
transparent.

**`Paint.blend` selects the compositing equation.** All four are defined on the
**linear** values, with `S` the source channel, `D` the destination channel and `a`
the source's coverage-folded alpha. `B` is the mode's function of `S` and `D`; the
result is then `D + (B - D) * a`, the same step every mode shares:

| `BlendMode` | `B` |
|---|---|
| `Normal` | `S` |
| `Multiply` | `S * D` |
| `Screen` | `S + D - S * D` |

`Add` is the exception and does not go through that step: it is
`min(D + S * a, 1)`. Coverage scales *how much source is added*, so a
partly-covered pixel adds proportionally less — which is both what "add the source"
means and what an additive blend computes on every GPU. Defining it instead as a
mix towards a pre-clamped `min(S + D, 1)` agrees at full coverage and differs by up
to `0.15` in linear at partial coverage over a bright destination, and no
fixed-function blend can produce that.

The destination is opaque (above), so no mode has to define what happens to a
partly-transparent destination. Alpha itself is not blended by the mode: it is
written back as `255` under every one.

**A stroke is styled by five `Paint` fields: `cap`, `join`, `miterLimit`, `dashes` and
`dashOffset`.** `canvas::fill`, `canvas::stroke` and `canvas::fillStroke` set `Butt`,
`Round`, `4.0`, `[]` and `0.0`. The cap is on `Paint` rather than on the three variants
with ends, because a dash has ends on every kind.
[[src/codegen/builtins/canvas/mod.rs:JoinStyle]]
[[src/codegen/builtins/canvas/func_fill_stroke.rs:BODY]]

**`cap` shapes a stroke's ends**: the two ends of a `Line`, an `Arc` or a `PolyLine`, and
both ends of every dash. An undashed closed shape has no ends, so its cap has no effect.
Each cap is defined on the signed distance the coverage rule already uses, so a cap edge
is antialiased exactly as any other edge is:

* `Butt` cuts the stroke square at the endpoint. A **butt line** is the round band
  intersected with the slab between the two planes through its endpoints, perpendicular
  to the segment: as a distance, `max(d − half, −t·|v|, (t−1)·|v|)` with `t` the
  unclamped projection. The half-width is subtracted *before* the intersection, because
  the planes bound the band and not the centre line. A **butt arc** is the band alone:
  the sweep already ends it along a radius.
* `Round` adds a half-disc of `half = strokeWidth / 2` centred on the endpoint. A **round
  arc** is the band unioned with a disc of `half` at each sweep endpoint, the `min` of
  the three distances.
* `Square` extends the stroke `half` past the endpoint and cuts it square there:
  `max(|perp| − half, −t·|v| − half, (t−1)·|v| − half)` against the original segment's
  `t` and `|v|`. It is not the butt planes moved out over the round band, which would
  round the square's corners. An arc's square end is a `half × half` box past each sweep
  endpoint along the tangent there. A `PolyLine` takes it on the outer ends of its first
  and last segments with length only.
  [[src/codegen/builtins/canvas/helper_shapes.rs:SEGMENT_DISTANCE_SQUARE]]

**A zero-length line** draws nothing under `Butt`, a dot of `half` under `Round` and an
axis-aligned square of side `2·half` under `Square` (SVG 2 §13.5.2). There is no
direction for a butt cap's planes to be perpendicular to, and an empty result is the
honest answer rather than an arbitrary orientation. A square end reaches `half·√2` from
its endpoint, so a Square-capped item's bounds and the GPU band index are widened by that.
The cap travels as header slot 34 (0 `Butt`, 1 `Round`, 2 `Square`), written by a `Line`,
`Arc` or `PolyLine` and by every dashed item.
[[src/codegen/builtins/canvas/helper_geometry.rs:__canvas_capTag]]
[[src/codegen/runtime/canvas/mod.rs:emit_square_cap_reach]]

**`join` shapes the corners**: every vertex where two edges of a `Polygon` ring or a
`PolyLine` meet, and a `Rectangle`'s or `Picture`'s four corners when they are square. A rectangle with
one circular radius has no corners to join; one drawn from a flattened ring (above)
is joined at every vertex of the ring.

* `Round` is the distance field's own corner: the stroke is `|d| − half` of the outline's
  distance, and the distance to a corner is round. An undashed `Round` stroke builds no
  join records, so it is exactly what a stroke was before joins existed.
* `Miter` and `Bevel` make the stroke the union of every edge's **butt** band and one
  **corner piece** per vertex. With `θ` the interior angle, a `Miter` corner is kept when
  `1 / sin(θ/2) ≤ miterLimit` — computed without a root as `(1 + u1·u2)·limit² ≥ 2` —
  and draws as `Bevel` otherwise; a `miterLimit` below 1.0 is 1.0, so every miter
  bevels.

The pieces **overlap**, and that is load-bearing. A union taken as a `min` of signed
distances is right only where some piece covers each point at least as deeply as the
union does; two pieces that merely abut both measure 0 on their shared line and draw a
half-covered seam. So each piece is built on the vertex's disc of radius `half`: straight
on, the disc; `Bevel`, the disc cut by the chord between the two bands' outer corners,
on the vertex's side; `Miter`, the disc and the polygon of the vertex, the two outer
corners and the miter point; straight back (a U-turn), nothing. Where an edge next to the
vertex is shorter than `half` the disc can reach past that edge's far end — the one place
this departs from the SVG outline, and it is the round join's own shape there.

The pieces are computed once, at geometry build, as a **join tail**: one 8-float record
per outline edge, the corner at that edge's start, as the four points `V, A, P, B`, its
kind read from exact equalities — no piece when `A = V` (an open path's first edge, a
zero-length edge, a U-turn), the disc when `A = B`, a bevel when `P = V`, a miter
otherwise. One record per edge lets the GPUs reach a pixel's corners through the band
index its edges already have. Header slot 47 counts the records and slot 48 is how far
one reaches from its vertex (`half·miterLimit` for a Miter, else `half`), which widens
the bounds and the band index. The GPUs carry the records in the edge region as two edge
slots each, after the item's own edges, and count them twice against the frame's edge
cap.
[[src/codegen/builtins/canvas/helper_joins.rs:JOIN_BUILD]]
[[src/codegen/builtins/canvas/helper_joins.rs:JOIN_DISTANCE]]
[[src/codegen/builtins/canvas/func_geo_build.rs:emit_join_record]]
[[src/codegen/runtime/canvas/mod.rs:HEADER_JOIN_COUNT]]
[[src/codegen/runtime/canvas/mod.rs:HEADER_JOIN_REACH]]

**`dashes` cuts the stroke into dashes along its outline.** It alternates on and off
lengths in pixels, before the transform; `dashOffset` is how far into the pattern the
outline starts, taken modulo the pattern's length (and may be negative).

* **Normalization.** An odd number of lengths is repeated once to make it even (SVG). An
  empty list, a list whose sum is not positive, a list holding a negative (or NaN)
  length, and a stroke that draws nothing all draw solid — SVG treats a negative entry as
  an error and draws solid, and so does this, on purpose.
* **Where `s = 0` is.** Every point of the outline has a perimeter position `s`: a `Line`
  from `startAt`; an `Arc` from `startAngle`; a `PolyLine` from `points[0]`; each
  `Polygon` ring from its own first point (each ring restarts the pattern, as SVG
  restarts it per subpath); a `Rectangle` or `Picture` from its top-left corner,
  clockwise (towards +X under Y-down); a rounded `Rectangle` from the left end of its top
  straight side, clockwise; a `Circle` or `Ellipse` from angle 0 (+X in its own frame),
  the way angles increase.
* **An outline of edges is cut edge by edge.** A pixel's position on an edge is its
  projection onto the edge's line, `u = s0 + (p − a)·d / |d|` (unclamped), and the edge's
  piece of the stroke is `max(|perp| − half, along)`, `along` being the signed distance
  along the edge from `u` to the dashes clipped to the edge's own `[s0, s1]`, negative
  inside one. The stroke is the `min` over edges, unioned with the corner piece at every
  vertex a dash runs **through** (on just before it and at it). A dashed `Round` paint
  therefore carries join records too — discs — because the round corner the distance
  field draws for free must vanish where the pattern is off. Taking `s` from the pixel's
  *nearest* outline point instead would be discontinuous on a corner's inner side, where
  the nearest edge is the second one while the first edge's band, still inside its dash,
  is what covers the pixel in SVG.
* **A curve is cut at its nearest point.** A `Circle`, `Arc`, `Ellipse` or a `Rectangle` with one circular radius
  has no corner: its `s` is the perimeter position of the pixel's nearest outline point,
  and `along` is measured in outline length there, so a dash end is cut along the
  outline's normal to within the band's curvature (`half / radius` of the antialiasing
  ramp). Angles come from a `+ − × ÷ sqrt` arctangent series (error under 1e-16) and
  points from the deterministic sine and cosine. An ellipse's arc length is a table of
  `N + 1` cumulative lengths at evenly spaced parametric angles, `N = 256`, each step
  Simpson's rule over 8 sub-steps; a pixel's `s` is interpolated linearly at its nearest
  point's parametric angle. Measured: the worst along-path error is 0.030 px over
  ellipses up to 800 px across and 80:1 (0.119 px at 128 steps).
* **Dash ends take the cap.** A **true** end — where a dash stops, not where it runs on
  past an edge's end into the next edge — is cut `Butt`, extended `half` for `Square`, or
  unioned with a disc of `half` about the end point for `Round`. An open path's own two
  ends always end a dash. A closed outline's seam is no end: a dash on at both sides of
  `s = 0` is one dash, and one on at one side only ends there.
* **Degenerate.** A dashed `Line` or `PolyLine` with no length draws the dot its cap
  draws when the pattern is on at its start; a dashed closed outline with no length
  draws nothing.

The dashes are a **dash tail** between the join records and the gradient stops:
`[n, period, c1 … cn]`, each `c` an entry's end along the period, then the kind's
perimeter data — three floats per edge (`s0`, `s1` and its ring's length, negative for
an open path), an ellipse's table, an arc's start angle and sweep, nothing for a circle
or rounded rectangle. Header slot 49 is the tail's length and slot 50 the offset wrapped
into one period at build. The record's tail order is therefore edges, join records,
dashes, a path clip's edges (see *`Paint.clip`*), gradient stops; the stops stay last,
so every reader finds the others counting back from the record's end. The native
builder writes the same tail bit for bit.
[[src/codegen/builtins/canvas/helper_dashes.rs:DASH_BUILD]]
[[src/codegen/builtins/canvas/helper_dashes.rs:DASH_DRAW]]
[[src/codegen/builtins/canvas/func_geo_build.rs:emit_dash_tail]]
[[src/codegen/runtime/canvas/mod.rs:HEADER_DASH_COUNT]]

**On the GPUs**, the item block's `strokeStyle` words (at byte 272 of its 368) are the
first join slot, the record count, the first dash word and the dash word count. Each
dashed item's words — the phase, then its dash tail — ride a frame-wide dash region as
float32 bit patterns, and each shader evaluates the oracle's dash functions in fp32 at
offsets from the item's own outline taken in double-float, so the precision is the
outline's position rather than its coordinates'.
[[src/codegen/runtime/canvas/mod.rs:ITEM_OFFSET_STROKE_STYLE]]
[[src/target/macos_aarch64/app/metal.rs:emit_dash_buffer]]
[[src/codegen/runtime/canvas/vulkan.rs:emit_dash_upload]]
[[src/codegen/runtime/canvas/shaders/mfb_canvas.frag:dashedStroke]]

**A `Polygon` is one or more closed rings filled under its `rule`.** `contours` is a
`List OF List OF vector::Float2`, each point an offset from `position` (Placement,
above); each ring closes from its own last point back to its own first.
A ring of fewer than 2 points contributes no edge and no bounds; a 2-point ring
contributes its two coincident edges, so it has no area and draws only its stroke; an
item none of whose rings contributes builds the empty header and draws nothing. Every
contributing ring's edges are one concatenated edge list, ring after ring, with no
separator and no ring count: the fill sums crossings over all of them and the stroke is a
minimum over all of them, so neither needs to know where a ring ends. The rule's ordinal
(0 `EvenOdd`, 1 `NonZero`) is header slot 21, the per-kind aux slot no other polygon
reader uses. A single-ring `EvenOdd` polygon builds the header and edge tail it built
before rings existed, slot 21 included (it was always 0).

The sign of a polygon's distance comes from a ray from the pixel centre towards −x. An
edge **crosses** when it straddles the pixel's row (`(ay > py) ≠ (by > py)`) and passes
left of the pixel. Under `EvenOdd` each crossing flips inside/outside. Under `NonZero` a
crossing edge running down the screen (`by > ay`) adds +1 and one running up adds −1,
and the pixel is inside when the sum is not zero — so a ring inside another is a hole
when the two run in opposite directions and more fill when they run the same way. The
sign convention is not observable (only zero-or-not is), but all three renderers use the
same one; both shaders sum the signed crossings as a winding number and take `EvenOdd`
as its parity. Stroked `Text` is lowered to a polygon whose header leaves slot 21 at 0,
so glyph outlines are always filled even-odd.
[[src/codegen/builtins/canvas/helper_geometry.rs:GEO_LAYOUT]]
[[src/codegen/builtins/canvas/helper_geometry.rs:GEO_TAIL]]
[[src/codegen/builtins/canvas/func_geo_build.rs:fill_polygon]]
[[src/codegen/builtins/canvas/helper_draw.rs:EDGE_DISTANCE]]
[[src/codegen/runtime/canvas/mod.rs:HEADER_POLYGON_RULE]]
[[src/codegen/runtime/canvas/shaders/mfb_canvas.frag:edgeCrossing32]]
[[src/target/macos_aarch64/app/metal.rs:edgeCrossing32]]

**A `PolyLine` is the union of its segments' round-capped bands.** Its `points` are
offsets from `position` (Placement, above). For `n` points it
strokes the `n − 1` segments `P[i] → P[i+1]`, and its distance is the minimum of the
segments' distances. A minimum of round-capped bands is what makes every interior corner
round, so the round join needs no code of its own. Each segment is measured with
`Line`'s own arithmetic, so a two-point `PolyLine` draws exactly the pixels of the `Line`
between the same points with the same cap. Under a `Miter` or `Bevel` join the inner
ends are cut butt and the corners are the join records (above); the colour parameter
still comes from the round walk, so it stays continuous round a corner.
[[src/codegen/builtins/canvas/helper_draw.rs:POLYLINE_SCAN]]

* **Ends.** The butt planes cut only the **first and last segments with length**, each
  on its outer end, with the same `max(d − half, −t·|v|, (t−1)·|v|)` a butt `Line` uses.
  A zero-length segment adds nothing, because its point is already inside its
  neighbours' round ends. So a repeated end point cannot turn a butt end round.
  The header records which segments those are, once, by the same `dx·dx + dy·dy > 0`
  test the walk uses.
* **Degenerate paths.** Fewer than two points draw nothing. A path whose points all
  coincide draws what a zero-length `Line` with its cap draws: a dot of `half` for
  `Round`, the axis-aligned square for `Square`, nothing for `Butt`.
  [[src/codegen/builtins/canvas/helper_geometry.rs:__canvas_polylineHeader]]
* **Colour.** An empty `colors` strokes the whole path in `Paint.stroke`. Otherwise
  point `k` has colour `colors[k]`, or `Paint.stroke` past the end of the list. The
  colours are stored as the record's gradient stops, one per point at offset
  `k / (n − 1)`, after its segments. A pixel whose nearest segment is `i`, at clamped
  parameter `t` along it, is drawn in the gradient colour at `(i + t) / (n − 1)`. That
  mixes the colours of the segment's two points exactly as `Paint.gradient` mixes
  two stops, in linear light with straight alpha. The segment that picks the colour is the
  one that sets the distance, because a single walk answers both. The parameter is
  continuous at a corner, so a tie between two segments picks the same colour either
  way. With colours, the path draws whatever the alpha of `Paint.stroke`; only a
  non-positive `strokeWidth` stops it.
  [[src/codegen/builtins/canvas/helper_geometry.rs:__canvas_polylineTail]]
  [[src/codegen/builtins/canvas/helper_geometry.rs:__canvas_polylineHalf]]
* **Fill is ignored**, as for `Line`: the stroke is carried in the fill slots and slot 7
  is negative. So `Paint.fill` and `Paint.gradient` never reach a polyline.

**An item's `transform` maps its coordinates onto the surface.** Every drawn item — each `DrawItem` variant but `Group` and `AutoPaint` — carries one as its last field, and each variant but `AutoPaint` an `id` as its first; `Paint` has none, and an `AutoPaint` has its shape's. The item is
drawn as though every point `(x, y)` of it stood at `(a*x + c*y + tx,
b*x + d*y + ty)`. The all-zero transform is the identity (above), so an item that
never sets one is unaffected.

Coverage is still `clamp(0.5 - d, 0, 1)`, but `d` is the distance measured **after**
the transform: a rotated edge is antialiased along where it now lies, not along
where it started. Because a distance field is transformed by transforming the query
point rather than the shape, this is specified as the shape's own distance
evaluated at the inverse-mapped point, divided by how fast that mapping stretches
distance there. That divisor is what makes the rule direction-aware, and it is
required rather than optional: under a non-uniform scale a single scalar such as
`sqrt(|det M|)` is wrong by up to 37 coverage steps of 255 at the shape's edge.

Two consequences follow, both specified rather than left to the backend:

* **A stroke is transformed with the shape it outlines.** `Paint.strokeWidth` is a
  width in the item's own coordinates, so scaling an item by 2 draws its outline
  twice as wide. The alternative — a width fixed in surface pixels — would make a
  uniformly scaled circle come out with a thinner-looking ring, which is not what
  scaling a drawing means.
* **A transform with no area is the identity.** If either row is all zero, or
  `|a*d - b*c|` is below `1e-12`, the item draws untransformed. Such a matrix
  collapses the item to a line or a point, which has no coverage under any
  antialiasing rule, so the choice is between drawing nothing and drawing the
  untransformed item; the second is specified because it is the one a program can
  see and diagnose.

A `Text` item is not a distance field — it is a run of coverage bitmaps — so it is
specified separately: each pixel of the run's transformed bounds samples the glyph
coverage at its inverse-mapped point. The result is the same rule stated for shapes,
and a rotated label is a rotated label on every backend.

**A `Picture` is a rectangle whose fill colour is its image.** Its coverage, its
antialiased edge, its stroke, `Paint.clip`, `Paint.blend`, its `transform` and a
group's translation are exactly those of a square-cornered `Rectangle` with the same
`position` and `size` —
the geometry is built as one and re-kinded. Only the fill colour differs: at each pixel
the image is sampled at the shape-space point (the inverse-mapped point under a
transform, the group-relative one inside a group), **nearest**, as texel
`(floor((px - x) * width / w), floor((py - y) * height / h))` clamped to the image. It
is nearest for the reason a transformed glyph is: `floor` and `+ - * /` only, so every
backend picks the same texel; a bilinear filter would change edge bytes between
renderers unless all three shared identical fixed-point weights.

The texel is then **tinted** by `Paint.fill`, whose alpha is the tint's strength, not
the picture's opacity. Each colour channel moves from the texel towards the product
`tint = (texel * fill) / 255` (integer, truncating) by `fill.alpha / 255`:

`out = texel + round((tint - texel) * fill.alpha / 255)`

in integers, the rounding half away from zero. An opaque fill is therefore exactly the
product (the multiply an opaque tint always was), a half-transparent red fill goes
halfway to the red-only image, and a white fill or the zero `Paint` — alpha 0 — draws
the image unchanged. The picture's alpha is the texel's, faded by `Paint.transparency`
alone: `(texel.alpha * (255 - T) + 127) / 255`, `T` the transparency step below. The
fill's alpha never fades it. `Paint.gradient` is ignored — the image, not a ramp,
fills a picture. An item whose image has been closed draws nothing (see *Images are
named, not embedded*).
[[src/codegen/builtins/canvas/helper_geometry.rs:GEO_HEADER]]
[[src/codegen/builtins/canvas/helper_items.rs:GEO_READ]]
[[src/codegen/builtins/canvas/helper_items.rs:DRAW_GEOMETRY]]

On the GPU a picture's texels travel in the same per-frame buffer region as glyph
coverage, one packed word per texel, and each item's block names its slice — so a
frame's pictures and glyphs together are bounded by that region, and a frame that
would overflow it is drawn in software instead. There is no texture object and nothing
to free: the pixels are copied from the image's own storage while the frame is built.
Both GPU backends copy each distinct image **once per frame**, however many `Picture`
items name it — a tilemap of thousands of tiles drawn from a few images costs those few
images — and their predicates count each distinct image once against the region's 8M
texels.
[[src/target/macos_aarch64/app/metal.rs:emit_picture_lookup]]
[[src/codegen/runtime/canvas/vulkan.rs:emit_picture_lookup]]

**`Paint.clip` restricts an item to the area its rings enclose.** `clip` is a
`List OF List OF vector::Float2`, each inner list a closed ring exactly as a `Polygon`'s
`contours` are, and `Paint.clipRule` decides which points the rings enclose as a
polygon's `rule` decides its fill (`EvenOdd`, the zero value, or `NonZero`). The empty
list, the zero value, is no clip. A clip that is not empty but encloses nothing — every
ring under two points, or a rectangle of no area — **hides the item**: a clip the
program wrote and that covers nothing is shown as such, never quietly dropped.

**The clip is in the item's own coordinates.** The item's `transform` maps it with the item,
and a group's translation moves it with the item, so a clipped item and its clip move,
turn and scale together. (Until plan-163-C the clip was a surface rectangle that a
transform and a group left where it was; a clip that should stay on the surface is now
written in coordinates that undo the item's transform.)

Its edges need not fall on pixel boundaries. A clipped pixel's coverage is the clip's
own `clamp(0.5 - d, 0, 1)` on its signed distance `d`, multiplied into the shape's
coverage as `(coverage * clipCoverage) / 255`. A clip edge is therefore antialiased
exactly as a shape edge is, and clipping a shape to an area larger than it changes
nothing. The builder takes one of two paths, in header slot 51:

* **Rectangle (1).** A clip of one ring of four points whose sides alternate horizontal
  and vertical, on an item with no transform, is kept as that rectangle in slots 22–25,
  and `d` is the rectangle's distance at the item's own point (the pixel centre less the
  group offset). This is a fast path, not a different rule: the same rectangle given as
  a path draws the same pixels, which the rasteriser tests pin by comparing whole
  frames.
* **Path (2).** Any other clip is a path. Its rings become edges exactly as a
  `Polygon`'s do and ride the record's tail after the dashes and before the gradient
  stops; slot 52 counts them and slot 53 is the rule (0 `EvenOdd`, 1 `NonZero`). `d`
  is the polygon distance of those edges under that rule at the item's own point. Under
  a transform it is divided by the **clip's own** gradient, by central differences at the
  same four inverse-mapped neighbours the shape uses — not the shape's gradient, which
  near the clip's edge belongs to whichever shape edge is nearest, so a sheared clip
  would be antialiased by the wrong slope. Slots 22–25 hold the clip's surface bounds,
  its ring points mapped by the transform and widened by a pixel each way, which narrow
  the pixels drawn.

[[src/codegen/builtins/canvas/helper_geometry.rs:GEO_HEADER]]
[[src/codegen/builtins/canvas/helper_items.rs:GEO_READ]]
[[src/codegen/builtins/canvas/helper_draw.rs:EDGE_DISTANCE]]
[[src/codegen/builtins/canvas/func_geo_build.rs:emit_clip_edges]]
[[src/codegen/runtime/canvas/mod.rs:HEADER_CLIP_MODE]]

**On the GPUs** the item block carries two `ivec4`s for it: `clipBand` at byte
288 is a path clip's band header, the same shape as a polygon's in `arcCaps`, and
`clipPath` at 304 is (first clip edge, clip edge count, mode | rule << 2, a picture's
`T`). The clip's edges go into the frame's edge region after the item's own and count
against the same per-frame edge cap, and they are banded by the same index builder
with no stroke in the reach — a clip's coverage clamps at `|d| ≥ 0.5`. Each shader
takes the clip's five samples at the same shape-space points as the shape's and divides
by the clip's own gradient, as the oracle does.
[[src/codegen/runtime/canvas/mod.rs:ITEM_OFFSET_CLIP_BAND]]
[[src/codegen/runtime/canvas/mod.rs:ITEM_OFFSET_CLIP_PATH]]
[[src/target/macos_aarch64/app/metal.rs:emit_clip_buffer]]
[[src/codegen/runtime/canvas/vulkan.rs:emit_clip_upload]]

**`Paint.transparency` fades the whole item.** It is clamped to `0.0`..`1.0` — a NaN
counts as `1.0` — and becomes the step `T = int(clamp(transparency, 0, 1) * 255 + 0.5)`.
At build time every alpha the item draws with becomes `(alpha * (255 - T) + 127) / 255`:
the fill, the stroke, each gradient stop and each `PolyLine` per-point colour; a `Text`
item's fill fades the same way. `T = 0` leaves every alpha as it was, so the zero value
changes nothing, and `0.5` is `T = 128`, an opaque colour's alpha `127`. The fill and
the stroke fade separately, so where they overlap the fill shows through a faded
stroke. A `Picture`'s fill alpha is its tint's strength and is not faded; its texels'
alpha is, at draw time, by `T` in header slot 54.
[[src/codegen/builtins/canvas/helper_geometry.rs:GEO_HEADER]]
[[src/codegen/runtime/canvas/mod.rs:HEADER_PICTURE_TRANSPARENCY]]

**`Paint.gradient` replaces an item's fill colour with a ramp**, and with
`Gradient.strokeApply := FillAndStroke` its stroke colour too. It replaces *colours* and
nothing else: the shape's distance, its coverage, its stroke band and its blend mode are
exactly what they would be with flat colours.

A gradient with **fewer than two stops is not a gradient** and `Paint.fill` is used
instead, which is what makes the all-zero `Gradient` inert like every other `Paint`
field. A `Text` item ignores the field: a glyph is drawn from a cached coverage bitmap,
and the cache is keyed by glyph and size, so a gradient would either be baked into the
cache or key it per colour. A `PolyLine` takes its colours from `colors` and a `Picture`
from its image.

**The ramp is in the item's own coordinates** (plan-163-D, reversing plan-116-F's
surface-anchored ramp by the user's 2026-09-28 decision). `t` is measured at the
shape-space point `q`, the point the shape's distance is evaluated at: the pixel centre
less the group's accumulated offset, mapped through the inverse of the item's `transform`.
The gradient's own `transform` `G` maps gradient space into item space first, so the
order is SVG's — `Gradient.transform`, then the item's `transform`, then the group's
translation — and a rotated or moved item carries its ramp with it. `G⁻¹` is the
inverse; the all-zero `G` and one with `|det| < 1e-12` are the identity, the
the item's `transform` rule. With `s = startPoint`, `e = endPoint` and `r = |e − s|`:

| `GradientKind` | `t` |
|---|---|
| `Linear` | `dot(G⁻¹q − s, e − s) / dot(e − s, e − s)` |
| `Radial` | `length(G⁻¹q − s) / r`, or the focal root below |

A zero-length axis or a zero radius leaves `t` at `0`, so the first stop's colour
fills the shape — defined, rather than a division by zero.

* **A linear ramp's transform is folded at build.** Its `t` is affine in `q`, `t = u·q +
  c`, which is the plain linear ramp from `−c·u/|u|²` along `u/|u|²`; the builder writes
  those two points into header slots 43–46 in f64, so every renderer evaluates an
  ordinary linear ramp. The identity is not folded — the fold would round `s` onto the
  axis line — so an untransformed gradient's record is the one it always was.
* **A radial ramp's cannot fold** (a transformed circle is an ellipse): `G⁻¹` rides header
  slots 57–62 as float32 bit patterns, the slots 27–32 convention, with its flag in 63,
  and each renderer maps `q` through it per pixel. A radial ramp scaled by different
  amounts in x and y is elliptical; without a transform it stays circular inside an
  `Ellipse`.
* **The focal circle.** With the centre `c = s`, the focus `f = c + focalOffset` and the
  focal radius `fr`, `t` is the larger root of `|p − f − t·(c − f)|² = (fr + t·(r −
  fr))²` at `p = G⁻¹q`: `(b − sqrt(b² − a·k)) / a` with `a = |c − f|² − (r − fr)²`,
  `b = (p − f)·(c − f) + fr·(r − fr)` and `k = |p − f|² − fr²`, a negative discriminant
  from rounding taken as 0. The builder clamps `fr` into `[0, r·(1 − 2⁻¹⁰)]` and the
  offset to `|focalOffset| ≤ (r − fr)·(1 − 2⁻¹⁰)`, scaling it toward the centre, so
  `a < 0` and the root always has `fr + t·(r − fr) ≥ 0`; the clamped offset and radius
  are header slots 64–66. With no offset and no focal radius it is the plain
  `length(p − c) / r`, bit for bit. SVG 2's cone for a focus outside the end circle is
  not drawn: the focus is pulled just inside it.
* **The spread** is applied to `t` before the stops, header slot 55: `Pad` leaves it for
  the clamp to `0.0`..`1.0`; `Repeat` is `t − floor(t)`; `Reflect` is `u = t −
  2·floor(t / 2)`, then `2 − u` when `u > 1`. `t` is held to ±1e9 first, where `floor`
  still fits an `Integer`.
* **The stroke.** With `FillAndStroke` (header slot 56) a stroke pixel takes the ramp
  colour `g` at `q` tinted by `Paint.stroke` — the `Picture` tint, the stroke colour in
  the fill's place: `tint = g.rgb · stroke.rgb / 255` truncating, `rgb = g + round((tint −
  g) · stroke.a / 255)` rounded half away from zero, and alpha `g.a`. So a white stroke,
  or any stroke colour at alpha 0, shows the plain ramp, and a zero-alpha stroke still
  strokes (`__canvas_strokeHalf`); the header's stroke alpha keeps the raw alpha, since
  the ramp's stops carry `transparency`'s fade. A `Line` or `Arc`, which has no
  interior, is drawn in the tinted ramp; under `FillOnly` it and every outline stay the
  flat stroke colour.
[[src/codegen/builtins/canvas/helper_geometry.rs:GEO_HEADER]]
[[src/codegen/builtins/canvas/helper_items.rs:GEO_READ]]
[[src/codegen/builtins/canvas/helper_items.rs:DRAW_GEOMETRY]]
[[src/codegen/runtime/canvas/mod.rs:HEADER_GRADIENT_SPREAD]]

**On the GPUs** `surface.w` packs the kind with the spread and the stroke apply, `kind |
spread << 1 | apply << 3`, and three `ivec4`s grow the item block to 368 bytes:
`gradInv0`/`gradInv1` at 320/336 carry `G⁻¹` and its flag word for word, and
`gradFocal` at 352 the focal offset and radius in 16.16. The shaders take `t` in fp32 at
the fp32 inverse-mapped `q`, the same operations in the same order.
[[src/codegen/runtime/canvas/mod.rs:ITEM_SURFACE_GRADIENT_KIND]]
[[src/codegen/runtime/canvas/mod.rs:ITEM_OFFSET_GRADIENT_INV]]
[[src/codegen/runtime/canvas/mod.rs:ITEM_OFFSET_GRADIENT_FOCAL]]

**Stops are used in the order given.** They are not sorted. An `offset` outside
`0.0`..`1.0`, or one that goes backwards, is clamped to the previous stop's offset
instead — so a stop given out of order produces a hard edge there, which is visible and
diagnosable, where sorting would silently draw a picture the program did not describe.
Before the first stop and after the last, that end stop's colour holds; neither end
extrapolates.

**A gradient inside a `canvas::Group` moves with the group**, as everything in the
item's own coordinates does: the group's accumulated translation is subtracted from the
point before `t` is computed, so the same group referenced at two offsets shows the same
colours at corresponding points within it.

**A gradient interpolates in linear light**, the same space compositing uses, through
the same 256-entry table. Between the two stops bracketing `t`, each channel is decoded
to linear, mixed, and re-encoded. A black-to-white ramp is therefore evenly bright
across its width; mixing the encoded bytes directly would make it dark for most of its
length, which is the same error as blending encoded bytes and is just as plausible-
looking.

**Frame statistics are two event logs, read in one critical section.**
`canvas::getFrameStats` reports two different counts, and they are written by
different threads:

- **Presents.** Every `present` and `presentLayers` call is logged by the worker,
  including an identical re-present that publishes nothing.
- **Drawn frames.** A frame a backend actually drew is logged by the graphics
  thread when the frame completes. A wake-up whose damage is empty is not logged.
  Nor is anything logged for a resize or an expose that redraws nothing.

Each log is a running total, plus a ring of the last 64 monotonic timestamps with
its head. All of it lives in the process-global graphics state, since the worker
cannot see the graphics thread's globals. It is written and read only under the
graphics mutex, which the frame-completion path already takes, so the drawn log adds
no lock. `canvas::frameCounters` copies both logs, the last frame's render time and
renderer, and the display rate in one critical section. Before the graphics thread
starts it copies without the lock: at that point nothing has written a word.
[[src/codegen/runtime/canvas/mod.rs:emit_note_present]]
[[src/codegen/runtime/canvas/mod.rs:emit_frame_done]]
[[src/codegen/runtime/canvas/mod.rs:emit_frame_counters]]

A rate is over the trailing second, counted from one log's ring:

- **The ring has not saturated.** If fewer than 64 held entries are newer than
  `now - 1 s`, the rate is exactly that count.
- **The ring has saturated.** If all 64 are newer, the ring spans less than a
  second, and the rate is `63 / (newest - oldest)`. A loop that presents thousands
  of times a second is then measured, not reported as 64.

Either way the rate is 0 one second after the last event. `dt` is the gap between
the two newest present timestamps, and 0 before the second present. `renderer` is
the backend that returned the drawn frame: `Software` (0), `Metal` (1, including a
frame presented straight to the window's layer) or `Vulkan` (2, including a frame
presented to the window's swapchain). It reads
`Software` before the first frame.
[[src/codegen/builtins/canvas/func_get_frame_stats.rs:body]]
[[src/codegen/builtins/canvas/helper_render.rs:RENDER_LOOP]]

Both wrappers call `__canvas_ensureGraphics` and then log the present
**unconditionally**, ahead of their publish test. So the first present of any scene
starts the graphics thread, even one identical to the empty initial scene.
[[src/codegen/builtins/canvas/func_present.rs:BODY]]

**`present` never waits; `waitFrame` is the pacing call.** `canvas::waitFrame` returns
once both of these hold:

1. **Nothing is owed.** The render loop has picked up every redraw asked for so far —
   the redraw flag is clear.
2. **The floor has passed.** At least one display interval (`1 / displayHz`, or 1/60 s
   when the rate is unknown) has elapsed since `waitFrame` last returned.

It also returns at once when the graphics thread is stopping. The floor holds on every
backend, because none of them waits for the display: without it, an unchanged scene
(nothing is ever owed) or a cheap one would spin.

It waits for the redraw to be *picked up*, not *drawn*. Waiting until the frame has
finished would serialize the threads: the worker would idle while the frame renders,
and the renderer would idle while the worker builds the next scene. Pickup leaves one
frame in flight.

`waitFrame` waits on a condition variable of its own, broadcast when a redraw is taken
and on stop. The render loop's condition is woken by `pthread_cond_signal`, which wakes
one waiter, so sharing it could hand a render-loop wake-up to the worker.

The floor's deadline is kept in the clock the condition variable's timed wait uses,
`CLOCK_REALTIME`. It is never later than `now + interval`, so a wall-clock step costs
at most one interval. The next interval is measured from the deadline when the return
is less than one interval late, so wake-up latency does not accumulate; otherwise it is
measured from the return, so a loop that fell behind gets no burst of catch-up returns.
[[src/codegen/runtime/canvas/mod.rs:emit_await_pickup]]
[[src/codegen/runtime/canvas/mod.rs:emit_wait_for_redraw]]

## Named groups

`canvas::setGroup(name, items)` installs a list of items under a name;
`canvas::removeGroup(name)` drops the name. A scene draws an installed group by
including a `canvas::Group` node — `id`, `position`, `transform` and the name — in place
of the items:

```basic
canvas::setGroup("panel", panelItems)
canvas::present([canvas::Group[id := "", position := vector::Float2[x := 10.0, y := 20.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"]])
```

**The node's `id` names the USE, not the group.** It is `id` first, as on every other
item, and it draws nothing: it is not folded into the draw entries' hashes, and no
geometry reads it. It is one part of the id path `canvas::MouseEvent.overID` reports
(*Hit-testing*, below), which is how one group drawn through two nodes is told apart.
`canvas::present` compares the scene's bytes, which include it, so a node whose `id`
changes is republished.

**A scene copies the node, not the group.** `canvas::present` copies the whole scene
every time it is called, so a large static sub-picture referenced from many scenes — or
from several positions in one — is copied once when it is installed rather than once per
frame per reference.

**`canvas::present` is the install point.** `setGroup` does not repaint. Presenting the
same scene list after a `setGroup` *does* repaint, even though the list is unchanged: the
change is inside the group and it is seen. This is the same rule everything else here
follows, and it is worth stating because the natural expectation is the opposite.

**A name that is not installed draws nothing and does not raise**, whether it was never
installed or has since been removed. A scene may therefore reference a group before it is
built, and removing a group does not invalidate any scene that names it.

**A group node is an affine: `T(position) ∘ transform`.** The group's own origin lands
at `position`, and `transform` turns, scales or shears what the group holds about that
point, its `tx` / `ty` adding to `position` — the placement rule an item follows
(Placement, above), with the group's items in the place of the item's geometry. A point
`q` of an item's geometry, already through the item's own placement `I`, reaches the
surface at

    G₁(G₂(… Gₙ(I(q)) …))

where `G₁` is the outermost node. So the item is placed in its group's coordinates
first and the groups carry it from the inside out.

A node's transform that is **not in effect** — all-zero, or with no area — makes the
node a translation by `position`, by the same test an item's uses. A scene whose nodes
are all like that is drawn exactly as a scene of `dx` / `dy` nodes was: the composed
translation is the same sum, and nothing below changes for it.
[[src/codegen/builtins/canvas/helper_render.rs:RENDER_METAL_TEMPLATE]]
[[src/codegen/builtins/canvas/helper_paint_defaults.rs:TRANSFORM_ACTIVE]]
[[tests/canvas/rt_canvas_rasteriser.rs:group_affine_a_nested_group_composes_with_its_parent]]

Everything an item has travels with its group, because all of it is in the item's own
coordinates:

* **`Paint.clip`** — a group places the clipped item and its clip together, and turns
  them together.
* **`Paint.gradient`** — the ramp is evaluated at the item's own point. A group exists to
  be drawn somewhere else, and an item whose colours depended on where its group was
  placed would not be reusable.
* **The stroke** — its width is in the item's space, so a group scaled 2× draws a 4-pixel
  stroke 8 pixels wide, and an uneven scale draws it uneven.
  [[tests/canvas/rt_canvas_rasteriser.rs:group_affine_a_scaled_group_scales_its_strokes]]
* **Text** — a run inside a group with a matrix is sampled as a transformed run is:
  nearest texel of the untransformed glyph bitmap, at the inverse-mapped point.

**How the composed affine is carried.** Both walks of the group tree — the software and
damage walk and the GPU draw-list walk — compose a node with its parents through one
function, so they cannot disagree about what a nested group means. The composition is
kept as a translation and, only when some node on the path has a transform in effect, a
2×2 linear part with its inverse:

* each draw entry of the software walk has its translation in `__CANVAS_DRAW_DX` /
  `_DY`, as before, and a matrix number in `__CANVAS_DRAW_MAT` — 0 for none, otherwise a
  row of `__CANVAS_GROUP_ROWS` (forward `a b c d`, then the inverse) plus one;
* the rasteriser subtracts the translation from the surface point, as it always has,
  then applies the inverse linear part, then the item's own inverse if it has one. An
  entry with a matrix takes the transformed-item path whatever the item is, so the
  distance is rescaled by its gradient and the stroke scales
  ([[src/codegen/builtins/canvas/helper_items.rs:GEO_READ]]);
* an item's bounds under a matrix are the hull of its header box through the linear
  part, padded by a pixel — the header's own antialiasing rim is in the group's space
  and a group that scales down would shrink it. The damage diff remembers and compares
  that hull ([[src/codegen/builtins/canvas/helper_damage.rs:DAMAGE_FOR]]);
* a rectangle clip, which is axis-aligned only in the item's space, is evaluated under a
  matrix as a path clip is: its distance at the item's point, divided by its own
  gradient;
* a draw entry's hash folds in the linear part when it has one, so a group turned in
  place — no geometry and no translation changed — is repainted.
  [[tests/canvas/rt_canvas_damage.rs:turning_a_group_repaints_and_matches_a_full_redraw]]

A composed linear part with no area (`|det| < 1e-12`, reached only by scaling something
out of existence through several nodes) draws nothing: the node and everything under it
is skipped.

**A GPU draw entry is sixteen 64-bit words.** Base and count, the translation as two
16.16 words, the blend mode and the translation's two residual words are words 0–6, as
they were when the entry was eight words. Word 7 is 1 when the entry has a matrix, words
8–11 are the inverse linear part as float32 bit patterns, and words 12–15 are zero. A
translate-only entry is therefore its old eight words followed by zeros.
[[src/codegen/runtime/canvas/mod.rs:CANVAS_DRAW_ENTRY_WORDS]]

**Both GPU backends bind 32 bytes per draw, to both stages.** The entry's four
translation words, then the four words of the inverse linear part: `MfbDraw` on Metal,
sent with `setVertexBytes:` and `setFragmentBytes:`, and one push-constant range on
Vulkan that both stages declare. `hasMatrix` is not sent — an inverse is never all
zero, so all-zero bits mean the groups only translate, and the shaders then take the
branch they took before a group could have a matrix.
[[src/target/macos_aarch64/app/metal.rs:emit_draw_list_pass]]
[[src/codegen/runtime/canvas/vulkan.rs:emit_draw_list_pass]]
[[src/codegen/runtime/canvas/vulkan.rs:PUSH_OFFSET_BYTES]]

With a matrix:

* the **vertex** stage carries the item's box through the forward linear part — the
  inverse of the 2×2 it was given, taken in the shader — so the quad is a parallelogram.
  The box is first grown on every side by the inverse's Frobenius norm, which is at
  least one surface pixel, because the box's own antialiasing rim is in the group's
  space and a group that scales down would shrink it;
* the **fragment** stage subtracts the translation from the pixel, as it always has,
  then applies the inverse linear part, and only then the item's own inverse if it has
  one. Every consumer reads that point: the distance (whose four neighbour samples are
  half a surface pixel apart, so the inverse is applied to those offsets too), the
  gradient ramp, the glyph texel, the picture texel and the clip. A rectangle clip is
  sampled like a path clip there, for the reason the software renderer does it.

Bound to the vertex stage alone, a turned group would have the right outline and the
wrong contents — the reasoning that puts the offset in both stages.
[[tests/canvas/rt_canvas_golden.rs:the_gpu_draws_the_group_affine_scene_the_reference_shows]]
[[tests/canvas/rt_canvas_metal.rs:a_group_with_a_matrix_draws_on_metal_and_matches_the_software_oracle]]

**Groups may nest, to a depth of 64.** A nested `canvas::Group` stays a reference: it is
resolved when the scene is presented, so replacing the inner group changes what the outer
one draws without reinstalling the outer. Two groups may name the same third one, and it
is drawn once per reference, through each reference's own position and transform. Exceeding the depth raises
`ErrDepthExceeded` from `canvas::present`, and so does a group that eventually references
itself — a cycle is unbounded depth, and the two need the same fix.

**A group stays installed until you replace it or remove it.** Installing again under the
same name replaces what that name draws. The items are copied, so the list passed to
`setGroup` is the caller's to change afterwards; the installed group does not follow it.

## Hit-testing

One question — which drawn item is topmost under a surface point — answered by the
rasteriser's own distance functions, read from the same geometry records. Its first
user is `canvas::MouseEvent.overID`; its second is `canvas::AutoPaint`. It is two
MFBASIC chunks. The region test — is a point on a draw entry —
[[src/codegen/builtins/canvas/helper_hit.rs:REGION]], is injected into a program that
calls `canvas::pollMouse` or constructs an `AutoPaint`
(`HelperGate::WhenUsedOrConstructed`), and is the one function both users call. The
worker's draw list, the id paths and `pollMouse` itself,
[[src/codegen/builtins/canvas/helper_hit.rs:BODY]], are injected only into a program
that calls `canvas::pollMouse` (`HelperGate::WhenUsed`).

### The region of an item

`__canvas_hitEntry(offset, mat, gdx, gdy, x, y)` asks whether surface point `(x, y)` is on
the record at `offset`, drawn under group matrix `mat` at group translation
`(gdx, gdy)`. It makes the entry's matrix current (`__canvas_enterGroupMatrix`) and
`__canvas_hitCurrent` then does, in order:

1. **Bounds.** Reject outside the entry's surface box (`__canvas_drawBound`) widened by
   3 px — except for a record that is hit as a text box (`__canvas_hitIsTextBox`), which
   skips this step: a record's bounds are what it draws, and a stroked run's are its
   outlines', tighter than the layout box.
2. **The item's own point.** `(x − gdx, y − gdy)`, then `__canvas_invX` / `__canvas_invY`
   when `__canvas_hasTransform` — the groups' inverse linear part and the item's own
   inverse, which is exactly the rasteriser's `qx`, `qy`.
3. **The clip.** A path clip (`__CANVAS_GEO_CLIP_MODE` = 2) rejects where
   `__canvas_clipPathDistance > 0`; a rectangle clip where `__canvas_clipRectDistance >
   0`. Both are evaluated at the item's own point, so a clip under a transform needs no
   rescale: the sign of a distance does not change with scale.
4. **The shape**, by kind:

| Kind | Hit when |
|---|---|
| `RECT`, `CIRCLE`, `ELLIPSE`, `POLYGON`, `PICTURE` | `d ≤ max(strokeHalf, 0)`, or the record has join records and `__canvas_joinedStroke ≤ 0` |
| `SEGMENT`, `ARC`, `POLYLINE` | `d ≤ 0`, or `d ≤ 3·scale − halfWidth` |
| `TEXT`, and a `POLYGON` carrying the text-box mark | the item's own point is inside its layout box |
| `NONE` | never |

`d` is `__canvas_hitDistance`: `__canvas_geoDistance` with the header's per-item
constants and `dashed` FALSE, so a dashed stroke is hit as the solid one it is cut from.

**A closed kind is its interior and its stroke, painted or not.** The stroke's band is
`|d| − strokeHalf`, so interior and band together are `d ≤ strokeHalf`; a paint that
draws nothing changes neither. A rectangle with the zero paint is a record
(`__canvas_rectHeader` declines only a non-positive size), so it is hit and draws no
pixel.

**An open kind is its stroke, at least 3 px either side of its centre line.** Its `d`
is already the distance to the band's edge (the half-width, header slot 6, taken off),
so the slack is `3·scale − halfWidth`, which is negative — no slack — once the stroke is
6 px wide. `scale` is shape units per surface pixel: 1 untransformed, otherwise the
length of `d`'s gradient by central differences at ±0.5 px, the quantity
`__canvas_drawGeometry` divides a transformed distance by. The 3 px are therefore
surface pixels under any transform. An open kind whose paint has no stroke builds no
record (`__canvas_segmentHeader`, `__canvas_arcHeader`, `__canvas_polylineHeader`) and
is not hit.

**A `Text` is its layout box**: from the baseline origin to where the pen stops, the
font's ascent above the baseline and its descent below. The box is four floats in
`__CANVAS_GEO_TEXT_BOX` — the gradient's four point slots, which no `TEXT` record uses —
and the two threads come by it differently:

* **The worker's record for a `Text` is not the renderer's.** `__canvas_textBoxHeader`
  builds a `TEXT` record that holds the paint (for the transform and the clip), the box,
  and no glyph: the run is shaped for its advances and never rasterised. A stroked run
  is the same record.
* **The graphics thread's is the renderer's, with the box written into it** — only in a
  program that constructs an `AutoPaint`, by `__canvas_autoTextBox` at the end of
  `__canvas_geometryFor`'s `Text` arm. A filled run is a `TEXT` glyph-run record. A
  stroked run is a `POLYGON` of its outlines, a kind whose gradient slots may be in
  use, so that record also carries `__CANVAS_GEO_TEXT_BOX_MARK` (2) in the
  gradient-kind slot, which a gradient writes 0 or 1 into; the mark is honoured only
  with fewer than two stops, where nothing else reads the slot or the points, and a
  stroked run never has more (`__canvas_textHeader` clears the count).
  [[src/codegen/builtins/canvas/helper_autopaint.rs:RESOLVE]]

In a program with no `AutoPaint` the renderer's text records carry no box and nothing on
the graphics thread asks for one.

### The walk, and the id path

`__canvas_hitAt(x, y)` walks the calling thread's draw list from the last entry to the
first and answers the first entry `__canvas_hitEntry` accepts, or −1. The list is
`__CANVAS_HIT_OFFSETS`, parallel to `__CANVAS_DRAW_DX` / `_DY` / `_MAT` and
`__CANVAS_HIT_IDS`. Every entry is tested, named or not: an unnamed item lying on a
named one answers, and its path is `""`.

`__canvas_hitAppend` builds the list. It is `__canvas_appendDraw`'s walk — the same
depth stop, the same `__canvas_composeGroup`, the same skip of a composed affine with no
area — with one more thing carried down: the path. A group node passes its children
`__canvas_joinId(path, g.id)`; a leaf records `__canvas_joinId(path, id)`.
`__canvas_joinId` joins with `::` and leaves an empty part out. So:

| Item | Path |
|---|---|
| `label`, in the scene itself | `label` |
| `label`, in a group drawn by node `save` | `save::label` |
| `label`, in a group drawn by node `save`, in a group drawn by node `toolbar` | `toolbar::save::label` |
| an item with no `id`, in a group drawn by node `save` | `save` |
| `label`, in a group drawn by a node with no `id` | `label` |
| nothing on the path has an `id` | `""` |

The path belongs to the draw ENTRY. A group's stored items are not renamed, and one item
reached through two nodes is two entries with two paths. Two nodes with the same `id`,
or with none, give equal paths. The path is one string; nothing parses it back, and an
`id` that contains `::` is reported as written.

It is a second walk rather than a parameter on `__canvas_appendDraw` because that
function's leaf does what a hit list must not: it builds the renderer's record
(rasterising every glyph of a `Text`), folds the draw hash, and its output is re-laid
natively by `canvas::sceneLayout`, which moves one word per entry per list and knows no
list of strings.

`__canvas_hitGeometry` chooses the record. Every kind but two is the renderer's
(`__canvas_geometryFor`). A `Text` is `__canvas_textBoxHeader`, above, and a `Picture`
is its rectangle (`__canvas_rectHeader`), without the image's pixel block.

### `canvas::pollMouse`

`canvas::pollMouse` is MFBASIC, `__canvas_pollMouse`, over the native
`canvas::pollMouseRaw` ([[src/codegen/builtins/canvas/func_poll_mouse.rs:lower_poll_mouse]]),
which dequeues one event into a whole `MouseEvent` block whose `overID` is the empty
string: 88 bytes — seven slots, `position`'s two floats at 56 and the empty string's
sub-block at 72. The wrapper hands that block straight back when the kind is `None` or
`__CANVAS_SCENE_IDS` is 0, which is nearly every poll there is. Otherwise it asks
`__canvas_overId` at the CENTRE of the event's pixel, `position + 0.5`, the point the
rasteriser evaluates, and answers `WITH event { overID := … }` only when the path is
not empty.

**`overID` is resolved at the poll, on the worker, against the scene the worker last
presented** — not at the moment the event happened, and not from a frame. The worker
builds its own draw list from `canvas::installedItems()` and `installedLayers()` into
its own thread-local geometry cache (`.ai/canvas-threading.md` section 2); nothing is
read from the graphics thread. `__canvas_overId` decides how much of that to do:

| Condition | Cost |
|---|---|
| `__CANVAS_SCENE_IDS` = 0 | none: `""` |
| `__CANVAS_HIT_REV` = `__CANVAS_SCENE_REV` | the walk only |
| otherwise | `__canvas_hitBuild`, then the walk |

`canvas::present` and `canvas::presentLayers` record the two facts, on a present that
changed the scene or a group it names: `__CANVAS_SCENE_REV`, a count of such presents,
and `__CANVAS_SCENE_IDS`, `canvas::sceneIdBits` of the scene
([[src/codegen/builtins/canvas/func_item_hash.rs:lower_scene_id_bits]]) — bit 0 when an
item of the scene's own lists has a non-empty `id`, a group node's included; bit 1 when
one of them is a group node or an `AutoPaint`. The native pass reads each item's first
field, which is `id AS String` on every variant but `AutoPaint` and is checked against
the type model at build time; an `AutoPaint` has no `id` of its own and reports its
shape's, which is behind a union field this pass does not open, so it is counted as
"may be named" with the group nodes.
It does not look inside a group, so a value of 2 — group nodes, none of them named —
makes `__canvas_hitBuild` ask `__canvas_reachesId` before it builds any geometry. A
list in which no entry has a path (`__CANVAS_HIT_NAMED` FALSE) is never walked.

A `setGroup` is seen at the next `present`, as it is by the renderer: the group
signature moves, `present` counts a revision, and the list is rebuilt.

The mouse-state reservation keys on the raw poll's symbol
([[src/target/shared/plan/symbols.rs:module_uses_mouse]]), and the chunk that calls it
is gated on use, so a canvas program that never calls `canvas::pollMouse` reserves no
mouse state and compiles none of this.

## The pointer, as the graphics thread sees it

The graphics thread can read where the pointer is without the worker's help: each
platform's MAIN thread publishes it into the graphics-state block, whether or not the
program called `canvas::enableMouse`. Nothing here reaches a program — `canvas::pollMouse`
and the SGR pipe behind it are a separate path. A pointer change wakes the render loop
only while the scene it last drew holds an `AutoPaint` (below, "AutoPaint"); otherwise
it signals nothing.

**Five words**, 40 contiguous bytes
([[src/codegen/runtime/canvas/mod.rs:GRAPHICS_OFFSET_POINTER_SEQ]]):

| Word | Holds |
|---|---|
| `POINTER_SEQ` | the sequence word: odd while a publish is writing the other four |
| `POINTER_X`, `POINTER_Y` | surface pixels, the units an SGR mouse report carries; meaningless while `POINTER_INSIDE` is 0 |
| `POINTER_BUTTONS` | bit 0 left, bit 1 middle, bit 2 right, set while held |
| `POINTER_INSIDE` | 1 while the pointer is over the surface |

**The four are written and read as one, under the sequence word, with no lock.**
[[src/codegen/runtime/canvas/mod.rs:emit_publish_pointer]] increments `POINTER_SEQ` to
odd, stores the four, and increments it to even; it makes no call, so it can sit in any
platform handler. `canvas::pointerSnapshot`
([[src/codegen/builtins/canvas/func_pointer.rs:lower_pointer_snapshot]]) reads the
sequence word, the four, and the sequence word again, and retries until the two reads
are equal and even. A lock would make a pointer move wait behind a frame. On AArch64
every store is a store-release and every read a load-acquire, so no data word can be
seen before the odd mark nor the even mark before the data; x86-64 keeps stores in
order and loads in order by itself. One writer at a time is the contract: the main
thread in a window, the graphics thread's own script in a headless `--debug` run.

**A point off the surface is published as outside, not dropped.** A report is dropped
there — there is nothing to report against — but a drag that leaves the surface keeps
arriving with the button held and no leave event, and the pointer words are how the
graphics thread learns it has gone.

**macOS.** `MFBCanvasView`'s mouse methods publish
([[src/target/macos_aarch64/app/mouse_view.rs:emit_publish]]), and a canvas program has
them whether or not it uses a mouse member: thirteen IMPs, the eleven that report plus
`mouseEntered:` and `mouseExited:`, which only publish. Each publishes FIRST and gates
on the mouse-mode word after; in a program with no mouse member there is no mode word,
and the IMP is the publish and a return (`emit_mouse_imp`'s `reports`). The buttons are
`+[NSEvent pressedMouseButtons]` with bits 1 and 2 exchanged (AppKit orders them left,
right, other) and then corrected by the event itself — a press or a drag sets its own
button's bit, a release clears it — because that class method is the hardware's state
NOW, and a quick click is released before its `mouseDown:` is dispatched. The canvas
view's tracking area asks for `NSTrackingMouseEnteredAndExited`; `TermView`'s does not,
and `TermView` publishes nothing.

**GTK.** The canvas drawing area carries two controllers of its own, attached where
the area is created ([[src/target/linux_gtk/pointer.rs:emit_pointer_attach]]): a motion
controller, whose `motion` and `enter` publish a position and whose `leave` publishes
outside, and a click gesture told to listen to every button. They are separate from the
reporting controllers on the window, which they neither replace nor starve: events
bubble from the area to the window, and nothing here claims a sequence. On the area the
coordinates GTK hands over are already the surface's, and the controllers are quiet
whenever the canvas is not the window's child. The held buttons are NOT read from GDK's
event state: its Wayland backend sets the state bit for a held right or middle button
by evdev order while numbering a press's `button` left, middle, right — so the two bits
mean the opposite of `GDK_BUTTON2_MASK` / `GDK_BUTTON3_MASK`, measured in a real
window. The mask is kept in `_mfb_gtkapp_pointer_held` from the button NUMBER each
press and release carries ([[src/target/linux_gtk/pointer.rs:emit_pointer_handler]]),
and a motion whose state shows no button held at all clears it.

**Windows.** The window procedure's dispatch chain ends, in a canvas program, at a
block that publishes and passes the message on
([[src/target/win_x86_64/app/pointer.rs:emit_pointer_arms]]): for `WM_MOUSEMOVE`
through `WM_MBUTTONDBLCLK`, while a canvas surface is live, it calls
`_mfb_winapp_pointer_move(x, y, MK flags)` with the signed client point from `lParam`,
and arms one `WM_MOUSELEAVE` with `TrackMouseEvent` unless one is armed; for
`WM_MOUSELEAVE` it calls `_mfb_winapp_pointer_leave`, which publishes outside with the
buttons last published. `wParam`'s `MK_*` flags are the state after the event, so the
mask needs no correction. The SGR arms run after it, unchanged.

**The headless variables.** `MFB_CANVAS_POINTER_X` / `_Y`, on GTK and Windows, call the
platform's own callback once with that position after the first completed frame — GTK's
motion callback with no controller, Windows' `_mfb_winapp_pointer_move` with no flags —
on the scripted-resize precedent, and after the scripted resize: inside is asked of the
PUBLISHED surface size, which a headless run has none of until `MFB_CANVAS_RESIZE_W` /
`_H` publish one. macOS has no headless path; its handlers are proved in a real window.

**A `--debug` build can script the pointer.** `MFB_CANVAS_POINTER` holds
`ms,x,y,buttons,inside;…`; at the start of each frame
[[src/codegen/builtins/canvas/helper_render.rs:POINTER_SCRIPT_DEBUG]] publishes every
entry whose time has come, counted from the first frame, through
`canvas::publishPointer` — the same writer. A normal build's source has neither the
reader nor a call to the writer. The stats line reports the snapshot as
`pointer=x,y,buttons,inside`.

## AutoPaint

A `canvas::AutoPaint` is a `Shape` and up to four alternative paints. Which one is
drawn is decided on the GRAPHICS thread, each frame, from the pointer words above — the
worker never resolves it and `present` never waits on it, so a shape goes on reacting
while the program is blocked in a key read. Everything below but the type and its
`MATCH` arms is one MFBASIC chunk, [[src/codegen/builtins/canvas/helper_autopaint.rs:RESOLVE]]
with [[src/codegen/builtins/canvas/helper_autopaint.rs:AUTO_STEP]], injected only into a
program that constructs an `AutoPaint`. Every other canvas program gets
[[src/codegen/builtins/canvas/helper_autopaint.rs:ABSENT]] in its place: the same twelve
function names, with the bodies that are right when no `AutoPaint` can exist
(`HelperGate::WhenNotConstructed`). A record value exists only where a constructor
expression made it, so a program that constructs none holds none, and the gate is
decided from the program's constructor expressions.

### The state of an instance

An instance is kept in one of four phases between frames, with one more bit:

| Phase | Meaning |
|---|---|
| 0 `OUT` | it is not the topmost item under the pointer |
| 1 `IN` | it is, and any enter window is over |
| 2 `ENTER` | it became so less than a window ago |
| 3 `EXIT` | it stopped being so less than a window ago |

The bit is whether the instance **owns the press**: set when the first button goes down
(buttons were 0 at the previous resolve and are not now) while it is the topmost item
under the pointer, cleared when every button is up. Any button is a press.

`__canvas_autoStep` advances one instance by one frame. In order:

1. A window that has run out ends: `ENTER` → `IN`, `EXIT` → `OUT`, when the clock has
   reached the instance's deadline. So does a window whose flag is now off, which a
   re-`present` of the item with different flags can leave behind.
2. Under the pointer, `OUT` or `EXIT` becomes `ENTER` and a new window starts — or `IN`
   at once, with no window, when `useEnter` is off. Not under it, `IN` or `ENTER`
   becomes `EXIT` and a new window starts — or `OUT` at once when `useExit` is off.
   Leaving during an enter window and returning during an exit window each START a
   window: the one in progress is abandoned, not resumed.
3. The paint shown is the first that applies of: `pressedPaint` (under the pointer,
   owns the press, `usePressed`), `enterPaint` (`ENTER`), `hoverPaint` (`IN`,
   `useHover`), `exitPaint` (`EXIT`), the shape's own.

A phase is never one its flag forbids, which is the whole of "an unused state falls
through": an unused flash starts no window and so asks for no frame when it would have
ended.

The window is the project's `options.canvasAutoPaintMS`, 100 ms unless project.json
says otherwise (`./mfb spec tooling project-manifest`), on `canvas::frameNanos`'
monotonic clock. It is one value for the program, not a field of the record: the build
bakes it into the graphics-state block
([[src/codegen/runtime/canvas/mod.rs:GRAPHICS_OFFSET_AUTO_WINDOW_MS]]) and
`__canvas_autoWindow` reads it through `canvas::autoWindowMs`
([[src/codegen/builtins/canvas/func_pointer.rs:lower_auto_window_ms]]). Nothing is
interpolated: each paint is drawn whole.

**The pointer is sampled, not replayed.** A frame reads the pointer words once. Two
changes between two frames are one change to it, so a press and release that both land
inside one frame are not seen, and a button that goes down off a shape and reaches it
before the next frame is a press that began on it. Every publish wakes the render loop
while an `AutoPaint` is live, so the window for this is one frame.

### An instance is a place in the scene

State is kept per instance, and an instance is identified by where the walk found it,
never by an `id` or by its contents. The key is a hash of the path: the index of the
top-level item in the frame's scene (its flat items, then each layer's), and for an
item inside a group, the chain of (index of the group node, slot the node resolved to)
down to the index of the item in its group (`__canvas_autoPath`, `__canvas_autoKey`).

| Change | Effect on the state |
|---|---|
| the item is re-presented in the same place — moved, restyled, its flags changed, or unchanged | kept, deadline included |
| one group is drawn by two nodes | two instances, whatever the nodes' `id`s are |
| an item is inserted or removed ahead of it in the scene, or in its group | a new key: it starts from `OUT` |
| the instance is not in a frame | its row is dropped; returning, it starts from `OUT` |

The table is five parallel lists — key, phase and ownership, deadline, paint shown,
parent path — rebuilt each frame from the instances that frame saw.

### One frame

`__canvas_appendDraw`'s `AutoPaint` arm calls `__canvas_autoAppend`, which notes the
instance and appends its shape's draw entry built from the shape's OWN paint. That
entry's geometry is cached under the item's hash stepped by 16 (`__canvas_autoGeoHash`),
never under the item's hash itself: the scene's published hashes name `AutoPaint`s, and
a record cached under one would let the native pass
(`canvas::sceneResolve`) resolve the item without the walk seeing it. An `AutoPaint` is
therefore always resolved by the MFBASIC walk, as a `Picture` is, and a frame holding
one copies the scene out of the ring.

After `canvas::sceneLayout`, when the frame has an instance or the last frame had one
(`__canvas_autoPending`), `__canvas_autoResolve`:

1. reads `canvas::pointerSnapshot` and `canvas::frameNanos`, once;
2. if the pointer is inside the surface, finds the topmost draw entry under the centre
   of its pixel (`__canvas_autoHitAt`: `__canvas_hitEntry` over the frame's entries as
   the walk left them, kept in `__CANVAS_AUTO_BASE`, last first). Every entry is
   tested, so an item that is not an `AutoPaint` and has no `id` takes the pointer from
   what it covers;
3. steps each instance, with "under the pointer" meaning its entry is that entry;
4. for an instance showing a state paint, builds the shape with that paint
   (`__canvas_autoItemWith`) under the item's hash stepped by 16 + the paint's number,
   and puts that record's offset in the frame's entries and in `__CANVAS_TOP_OFFSETS`,
   and steps the entry's draw hash the same way;
5. publishes that an `AutoPaint` is live (`canvas::autoPublish`).

**The hit test reads the entries before step 4 swaps any**, so a region is always the
shape's own paint's — its stroke width and its clip. A state paint with a wider stroke
does not widen it, and one with a different clip does not move it.

**A state paint is a different record under a different draw hash.** That is all that
connects the state to the renderers: the software walk and the damage diff read the
swapped entry, and so does the GPU draw list, which is laid out from
`__CANVAS_TOP_OFFSETS`. A group's run is laid out once per frame and shared by every
node that draws the group (`__canvas_memoGroup`), so a node under which some `AutoPaint`
shows a state paint gets a run of its own: the memo row is keyed by the group's slot AND
`__canvas_autoVariant` of that use, which is 0 — the shared run — when nothing in it
shows one.

### The two redraw triggers

A redraw is otherwise owed only by a `present`, a `setBytes` on a drawn image, or a
resize. An `AutoPaint` adds two ways for the render loop to wake, and neither OWES a
frame:

* **The pointer changed.** Each platform's pointer handler, after it publishes, reads
  `GRAPHICS_OFFSET_AUTOPAINT_LIVE` and, when it is set, calls `canvas::signalRedraw`
  with 1 ([[src/codegen/runtime/canvas/mod.rs:emit_skip_unless_autopaint_live]]). The
  word is set by step 5 and cleared by the first resolve that finds no instance. With
  it clear, a pointer change signals nothing: a scene with no `AutoPaint` renders no
  frame for any pointer activity.
* **A window ended.** `canvas::waitForRedraw` takes a timeout, and the render loop
  passes `__canvas_autoTimeout`: the time left to the earliest deadline in the table,
  or 0 — wait until signalled — when no instance is in a window.

`canvas::signalRedraw(pointerOnly)` with exactly 1 sets `GRAPHICS_OFFSET_AUTO_WAKE`
under the graphics mutex and signals; with anything else it sets `PENDING` and `WANTED`
as it always did ([[src/codegen/runtime/canvas/mod.rs:emit_signal_redraw]]). The resize
handlers pass 0. `canvas::waitForRedraw(timeoutNanos)` answers
([[src/codegen/runtime/canvas/mod.rs:emit_wait_for_redraw]]):

| Answer | When | Takes |
|---|---|---|
| 0 | the loop is stopping, and nothing is pending | nothing |
| 1 | `PENDING` is set | `PENDING` and `AUTO_WAKE`; broadcasts `FRAME_COND` |
| 2 | `AUTO_WAKE` is set, or the timeout ran out | `AUTO_WAKE` |

They are checked in that order, so a redraw outranks a stop and a stop outranks a
pointer wake or a deadline: shutdown never waits for a window to end. The timeout is
turned into one absolute `CLOCK_REALTIME` deadline before the lock is taken — the clock
`pthread_cond_timedwait` takes — and is compared again after every wake, so a spurious
wake neither returns early nor restarts it. A wall-clock step during one wait moves
that one wait, as it does `canvas::waitFrame`'s.

**A frame woken with answer 2 is drawn only if some instance now shows a different
paint, and that is decided without walking the scene.** `__canvas_renderFrame` asks
`__canvas_autoLight` first. Nothing is owed, so nothing but the pointer and the clock
has changed since the last walked frame: its entries (`__CANVAS_AUTO_BASE`), their
group transforms and the geometry they name are as that frame left them, and each table
row holds its instance's entry and flags. `__canvas_autoLight` reads the pointer, makes
the one hit test, steps every row as `__canvas_autoResolve` would, and answers whether
any row's paint changed. On FALSE the frame ends there — counted in `skipped=`, nothing
walked, nothing presented, whatever `MFB_CANVAS_DAMAGE` says. On TRUE the frame is an
ordinary one from that point, and its resolve steps the same rows again moments later
to no effect: a window the first step started is still running and a press it took is
still held.

Measured on macOS AArch64 with a `--debug` build, 2,000 scripted pointer changes 1 ms
apart over a scene of 10,000 rectangles and one `AutoPaint` were 1,842 separate wakes,
and over 1,000 rectangles 1,830: a wake is under a millisecond at either size, its
stats line included. Before this function existed a wake walked the scene, which
copies it out of the ring, and cost 8 ms at 10,000 items.

A pointer change can land between a frame's snapshot (step 1) and the first store of
the live word (step 5), where the handler reads 0 and signals nothing. The resolve that
first sets the word reads the pointer again afterwards and, if it differs, has the loop
wait 1 ns — one more resolve, with nobody asking.

### What it costs a program without one

`ABSENT`'s bodies are a constant or the argument handed back. Per frame that is two
calls (`__canvas_autoTimeout`, `__canvas_autoPending`), per group node one in each of
the two walks (`__canvas_autoPath`), per group node in the GPU walk one more
(`__canvas_autoVariant`), per `Text` record built one (`__canvas_autoTextBox`). The two
graphics-state words are zero and nothing reads `AUTO_WAKE`'s being set, because nothing
sets it.

In code, a canvas program with no `AutoPaint` is 13,628 bytes larger than it was before
the type existed (`__text`, macOS AArch64; one more 16,512-byte page of file): the
`AutoPaint` arms the item unions oblige every `MATCH` to have, the walks' two extra
parameters, the timed wait, and `ABSENT`. The shape converter (`__canvas_autoItem`) and
the item hash (`__canvas_autoHash`) are in the gated pair rather than the unconditional
source because they alone were another 24,744.

### The `--debug` affordances

`MFB_CANVAS_POINTER`'s script (above) is applied at the start of a frame by the graphics
thread, so no handler signals for it. With an `AutoPaint` live, `__canvas_autoTimeout`
also waits no longer than the script's next entry (`__canvas_pointerNextDue`), and the
entry is applied by the frame that wakes. `MFB_CANVAS_AUTO_WINDOW_MS` replaces the
project's window for one run (1 to 3,600,000; anything else leaves it). The stats line gains
`auto=`: the paint each instance of the frame shows, one digit each in walk order — 0 the
shape's own, 1 enter, 2 exit, 3 hover, 4 pressed — or `-` when the frame had none. A
normal build has none of the three.

## Images are named, not embedded

An `Image` is an ordinary resource, closing when it leaves scope or with
`canvas::destroyImage`. An item that draws one names it directly:
`canvas::Picture.image` is a `RES canvas::Image`, and `canvas::Text.font` is a
`RES canvas::Font`.

**Naming a resource in a scene does not keep it alive.** The scene draws through
the image you still own, and closing it is still yours to do — `canvas::destroyImage`
means what it says, and takes effect at once. An item whose image has been closed
draws nothing; the frame around it renders normally and nothing is raised. That is
what makes the two lifetimes independent without the scene having an opinion about
either.

Destroying an image a presented scene still draws is therefore safe. Closing an
image sets its closed flag and frees nothing, and there is no GPU-side copy to
release: every backend reads a frame's texels from the image's own storage while
that frame is built, so a frame already in flight finishes with the pixels it
started with and the next one sees the image closed. That is entirely runtime-side
and invisible from MFBASIC: there is no reference count, no generation table, and
nothing for a program to synchronise.

### A named group is the exception

`canvas::setGroup` is the one place where naming a resource *does* keep it usable.
A group outlives the `present` that installed it — that is the point of installing
one — so an image or font named in its items becomes the group's to close, and the
program does not close it:

```
FUNC installPanel() AS Nothing
  RES face AS canvas::Font = canvas::loadFont("ui.ttf")
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 8.0, y := 24.0], text := "Ready", font := face, fontSize := 18.0, paint := canvas::fill(color::rgb(230, 230, 230)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("panel", [label])
END FUNC
```

`installPanel` returns and its bindings go out of scope, and the group keeps
drawing. Presenting a `canvas::Group` naming `"panel"` still shows the text.

The compiler enforces the hand-off rather than leaving it to a convention: after
`canvas::setGroup`, naming the same font again is `2-203-0055`. If two groups need
one image, build the image twice. This is deliberately stricter than a runtime
rule would be, because the failure it prevents is silent — an item whose image has
been closed draws nothing and raises nothing, so a group closing an image another
group still names would show up only as a picture that is quietly wrong.

`canvas::removeGroup`, and a `canvas::setGroup` that replaces the items, close what
the old items named — *unless something still on screen names it too*. A group
rebuilt every frame from one long-lived font therefore keeps drawing: the font is
named by the new items as well as the old, so it stays usable. There is still no
reference count; the question asked is "does anything drawn right now name this?",
and it is asked only when a replaced group's items are being discarded.

One consequence follows from the field being a resource rather than a plain value:
**a `List OF canvas::DrawItem` cannot cross a thread's data plane.** A `Picture` or
a `Text` carries a resource, and a resource crosses a thread only on the `RES`
plane, so sending a scene to another thread is refused with
`2-203-0138 TYPE_THREAD_RESOURCE_PLANE_REQUIRED`. Move the `Image` or `Font` with
`thread::transfer` / `thread::accept` and build the scene on the receiving side.

### Image content is orthogonal to the scene

An image's *pixels* are mutable without touching the scene. `canvas::setBytes`
replaces them behind the image, and the change appears on the next rendered frame —
no `present` is involved, because the scene has not changed: the same items are in
the same places, and only the content of one of their images is different.

This is why a video frame, a plot, or a progress bar can update without rebuilding
the scene at all.

**`setBytes` asks for that frame itself** when something live names the image — the
installed scene, its layers, or any installed group — and, like `present`, waits for it
when `MFB_CANVAS_SYNC` is set. Nothing else would: re-presenting the unchanged scene is
refused by the frame skip. An image nothing draws is refilled without a frame. The
image's storage is replaced rather than overwritten, so a frame already drawing keeps
the pixels it started with.
[[src/codegen/builtins/canvas/func_set_bytes.rs:BODY]]

The runtime keeps its own copy of every image's pixels as the source of truth the
backend is uploaded from, so `canvas::getBytes` answers without a GPU readback —
a memory copy rather than a pipeline stall — and a lost device is recovered by
re-uploading rather than by asking the program to redraw.

An RGBA8 image is exactly `width * height * 4` bytes; any other length is
`ErrBadPixelCount`. An image cannot be resized, only re-filled.

## Composite glyphs draw their components

A TrueType glyph with a negative contour count is a **composite**: a list of other
glyphs, each placed by an offset and optionally a uniform, x/y or 2×2 F2Dot14
scale. Accented letters in most fonts are built this way. The outline reader walks
the component records and composes each component's transform with the glyph's own
surface affine (scale, Y flip, pen). Simple components are drawn at their placed
coordinates, and nested composites recurse to a depth of 8, so a cycle in a broken
font draws nothing for that component rather than looping. The offset is applied
unscaled unless `SCALED_COMPONENT_OFFSET` is set, as in FreeType. A point-matched
component (its arguments are point numbers, not an offset) is drawn unoffset; no
system font on the reference host uses one. A simple glyph's edges are computed
exactly as before. [[src/codegen/builtins/canvas/helper_glyph.rs:COMPOSITE_EDGES]]

## A loaded font is always one standalone face

`canvas::loadFont` accepts a TrueType Collection (`ttcf`, a `.ttc` file) as well as
a plain sfnt. A collection is a header, a face count, and one table-directory offset
per face; each face's table offsets count from the start of the *file*. The loader
does not teach the renderer about collections. It copies the chosen face out into a
standalone sfnt — the face's own header, one record per table with its offset
rebased, the tables padded to four bytes — and stamps the resource around that.
[[src/codegen/builtins/canvas/func_load_font.rs:COLLECTION]]

So everything past the load reads the table directory at byte `0`
(`__canvas_fontTable`), and neither the `Font` record, the graphics thread's font
table, nor any glyph helper knows collections exist. A plain `.ttf` is its own single
face at directory `0` and is handed on byte-for-byte, so its pixels are unchanged. The
TrueType-outline and `unitsPerEm` checks run on the extracted face, so a collection of
CFF faces is refused exactly as an `OTTO` file is.

`loadFont(path)` takes face 0. `loadFont(path, face)` walks every face for one whose
`name` table nameID 6 (PostScript name) equals `face`, then again for nameID 4 (full
name); no match is `ErrNotFound`, since the file itself is a good font.
[[src/codegen/builtins/canvas/func_load_font.rs:LOAD_FONT_NAMED]] Names are read
preferring the Windows Unicode record in US English, then any Windows Unicode record,
then Mac Roman; a malformed string reads as `""` rather than failing, so one bad
record cannot stop a search that another face would satisfy.
[[src/codegen/builtins/canvas/helper_font_name.rs:FACE_NAME]]

A collection claiming more than 4096 faces is treated as holding none, and a face
whose directory or tables run past the end of the file is dropped or refused with
`ErrBadFontFile` — the bug-509 rule that a hostile header costs a comparison, not a
loop or an out-of-bounds read.

## Every text path reads one shaped run

A string becomes glyphs in exactly one function, `__canvas_shapeRun(fonts, text)`,
which takes font **handles** in fallback order and returns flat
`(fontIndex, glyphId, advanceUnits)` triples, one per codepoint. Each codepoint takes
the first font whose `cmap` maps it to a glyph other than `0`; one no font maps is
glyph `0` (`.notdef`) of the first font.
[[src/codegen/builtins/canvas/helper_font.rs:SHAPE_RUN]] `canvas::measureText`, the
filled glyph run, the stroked text edges, and the `MFB_CANVAS_GEO_VERIFY` reference for
the glyph run all read that run, and none of them looks up a glyph or an advance
itself. The `canvas` unit test `canvas_text_has_one_glyph_walk` enforces this over the
assembled source. [[src/codegen/builtins/canvas/mod.rs:canvas_text_has_one_glyph_walk]]
Today each caller passes a one-font list.

Advances stay in font units, because the callers deliberately convert them
differently. `measureText` sums the advances as Integers and scales once.
[[src/codegen/builtins/canvas/func_measure_text.rs:MEASURE_TEXT]] The filled run and
its verify reference add a scaled Float per glyph and snap each glyph's pen to a whole
pixel, because a glyph is a cached bitmap placed on integer pixels. The stroked edges
add the same Floats but do not snap, because edges are exact geometry.
[[src/codegen/builtins/canvas/helper_geometry.rs:__canvas_textGlyphRun]] The two sums
can differ in the last bit, so this split is part of the output, not an accident of
it.

`measureText` gets its handle from the internal `canvas::fontOpenHandle`, which raises
`ErrResourceClosed` for a closed font; the renderer's `canvas::fontHandle` answers `0`
instead, so that a scene naming a closed font draws nothing rather than stopping.
[[src/codegen/builtins/canvas/func_measure_text.rs:lower_font_open_handle]]

The bytes behind a handle come from the process-global font table that `loadFont`
registers into and `destroyFont` clears. It is a chain of sixteen-slot chunks. The
worker links in a new arena chunk only when every slot is taken, and chunks never move
and are never freed, so the graphics thread walks the chain without a lock. A slot's
handle and a chunk's `next` word are published with a store-release and read with a
load-acquire on AArch64. [[src/codegen/builtins/canvas/gen_font_table.rs:emit_register_font]]
[[src/codegen/builtins/canvas/gen_font_table.rs:emit_lookup_font]] Every live font
therefore draws and measures, however many are open. The table was once sixteen fixed
slots, and a seventeenth font's glyphs silently drew nothing (bug-756).

## System fonts are an OS query, then an ordinary load

`canvas::listSystemFonts` and `canvas::loadSystemFont` rest on one native member,
the internal `canvas::systemFontTable`, which asks the operating system for its
installed faces and answers a single `String`: per face, the full name, the
PostScript name and the file path, each ended by U+001F, the record ended by U+001E.
Everything after that is MFBASIC: the table is split on its UTF-8 bytes (neither
separator can occur inside a multi-byte sequence), names are sorted and de-duplicated,
and `loadSystemFont` hands the chosen path and PostScript name to
`canvas::loadFont(path, face)` — the same load a program can make itself.
[[src/codegen/builtins/canvas/func_system_fonts.rs:lower_system_font_table]]
[[src/codegen/builtins/canvas/func_system_fonts.rs:SYSTEM_FONT_FIELDS]]

The table is read once per thread, on first use, and kept: the OS query measured
0.58 s per call on macOS against 0.08 s for the load itself. A font installed while a
program runs is seen by the next run.

Each backend lists only faces the loader can draw — TrueType (`glyf`) outlines, not
a variable font — and only faces with a full name and a file:

- **macOS** — CoreText. `CTFontManagerCopyAvailableFontURLs` answers one URL per
  face (a collection's URL repeats) and no face index, which is why every backend
  reports the PostScript name and the loader finds the face by its `name` table.
  Filters: `CTFontCopyTable(glyf)`, no `CTFontCopyVariation` (CoreText lists a variable
  font's named instances under PostScript names no face carries), and no full name
  starting with `.` — macOS's private UI faces, visible to an app process.
  [[src/codegen/builtins/canvas/gen_system_fonts_macos.rs:emit_system_font_table]]
- **Linux** — fontconfig, reached through `dlopen("libfontconfig.so.1")`, never a
  `DT_NEEDED`: without it the table is empty and the program still runs.
  `FcFontList(NULL, …)` uses the current configuration. Filters: `fontformat` is
  `TrueType`, `variable` is not true, `index >> 16` is 0 (not a named instance).
  [[src/codegen/builtins/canvas/gen_system_fonts_linux.rs:emit_system_font_table]]
- **Windows** — DirectWrite's system font collection through COM; only
  `DWriteCreateFactory` is imported. Filters: no simulations (DirectWrite synthesises
  bold/oblique faces no file carries), `TryGetFontTable(glyf)`, no
  `IDWriteFontFace5::HasVariations`, a local file. Names are taken in `en-us` when
  present, else the first string.
  [[src/codegen/builtins/canvas/gen_system_fonts_windows.rs:emit_system_font_table]]

Text drawn in a system font is not reproducible across machines. Text goldens load a
fixture font with `canvas::loadFont`, never a system font.

A package companion — canvas's included — may call only `collections`' native
members: the `collections` source is injected from the *program's* imports, so a
companion calling a source generic such as `collections::sort` links only when the
program also imports `collections`. canvas sorts the font names with its own
`__canvas_sortedUnique`, and a registry test fails any companion that breaks the rule.
[[src/codegen/builtins/canvas/func_system_fonts.rs:SORTED_UNIQUE]]

## Mode gating

Every `canvas::` call that touches the surface requires `Mode.Canvas` and raises
the trappable `ErrWrongMode` elsewhere, on the same seam `term::` uses.
[[src/codegen/app/hook/app.rs:ModeRequirement]]

The **value constructors are exempt**: `canvas::fill`, `canvas::stroke` and
`canvas::fillStroke` build a `canvas::Paint` and touch no surface, so a program may
assemble its paints before it presents anything. Gating them would buy no safety and
cost real ergonomics.

That list used to name `canvas::rgb` and `canvas::rgba` as well. It no longer can:
plan-122-D removed them, and colour construction lives in `color`, which is not an
app-mode package at all. So the exemption did not grow to cover colour — colour left
the gated package entirely. `color::rgb` is callable from a console build, from a
`Mode.Term` build, and from a canvas build before `setMode`, because nothing about
`color` is mode-dependent. See `./mfb spec stdlib color`.

`canvas` is importable only in `--app` builds: `IMPORT canvas` in a console build
is a compile-time error, because a console binary has no surface to draw on.

## See Also

* ./mfb spec app presentation-mode — the `Mode` enum and the surface-reconcile seam
* ./mfb man canvas — the per-call API
* ./mfb spec language types — records, unions, and named construction
* ./mfb spec language resource-management — the RES ownership model `Image` follows
