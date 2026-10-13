Take a look at the canvas API and native backend...
I would like to do something like this:

--

## Big Change:

* Remove canvas::RoundedRect. `canvas::Rectangle` carries the four corner radii.
* Remove transform from canvas::Paint. Every shape and `Group` carries its own `transform`.
* Remove canvas::Point replace with vector::Float2. Everywhere, not just the shapes:
  `Gradient.startPoint` / `Gradient.endPoint`, `Paint.clip`, `MouseEvent.position`,
  `Polygon.contours`, `PolyLine.points`. `canvas` gains `vector` as an import.
* Remove `Ellipse.angle`. Rotation is the shape's `transform`.
* `Rectangle` and `Picture` lose `x` / `y` / `w` / `h` for `position` / `size`.
* `Group` loses `dx` / `dy` for `position`, and gains a full affine `transform`.
* local geometry → shape transform → position → enclosing group transforms

This is a source-breaking rewrite of the canvas surface. Every canvas example, golden,
`mfb man canvas` page and `mfb spec app canvas` section changes with it. Nothing here is
expected to be byte-neutral.

### Transform order, spelled out

A point `p` of a shape's local geometry reaches the surface as:

    world = G1( G2( ... position + transform(p) ... ) )

* **Local geometry has its origin at the reference point.** A `Circle`'s centre is local
  `(0, 0)`; a `Polygon`'s contours are offsets from `position`.
* **The transform pivots about the reference point.** Rotating a `Circle`, `Ellipse` or
  `Arc` turns it about its centre. A `Rectangle` pivots about its bind point: `Center`
  spins it in place, `TopLeft` swings it about that corner.
* **`transform.tx` / `transform.ty` stay and add to `position`.** They are redundant with
  it for a single shape; they are kept so one `Transform` value means the same thing on a
  shape and on a group.
* **The all-zero `Transform` is still the identity**, and a transform with no area (a zero
  row, or `|a*d - b*c|` below `1e-12`) is still treated as the identity. Both rules apply
  to a group's transform as well.
* **A stroke is transformed with its shape**, as today, and by every enclosing group's
  transform too: a group scaled by 2 draws a 4-pixel outline 8 pixels wide.
* **`Paint.clip` and `Paint.gradient` are in the shape's local space.** The shape's
  `transform`, its `position` and every group transform carry them, exactly as
  `Paint.transform` carried them before. This moves every existing clip ring and gradient
  `startPoint` / `endPoint`: they were in surface coordinates and are now offsets from
  `position`.
* **A group composes the same way.** Its items are placed in the group's local space, then
  `group.transform`, then `group.position`, then the next group out.

### Group transform is a renderer change, not a field rename

Today a group is translate-only: the GPU draw-list entry carries `dx` / `dy` (16.16 words
plus the bug-735 residual words) and both stages bind that offset. A full affine means:

* The draw-list entry carries a 2x3 matrix per draw, not an offset. It must stay per-draw
  rather than be folded into the items, because a group's blocks are recorded once and
  shared by every use of the group.
* The vertex stage maps the quad through the matrix; the fragment stage maps the query
  point through its inverse, on Metal and Vulkan, and the software renderer does the same.
  Distance, gradient ramp, glyph coverage and clip are all evaluated at that local point.
* Nested groups compose into one matrix per draw entry. The depth guard stays 64.
* A group whose transform is the identity must render byte-identically to a translate-only
  group today, including the bug-735 residual precision on the translation.

## Add new:

UNION Shape
    Rectangle
    Line
    Polygon
    Circle
    Arc
    Text
    Ellipse
    PolyLine
END UNION

UNION AutoShape INCLUDES Shape
    AutoPaint
END UNION

UNION DrawItem INCLUDES AutoShape
    Picture
    Group
END UNION

* `INCLUDES` is not subtyping: a `Shape` value is not accepted where a `DrawItem` is
  expected. The member records are, so `canvas::Rectangle[...]` is still a `DrawItem`.
* The variant tags renumber. Today each variant's tag is fixed by its position in
  `DrawItem` and new variants are appended last to keep the others still. Nothing may
  assume the old order: the item hash, the geometry kinds and the container exemption list
  all get re-derived.

' Will update the Shape's paint based on the current mouse movement
' default - Keep the shape's original paint unchanged
' hover - set when the mouse is within the Shape
' pressed - set when the mouse is within the Shape and a mouse button is pressed
' enter - set when the mouse enters the shape for 50ms then to hover; auto-scheduled redraw when after expires (even if the pointer stops moving)
' exit - set when the mouse exits the shape for 50ms then to default; auto-scheduled redraw when after expires (even if the pointer stops moving)

TYPE AutoPaint
    shape      AS canvas::Shape
    useEnter   AS Boolean
    useExit    AS Boolean
    useHover   AS Boolean
    usePressed AS Boolean
    enterPaint   AS canvas::Paint
    exitPaint    AS canvas::Paint
    hoverPaint   AS canvas::Paint
    pressedPaint AS canvas::Paint
END TYPE

> The four paint fields were first written here as `enter`, `exit`, `hover` and
> `pressed`. `EXIT` is a keyword, so a field cannot be named `exit`, and all four took
> the suffix rather than one (plan-172-K; confirmed by the owner, 2026-10-11). The block
> above was brought into line by plan-172-M's surface audit.
>
> The enter and exit flashes last 100 ms, not 50, and the length is the project's
> `options.canvasAutoPaintMS` (owner, 2026-10-11, after trying both in a real window).

### AutoPaint states

* Priority, highest first: `pressed`, `enter`, `hover`, `exit`, default.
* A state whose `use*` flag is FALSE falls through to the next one down that is in effect:
  `pressed` unused shows `hover`; `enter` unused shows `hover` at once; `hover` unused
  shows the default paint; `exit` unused shows the default paint at once.
* `enter` and `exit` are flash paints held for a fixed 50ms, not transitions. Nothing is
  interpolated.
* Re-entering during the `exit` window cancels it and starts `enter`. Leaving during the
  `enter` window cancels it and starts `exit`.
* `pressed` is any mouse button, and needs the press to have begun inside the shape. While
  that button is held: pointer inside shows `pressed`; dragged out it goes through `exit`
  to default; dragged back in it shows `pressed` again. A press that began outside and is
  dragged in shows `hover`, not `pressed`.
* Only the **topmost** item under the pointer is hovered or pressed. Any drawn item above
  it blocks it, whether or not that item is an `AutoPaint`. A `Group` blocks nothing
  itself; its items do.
* An `AutoPaint` cannot wrap an `AutoPaint` (its `shape` is a `Shape`), so states do not
  nest.
* `AutoPaint` carries four extra `Paint` values per item. They ride every `present`
  deep-copy and item hash, so it is for the handful of interactive items in a scene, not
  for every item.

### Where AutoPaint is resolved: the graphics thread

* Main publishes the pointer position, the button state and whether the pointer is inside
  the surface into the graphics state with plain stores, the way it publishes the resize
  size. The worker is not involved: hover keeps working while the program is blocked in
  `io::input`.
* The graphics thread owns the per-item state (which state, and the enter/exit deadline).
  State is kept per item *instance* across scene revisions, so a program that re-presents
  every frame does not restart the 50ms window each frame.
* The paint the graphics thread picks folds into the item's draw hash, as a picture's
  pixel-block address does. Without that the geometry cache and the damage diff reuse the
  previous state's pixels.
* Two new redraw triggers join the five in `.ai/canvas-threading.md` §4:
  * 6 - the pointer state changed **and the live scene holds an `AutoPaint`**. Signalled
    by main.
  * 7 - an enter/exit deadline expired. The render loop's wait becomes a timed wait to the
    earliest deadline. This is the one time-based repaint, and it exists only while a
    deadline is pending.
  A scene with no `AutoPaint` still costs zero frames when the pointer moves.
* Headless tests need a scripted pointer, the twin of `MFB_CANVAS_RESIZE_W` / `_H`.

### Groups and identity

* group interaction. A group saves the DrawItem list. Each use of the group gets its own identity. This means if a group is used twice, AutoPaint applies separately to each DrawItem instance.
  * That identity is the instance's place in the scene — which nodes and which items led
    to it — and never an `id` (owner, 2026-10-10: "ID is more for mouse overId, not for
    AutoPaint logic"). Two uses of one group are independent whether their `id`s differ,
    match or are empty.
* `overID` is a **path of ids**, outermost first (owner's decision, 2026-10-10, replacing
  "the item's `id` string and nothing more"). A `Group` node has an `id`, and so does a
  `Picture`; each contributes its own in front of what it holds, and the shape's `id` comes
  last. The parts are joined by `::` — the language's own namespace separator (owner's
  answer to plan-172-I Open Decision 1) — so a shape `"label"` in a group `"save"` reports
  `"save::label"`.
  * An `AutoPaint` has no `id` and adds nothing to the path. It is a decorated shape, not
    something that uses a shape (as a `Picture` does) or holds items (as a `Group` does):
    it reports its shape's `id`, exactly as the bare shape would.
  * A part that is empty is left out. A group or picture with no `id` adds nothing, so a
    scene that names only its shapes reports exactly those names.
  * A shape with no `id` inside a named group or picture reports the container's path
    alone: a program usually wants to know which button, not which part of the button.
  * With no `id` anywhere on the path, `overID` is `""`, and the item still blocks what
    is beneath it.
  * This is why containers have their own: a group's items are stored once and a shape is
    a value a program reuses, so two uses of one group, or two pictures built on one
    shape, would otherwise report the same string — and since a group can turn and scale
    its contents, telling the uses apart by `position` would mean redoing the hit test.

### Hit-testing

* Hit-testing on stroke-only shapes. Line, PolyLine and Arc have zero area. Their hit region is the stroke, at least 3 pixels either side of the centre line, otherwise AutoPaint on a Line is unusable.
* A closed shape (Rectangle, Circle, Ellipse, Polygon) is hit on its interior plus its
  stroke, whether or not its paint fills it. An outlined button hovers when the pointer is
  inside the outline.
* `Text` is hit on its layout box (advance width by ascent plus descent), not on its
  glyphs.
* A `Picture` is hit as its shape is.
* Hit-testing is based on the world shape points after all transforms apply. Not the bounding box, but the final shape. Internally a "fast" bounding box check can happen as a 1st pass, but the final hit-testing is the actual shape
* Hit-testing is based on the Shape's hit-box. If hover changes the shape, for example, it does not change the hit-box. The hit region is always built from the shape's own default `paint` (its `strokeWidth` and its `clip`), never from the state paints.
* The default paint's `clip` cuts the hit region: what is clipped away cannot be hovered.
* One hit-test function serves both `AutoPaint` (graphics thread) and `overID` (worker),
  so the two cannot disagree about what is under a point.

## Updates / Cleanups:

ENUM BindTo
    TopLeft
    Center
END ENUM

ENUM PictureClip
    ClipOnly            ' Clip picture to the Shape
    ClipAndTransform    ' Clip picture to the Shape and have the shape transform apply to image
END ENUM

' Contain and Cover center the image

ENUM PictureFit
    Stretch             ' Fill the shape's bounds exactly
    Contain             ' Preserve aspect ratio and fit the entire image inside the bounds
    Cover               ' Preserve aspect ratio and fill the bounds completely
END ENUM

TYPE MouseEvent
    kind     AS MouseKind
    button   AS MouseButton
    position AS vector::Float2
    shift    AS Boolean
    ctrl     AS Boolean
    alt      AS Boolean
    overID   AS String
END TYPE

* `overID` is the id path of the topmost item under `position` — the `id`s of the groups
  and the picture that hold it, outermost first, then its own, empty ones left out
  ("Groups and identity", above) — or `""` when there is no item there or nothing on its
  path has an `id`. An item with an empty `id` is still hit-tested: it still blocks what
  is beneath it and still drives its own `AutoPaint`.
* It is resolved on the worker when `canvas::pollMouse` returns the event, against the
  scene the program last presented. It is not resolved when the event happened, so it can
  name an item that was not there at that moment.

' Corner radius, per corner. One value, two meanings chosen by its size:
'
' Radius 0 or less:
'     A square corner.
'
' Radius above 0 through 1 (a fraction of the half-size, so the corner is elliptical):
'     Horizontal radius = radius * width / 2.
'     Vertical radius = radius * height / 2.
'
' Radius greater than 1 (pixels, so the corner is circular):
'     Horizontal and vertical radius = min(radius, width / 2, height / 2).
'     yes for a 1x1 size this make a subpixel error that noone can actually see
'     and I'm okay with that.
'
' The jump at 1 is intended: 1.0 is the full pill, 1.01 is a one-pixel corner. A radius
' animated in pixels has to start above 1.
'
' Per-corner radii and elliptical corners are both new: today's RoundedRect has one
' circular cornerRadius. How they are drawn is plan-172-E's decision.

TYPE Rectangle
    id        AS String
    position  AS vector::Float2   ' Based on bind
    bind      AS canvas::BindTo
    size      AS canvas::Size
    radiusTL  AS Float
    radiusTR  AS Float
    radiusBL  AS Float
    radiusBR  AS Float
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

TYPE Circle
    id        AS String
    position  AS vector::Float2   ' Center
    radius    AS Float
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

TYPE Ellipse
    id        AS String
    position  AS vector::Float2   ' Center
    radiusX   AS Float
    radiusY   AS Float
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

' An arc has no interior, as today. It is drawn from the stroke only.
TYPE Arc
    id        AS String
    position  AS vector::Float2   ' Center
    radius    AS Float
    startAngle AS Float
    endAngle  AS Float
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

' startAt and endAt are offsets from position, like a PolyLine's points. position is the
' pivot the transform turns the line about, which is why a two-point shape names three.
TYPE Line
    id        AS String
    position  AS vector::Float2   ' Reference Point
    startAt   AS vector::Float2
    endAt     AS vector::Float2
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

' text stays a String in this change. Attributed text is under Future below.
TYPE Text
    id        AS String
    position  AS vector::Float2   ' baseline's leftmost position
    text      AS String
    font      AS RES canvas::Font
    fontSize  AS Float
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

' Bigger update, contours are local to the position now
TYPE Polygon
    id        AS String
    position  AS vector::Float2   ' Reference Point
    contours  AS List OF List OF vector::Float2
    rule      AS FillRule
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

' Bigger update, points are local to the position now
TYPE PolyLine
    id        AS String
    position  AS vector::Float2   ' Reference Point
    points    AS List OF vector::Float2
    colors    AS List OF color::Color
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

TYPE Picture
    id        AS String                 ' The picture's own; it goes in front of its shape's in overID
    shape     AS canvas::AutoShape      ' This is the shape/position/etc the picture clipped to
    clip      AS canvas::PictureClip
    fit       AS canvas::PictureFit
    image     AS RES canvas::Image
END TYPE

* The picture has no paint of its own; it uses its shape's, with today's `Picture` meaning:
  `fill` tints the image, `transparency` fades it, `stroke` outlines the shape on top,
  `blend` and `clip` apply, and a fill gradient is ignored. The shape's interior is not
  filled separately behind the image.
* A `Picture` has its own `id`, and it goes in front of its shape's in `overID` (owner's
  decision, 2026-10-10, replacing "a `Picture`'s `id` is its shape's `id`"): more than one
  picture can be built on the same shape value.
* When the shape is an `AutoPaint`, the state paints change the tint and outline the same
  way.
* The shape must enclose an area. A `Picture` whose shape is a `Line`, `Arc`, `PolyLine`
  or `Text` draws nothing.
* The bounds `fit` fills:
  * `ClipAndTransform` - the shape's local bounding box. The image is fitted there and
    then carried by the shape's transform, so it rotates and scales with the shape.
  * `ClipOnly` - the axis-aligned bounding box of the shape after its own transform. The
    image is fitted there and drawn upright; only the clip outline is transformed.
  Enclosing group transforms apply to the image in both modes.
* Scaling still picks the nearest image pixel.

TYPE Group
    id        AS String           ' This use of the group; it goes in front of its items' ids in overID
    position  AS vector::Float2   ' Where the group's local origin lands
    transform AS canvas::Transform
    name      AS String
END TYPE

## Future (dont do this now):

### Attributed text

`Text.text AS AttributedString`. Needs its own rule for a span that carries a font or a
colour against the item's `font` and `paint` before it can land.

### 3D Object in 2D World

' This is drawing a 3d model in a 2D world, flat lighting etc.
' This is not meant to be a full 3D world.

TYPE Mesh
    vertices  AS List OF vector::Float3
    triangles AS List OF vector::Integer3
    UV        AS List OF vector::Float2
    normals   AS List OF vector::Float3
    colors    AS List OF color::Color       ' vertex colors
END TYPE

TYPE Object3D
    position  AS vector::Float2   ' Reference Point
    mesh      AS Mesh
    texture   AS RES canvas::Image
    transform AS canvas::Transform
END TYPE

### Particles - plan 150

' plan-150 builds on this document: it already uses vector::Float2 and canvas::Shape, so
' this lands first. With AutoPaint in the union ParticleSystem is the 12th DrawItem
' variant, not the 11th.
' Particles are never hit-tested and block nothing beneath them. An AutoPaint inside the
' template draws its default paint.
' The template is an AutoShape, not a DrawItem (owner, 2026-10-13; plan-150-A Corrections
' 3): a DrawItem that held a DrawItem is a type cycle, and a scene would stop being one
' flat block. So a Picture, a Group or another ParticleSystem cannot be a template.

ENUM canvas::EmissionMode
    Continuous
    Burst
END ENUM

TYPE canvas::ParticleSystem               ' the 12th DrawItem variant — a CONTAINER, no paint
    position    AS vector::Float2         ' emitter Reference Point
    item        AS canvas::AutoShape      ' the template drawn once per live particle
    emitter     AS canvas::Shape
    emission    AS canvas::Emission
    motion      AS canvas::Motion
    appearance  AS canvas::Appearance
    seed        AS Integer                ' fixes every per-particle random draw
    time        AS Float                  ' the program's clock in seconds; the program advances it
END TYPE

TYPE canvas::Emission
    mode          AS canvas::EmissionMode
    maxParticles  AS Integer   ' the slot pool; the hard bound on this system's quads
    rate          AS Float     ' Continuous: particles per second
    burst         AS Integer   ' Burst: particles per burst, clamped to maxParticles
    burstPeriod   AS Float     ' Burst: seconds between bursts; 0 = a single burst at time 0
    lifetime      AS Float     ' seconds a particle lives
    lifetimeVar   AS Float     ' ± fraction of lifetime, 0..1
END TYPE

TYPE canvas::Motion
    speed         AS Float     ' initial speed, px/s
    speedVar      AS Float     ' ± fraction, 0..1
    angle         AS Float     ' initial direction in radians; 0 = +X, matching Arc
    spread        AS Float     ' ± radians around angle; PI = omnidirectional
    gravityX      AS Float     ' px/s²
    gravityY      AS Float     ' px/s² — positive is DOWN, matching canvas' top-left origin
    drag          AS Float     ' exponential damping, 1/s; 0 = none
    rotation      AS Float     ' initial rotation, radians
    rotationVar   AS Float     ' ± radians
    spin          AS Float     ' radians/s
    spinVar       AS Float     ' ± fraction
    turbulence    AS Float     ' px amplitude of the wander
    turbulenceHz  AS Float     ' cycles/sec of the wander
END TYPE

TYPE canvas::Appearance
    startColor    AS color::Color   ' multiplied into the template's fill AND stroke
    endColor      AS color::Color
    startScale    AS Float
    endScale      AS Float
    scaleVar      AS Float     ' ± fraction, applied to both endpoints per particle
END TYPE
