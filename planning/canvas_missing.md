# Canvas planned updates (2026-10-13)

These are separate features. Attributed text must be reconciled with plan 162 before
planning; 3D needs its projection model settled first.

## Attributed text

Existing:

```basic
canvas::measureText(font AS canvas::Font, size AS Float, text AS String) AS canvas::TextMetrics
```

Proposed overloads:

```basic
canvas::measureText(font AS canvas::Font, size AS Float, text AS AttributedString) AS canvas::TextMetrics
canvas::measureText(font AS canvas::Font, size AS Float, text AS AttributedString, maxWidth AS Float) AS canvas::TextMetrics

TYPE canvas::AttributedText
  id        AS String
  position  AS vector::Float2
  text      AS AttributedString
  font      AS RES canvas::Font
  fontSize  AS Float
  maxWidth  AS Float
  paint     AS canvas::Paint
  transform AS canvas::Transform
END TYPE
```

`measureText` and `AttributedText` must use the same wrapping and shaping rules.

Open decisions:

- Does `maxWidth <= 0` mean no wrapping, or does an omitted width use the current canvas
  width? The latter makes measurement change after a resize.
- For multiple lines, define `width`, `height`, `ascent`, `descent`, `lineGap`, explicit
  newlines, and an unbreakable grapheme wider than `maxWidth`.
- Plan 162 changes `font` to an ordered `fonts` fallback list. Decide whether these APIs
  launch before or after that change.
- Define how every `AttributedString` attribute affects canvas drawing: font-family
  strings, font size, bold, italic, underline, strike, overline, foreground, and
  background. In particular, define how a family string selects a `RES canvas::Font`.
- Decide whether `AttributedText` is a `Shape` or only a `DrawItem`; adding a `Shape`
  variant changes later union tags.

## Group compositing

Add `composite` to the existing `Group` draw item:

```basic
TYPE canvas::Composite
  transparency AS Float
  blend        AS canvas::BlendMode
  clip         AS List OF List OF vector::Float2
  clipRule     AS canvas::FillRule
END TYPE

TYPE canvas::Group
  id        AS String
  position  AS vector::Float2
  transform AS canvas::Transform
  composite AS canvas::Composite
  name      AS String
END TYPE
```

The required visual contract is isolated compositing: draw the children normally,
clip the flattened result, apply group transparency, then blend it into the parent
once. The default `Composite` should retain the existing direct-expansion fast path.

Open decisions:

- Is `clip` in group-local coordinates before `position` and `transform`?
- Does the group clip also restrict hit testing and `MouseEvent.overID`?
- Define nesting and bounds for intermediate surfaces, including very large groups.

## Mouse events

Extend the record returned by `canvas::pollMouse`:

```basic
TYPE canvas::MouseEvent
  kind       AS canvas::MouseKind
  button     AS canvas::MouseButton         ' button that caused Down or Up
  buttons    AS List OF canvas::MouseButton ' buttons held after the event
  position   AS vector::Float2
  wheelDelta AS vector::Float2
  timeNanos  AS Integer                     ' datetime::monotonicNanos() time base
  shift      AS Boolean
  ctrl       AS Boolean
  alt        AS Boolean
  meta       AS Boolean
  leftID     AS String
  enterID    AS String
  overID     AS String
END TYPE
```

Open decisions:

- Define `wheelDelta` units, signs, horizontal scrolling, and whether the existing
  `ScrollUp`/`ScrollDown` kinds remain.
- Are `leftID` and `enterID` transitions computed from the scene when the event was
  captured or from the latest scene when polled? Define behavior when the scene changes
  without pointer movement and when movement goes directly from one item to another.
- Decide whether explicit `Enter` and `Leave` kinds are clearer than transition fields.
- `leftID`, `enterID`, and `overID` use the same group-qualified ID format.

## Mouse pointer

```basic
canvas::setMouse(image AS canvas::Image, hotspot AS vector::Float2) AS Nothing
canvas::resetMouse() AS Nothing
```

`setMouse` installs a custom pointer image; `resetMouse` restores the system default.
Other system pointer kinds are intentionally omitted because they are inconsistent
across targets.

Open decisions:

- Define hotspot bounds, maximum image size, DPI scaling, alpha, and animation.
- Define what happens when the active image is closed and when Canvas mode exits.
- Define errors for a closed image, invalid hotspot, or use outside Canvas mode.
- Consider `setCursor`/`resetCursor`; those names describe the changed object more clearly.

## 3D object in a 2D scene

```basic
TYPE canvas::Mesh
  vertices  AS List OF vector::Float3
  triangles AS List OF vector::Integer3
  uv        AS List OF vector::Float2
  normals   AS List OF vector::Float3
  colors    AS List OF color::Color
END TYPE

UNION canvas::Texture
  canvas::Image
  Nothing
END UNION

TYPE canvas::Object3D
  id         AS String
  position   AS vector::Float2
  mesh       AS canvas::Mesh
  texture    AS canvas::Texture
  transform  AS canvas::Transform
END TYPE
```

This is a 3D mesh embedded in the ordered 2D draw list, not a general 3D world. The
mesh still has model-space Z; a projection is required to map it into the 2D scene.

Required before planning:

- Choose orthographic or perspective projection, view direction, and how 3D rotation
  and scale are represented. A 2D `canvas::Transform` cannot provide these alone.
- Define winding, culling, depth testing, triangle ordering, and transparency.
- Define flat-light direction, ambient light, and how texture and vertex color combine.
- Define UV sampling and validation of indices and per-vertex list lengths.
- Decide whether `Mesh` is a copied value or a resource, and define texture lifetime.
- Decide whether `Object3D` is a `DrawItem` and how its `id` participates in hit testing.
