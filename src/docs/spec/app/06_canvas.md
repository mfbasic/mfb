# Canvas

The 2D drawing surface `app::setMode(Mode.Canvas)` presents, and the model the
`canvas` package draws on it. This topic specifies the *contract* — what a scene
is, what installing one guarantees, and how images live; the per-call API is
`./mfb man canvas`, and the mode itself is `./mfb spec app presentation-mode`.

## Retained, not immediate

`canvas` is a **retained scene**: a program builds a `List OF DrawItem` and
installs it with `canvas::present`, and the runtime keeps rendering that scene —
on vsync, on resize, on damage — until the next `present` replaces it.
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

## A published scene points at nothing caller-owned

`present` **deep-copies the scene transitively** into runtime-owned storage:
item fields, a `Polygon`'s point list, a `Text`'s string, the `Paint` values.
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
transform, and a zero-area (absent) clip. One consequence is worth stating
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

## Rendering conventions

These are the observable contract a backend must meet, not implementation notes.
The software rasteriser fixes them; the GPU backends match them within a
documented tolerance rather than exactly, because rasterisation rules and blend
precision differ legitimately between drivers.

**Compositing happens in linear light.** A colour's channels are sRGB-encoded
bytes, so they are decoded to linear before blending and re-encoded on store. The
transfer function is the standard one — `c / 12.92` below `0.04045`, else
`((c + 0.055) / 1.055) ^ 2.4` — evaluated over a 256-entry table so the result
cannot depend on a platform's `pow`. Blending is `dst + (src - dst) * alpha / 255`
on the linear values, rounded to nearest.

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

**`Paint.clip` restricts an item to a rectangle.** The rectangle is axis-aligned,
in surface pixels, and unaffected by `Paint.transform` — `Bounds` cannot express a
transformed rectangle. A zero-area or negative-extent `Bounds` means no clipping,
which is what the zero value gives.

Its edges need not fall on pixel boundaries. A clipped pixel's coverage is the
clip's own `clamp(0.5 - d, 0, 1)` on the signed distance to that rectangle,
multiplied into the shape's coverage as `(coverage * clipCoverage) / 255`. A clip
edge is therefore antialiased exactly as a shape edge is, and clipping a shape to a
rectangle larger than it changes nothing.

## Images are named, not embedded

An `Image` is an ordinary resource, closing when it leaves scope or with
`canvas::destroyImage`. A scene never holds one: an item that draws an image
carries an `ImageRef`, a plain value holding the id the backend knows the image
by, obtained with `canvas::imageRef`.

That indirection is what makes the two lifetimes independent. A scene holding
resources would have to keep them alive, which would make `canvas::destroyImage`
a lie; holding only an id means an installed scene has no opinion about any
image's lifetime at all. Destroying an image a presented scene still draws is
therefore safe — the runtime simply defers freeing the backing object until the
GPU has finished with the last frame that used it. That deferral is entirely
runtime-side and invisible from MFBASIC: there is no reference count, no
generation table, and nothing for a program to synchronise.

### Image content is orthogonal to the scene

An image's *pixels* are mutable without touching the scene. `canvas::setBytes`
replaces them behind the id, and the change appears on the next rendered frame —
no `present` is involved, because the scene has not changed: the same items are in
the same places, and only the content behind one of their ids is different.

This is why a video frame, a plot, or a progress bar can update without rebuilding
the scene at all.

The runtime keeps its own copy of every image's pixels as the source of truth the
backend is uploaded from, so `canvas::getBytes` answers without a GPU readback —
a memory copy rather than a pipeline stall — and a lost device is recovered by
re-uploading rather than by asking the program to redraw.

An RGBA8 image is exactly `width * height * 4` bytes; any other length is
`ErrBadPixelCount`. An image cannot be resized, only re-filled.

## Mode gating

Every `canvas::` call that touches the surface requires `Mode.Canvas` and raises
the trappable `ErrWrongMode` elsewhere, on the same seam `term::` uses.
[[src/codegen/app/hook/app.rs:ModeRequirement]]

The **value constructors are exempt**: `canvas::rgb`, `canvas::rgba`,
`canvas::fill`, `canvas::stroke` and `canvas::fillStroke` build values and touch
no surface, so a program may compute its palette before it presents anything.
Gating them would buy no safety and cost real ergonomics — the same reasoning that
leaves `io::readByte` outside the gated console-read set.

`canvas` is importable only in `--app` builds: `IMPORT canvas` in a console build
is a compile-time error, because a console binary has no surface to draw on.

## See Also

* ./mfb spec app presentation-mode — the `Mode` enum and the surface-reconcile seam
* ./mfb man canvas — the per-call API
* ./mfb spec language types — records, unions, and named construction
* ./mfb spec language resource-management — the RES ownership model `Image` follows
