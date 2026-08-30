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

## Images are named, not embedded

An `Image` is an ordinary owned resource, released when it leaves scope or by
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
