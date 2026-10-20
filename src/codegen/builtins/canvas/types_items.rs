use super::*;

/// plan-176-S: the closed `DrawItem` variant set and its shape unions. Registered by `canvas::register`, in this
/// position (registration order is render order).
pub(super) fn register(pkg: &mut RegistryPackage) {
    // ---- The `DrawItem` variants (a CLOSED set) ---------------------------

    pkg.add_enum(RegistryEnum {
        name: "BindTo",
        export: true,
        variants: vec![
            EnumVariant {
                name: "TopLeft",
                description: "`position` is the rectangle's top-left corner. The zero \
                              value.",
                advisory: None,
            },
            EnumVariant {
                name: "Center",
                description: "`position` is the rectangle's centre, so a rotation \
                              spins it in place.",
                advisory: None,
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Rectangle",
        export: true,
        description: "A rectangle, with a radius for each corner. `position` places \
                      it by the point `bind` names — its top-left corner, or its \
                      centre — and its `transform` turns it about that same point: a \
                      `Center` rectangle spins in place, a `TopLeft` one swings about \
                      its corner. Every field has a usable default, so the short way to \
                      make one is to declare it and set what you need: \
                      `MUT box AS canvas::Rectangle`, then `WITH box { position := \
                      vector::Float2[x := 20.0, y := 20.0], size := canvas::Size[width \
                      := 120.0, height := 80.0], paint := canvas::fill(ink) }` — it \
                      binds top-left, has four square corners, no `id` and the \
                      identity transform.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Where the rectangle is, in pixels: the point `bind` \
                              names. The transform turns the rectangle about it.",
            },
            RecordProp {
                name: "bind",
                ty: ParameterType::named("BindTo"),
                description: "Which point of the rectangle `position` is.",
            },
            RecordProp {
                name: "size",
                ty: ParameterType::named("Size"),
                description: "The width and height in pixels. A rectangle with no \
                              width or no height draws nothing.",
            },
            RecordProp {
                name: "radiusTL",
                ty: ParameterType::Float,
                description: "The top-left corner's radius. `0.0` or less is a square corner. Above `0.0` through \
                              `1.0` it is a fraction of the half-size, so the corner is \
                              an ellipse quadrant: `radiusTL * width / 2` across and \
                              `radiusTL * height / 2` down. Above `1.0` it is pixels, and \
                              the corner is circular, no larger than half the shorter \
                              side. `1.0` is the full pill and `1.01` is a one-pixel \
                              corner, so a radius animated in pixels starts above `1.0`.",
            },
            RecordProp {
                name: "radiusTR",
                ty: ParameterType::Float,
                description: "The top-right corner's radius. See `radiusTL` for how its size chooses its meaning.",
            },
            RecordProp {
                name: "radiusBL",
                ty: ParameterType::Float,
                description: "The bottom-left corner's radius. See `radiusTL` for how its size chooses its meaning.",
            },
            RecordProp {
                name: "radiusBR",
                ty: ParameterType::Float,
                description: "The bottom-right corner's radius. See `radiusTL` for how its size chooses its meaning.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Line",
        export: true,
        description: "A straight segment from one point to another. Its two ends, \
                      `startAt` and `endAt`, are measured from `position`, so moving \
                      the line is changing `position` alone, and its `transform` turns \
                      it about `position` — put `position` at the middle and the line \
                      spins in place. A line has no interior, so it is drawn from \
                      `paint.stroke`/`paint.strokeWidth` and ignores `paint.fill`; \
                      `paint.cap` shapes its two ends. Every field has a usable \
                      default, so the short way to make one is to declare it and set \
                      what you need: `MUT rule AS canvas::Line`, then `WITH rule { \
                      startAt := vector::Float2[x := 20.0, y := 40.0], endAt := \
                      vector::Float2[x := 220.0, y := 40.0], paint := \
                      canvas::stroke(ink, 2.0) }` — `position` stays at the surface's \
                      top-left corner, so the two ends are surface pixels.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The point the two ends are measured from, in pixels. \
                              The transform turns the line about it.",
            },
            RecordProp {
                name: "startAt",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Where the line starts, as an offset from `position`.",
            },
            RecordProp {
                name: "endAt",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Where the line ends, as an offset from `position`.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Polygon",
        export: true,
        description: "A filled shape bounded by one or more closed rings of \
                      points. Several rings make one shape: a ring inside another \
                      can be a hole (an \"O\", a donut) or more of the fill, as \
                      `rule` decides. The stroke outlines every ring. A ring of \
                      fewer than three points has no area and adds only its \
                      stroke; a ring of fewer than two adds nothing. Every point is \
                      measured from `position`, so moving the shape is changing \
                      `position` alone, and its `transform` turns it about \
                      `position`. Every field has a usable default, so the short way \
                      to make one is to declare it and set what you need: `MUT wedge \
                      AS canvas::Polygon`, then `WITH wedge { position := \
                      vector::Float2[x := 120.0, y := 80.0], contours := [[ \
                      vector::Float2[x := 0.0, y := 0.0 - 40.0], vector::Float2[x := \
                      40.0, y := 40.0], vector::Float2[x := 0.0 - 40.0, y := 40.0]]], \
                      paint := canvas::fill(ink) }` — a triangle around 120, 80.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The point every vertex is measured from, in pixels. \
                              The transform turns the shape about it.",
            },
            RecordProp {
                name: "contours",
                ty: ParameterType::list_of(ParameterType::list_of(ParameterType::named(
                    crate::codegen::builtins::vector::FLOAT2_TYPE_ID,
                ))),
                description: "The rings, each a list of vertices in order, each \
                              vertex an offset from `position`. Every \
                              ring closes from its own last point back to its own \
                              first point automatically. The direction a ring runs \
                              in matters only under `FillRule.NonZero`.",
            },
            RecordProp {
                name: "rule",
                ty: ParameterType::named("FillRule"),
                description: "Which points the rings enclose: `EvenOdd` makes a ring \
                              inside another a hole whichever way it runs, and \
                              `NonZero` makes it a hole only when it runs the \
                              opposite way to the ring around it.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Circle",
        export: true,
        description: "A circle given by its centre and radius. Every field has a usable \
                      default, so the short way to make one is to declare it and set \
                      what you need: `MUT dot AS canvas::Circle`, then \
                      `WITH dot { position := vector::Float2[x := 80.0, y := 60.0], \
                      radius := 20.0, paint := canvas::fill(ink) }`. `id` stays empty \
                      and `transform` stays the identity.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The centre, in pixels. The transform turns the circle about it.",
            },
            RecordProp {
                name: "radius",
                ty: ParameterType::Float,
                description: "The radius in pixels.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Arc",
        export: true,
        description: "A circular arc — the part of a circle's outline between two \
                      angles. Angles are in **radians, measured clockwise from +X**; \
                      because Y increases downward, a `0.0`..`PI` arc sweeps below \
                      its centre (so that is the smile, not the frown). An arc has \
                      no interior, so it is drawn from `paint.stroke`; `paint.cap` \
                      shapes its two ends. It is shortest made from its default: \
                      `MUT bow AS canvas::Arc`, then `WITH bow { position := …, \
                      radius := …, startAngle := …, endAngle := …, paint := … }`.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description:
                    "The centre of the circle the arc lies on, in pixels. The transform turns \
                              the arc about it.",
            },
            RecordProp {
                name: "radius",
                ty: ParameterType::Float,
                description: "The radius in pixels.",
            },
            RecordProp {
                name: "startAngle",
                ty: ParameterType::Float,
                description: "Where the arc begins, in radians clockwise from +X.",
            },
            RecordProp {
                name: "endAngle",
                ty: ParameterType::Float,
                description: "Where the arc ends, in radians clockwise from +X.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Text",
        export: true,
        description: "A run of text drawn at a size in a list of fonts. Each \
                      character is drawn in the first font in `fonts` that has it, so \
                      a line mixing scripts draws each from a font that covers it, and \
                      `canvas::measureText` over the same list measures what is drawn. \
                      The fonts' pair kerning and standard ligatures (`fi`, `fl`) \
                      are applied, the ligatures only when `letterSpacing` is \
                      `0.0`. Glyph outlines come from the fonts' TrueType `glyf` \
                      tables only: \
                      colour and bitmap glyphs (most emoji fonts) draw nothing. Arabic, \
                      Indic and other scripts that need complex shaping draw one glyph \
                      per character in reading order, unjoined; a font is chosen per \
                      character, not per cluster. `position` is the start of the \
                      baseline, not the top-left corner — use `canvas::measureText` to \
                      lay text out without drawing it.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description:
                    "Where the baseline starts, in pixels: its left end. The transform turns \
                              the text about it.",
            },
            RecordProp {
                name: "text",
                ty: ParameterType::String,
                description: "The text to draw.",
            },
            RecordProp {
                name: "fonts",
                ty: ParameterType::list_of(ParameterType::res(ParameterType::named("canvas.Font"))),
                description: "The fonts to draw it in, in fallback order: each \
                              character is drawn in the first font in the list that \
                              has it, so `[latinFace, cjkFace]` draws Latin text from \
                              the first and CJK from the second. One font is a list of \
                              one, `[face]`. The item holds the fonts themselves — you \
                              still close them, and a closed font in the list is \
                              passed over; with none open the text draws nothing \
                              rather than failing.",
            },
            RecordProp {
                name: "fontSize",
                ty: ParameterType::Float,
                description: "The em size in pixels.",
            },
            RecordProp {
                name: "letterSpacing",
                ty: ParameterType::Float,
                description: "Extra space in pixels after every character but the \
                              last, where a character is what a reader sees as one: \
                              a letter with its accents, a whole emoji sequence. \
                              `0.0`, the zero value, adds none; a negative value draws \
                              characters closer. Any other value turns the font's \
                              ligatures off. `canvas::measureText` takes the same \
                              spacing, so it measures what is drawn.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    // plan-172-L: how a picture's image is placed in its shape.
    pkg.add_enum(RegistryEnum {
        name: "PictureClip",
        export: true,
        variants: vec![
            EnumVariant {
                name: "ClipOnly",
                description: "The shape only cuts the image out. The image stays \
                              upright however the shape is turned, fitted to the box \
                              around the shape as it sits on the canvas. The zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "ClipAndTransform",
                description: "The image is fitted to the shape as it was built, and \
                              the shape's transform carries the image with it: turn \
                              the shape and the image turns too.",
                advisory: None,
            },
        ],
    });
    pkg.add_enum(RegistryEnum {
        name: "PictureFit",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Stretch",
                description: "The image is scaled to the box around the shape, each \
                              way separately, so it fills it and may be squashed. The \
                              zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "Contain",
                description: "The image keeps its proportions and is as large as fits \
                              inside the box, centred. The rest of the shape shows \
                              nothing.",
                advisory: None,
            },
            EnumVariant {
                name: "Cover",
                description: "The image keeps its proportions and is as small as \
                              covers the whole box, centred. What spills past the \
                              shape is cut off.",
                advisory: None,
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Tile",
        export: true,
        description: "A rectangle of an image drawn into a rectangle on the canvas. \
                      `srcPosition` and `srcSize` select image pixels; `position` and \
                      `size` place them in canvas coordinates. Scaling uses the nearest \
                      image pixel. A sample outside the image is transparent, without \
                      moving or stretching the part that is inside. A nonpositive \
                      source or destination size draws nothing. A tile has no paint or \
                      transform of its own; put it in a `canvas::Group` to move, turn, \
                      scale or shear a map. A tile is never under the mouse: it has \
                      no `id`, `canvas::MouseEvent.overID` never names it, and it \
                      does not hide what is beneath it from the mouse. A `Tile` has \
                      no default value because it holds an image, so every field is \
                      written out.",
        props: vec![
            RecordProp {
                name: "image",
                ty: ParameterType::res(ParameterType::named("canvas.Image")),
                description: "The image whose pixels are sampled.",
            },
            RecordProp {
                name: "srcPosition",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The top-left of the requested source rectangle, in image pixels. Fractional coordinates are allowed.",
            },
            RecordProp {
                name: "srcSize",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The width and height of the requested source rectangle, in image pixels. A nonpositive value draws nothing.",
            },
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The top-left of the destination rectangle, in canvas coordinates.",
            },
            RecordProp {
                name: "size",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The width and height of the destination rectangle, in canvas coordinates. A nonpositive value draws nothing.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Picture",
        export: true,
        description: "An image drawn inside a shape. The shape says where the picture \
                      is and what outline it is cut to: a `canvas::Rectangle`, with \
                      square or rounded corners, a `canvas::Circle`, a \
                      `canvas::Ellipse`, a `canvas::Polygon` or a \
                      `canvas::CurvePolygon`. A shape with no inside — a \
                      `canvas::Line`, `canvas::Arc`, `canvas::PolyLine`, \
                      `canvas::PolyCurve` or `canvas::Text` — draws nothing. The image is visible only inside \
                      the shape. Scaling picks the nearest image pixel, with no \
                      smoothing, so a small image drawn large shows crisp blocks. The \
                      shape's paint is the picture's: its fill colour tints the image \
                      — each channel moves towards the image's channel multiplied by \
                      the fill's, by the fill's alpha, so the zero paint and a white \
                      fill draw it unchanged, an opaque red fill keeps only its red, \
                      and a half-transparent red fill goes halfway — and never fills \
                      the shape itself; `canvas::Paint.transparency` fades the \
                      picture; its blend mode composites it; its stroke outlines the \
                      shape on top of the image; and the shape's `paint.clip`, if it \
                      has one, cuts the image further, so it shows only where the \
                      shape and that clip both cover. A fill gradient is ignored. \
                      The image is fitted to the box around the shape — the smallest \
                      upright rectangle that holds it — as `fit` says, and `clip` \
                      says which box when the shape has a `transform`. A group the \
                      picture is in moves and turns the whole picture, image and \
                      all, whatever `clip` is. When the shape is a \
                      `canvas::AutoPaint`, the paint the mouse selects is the one \
                      used, and nothing else is needed for that to work — not \
                      `canvas::enableMouse`. `canvas::MouseEvent.overID` reports the \
                      picture's own `id` in front of its shape's. A `Picture` has no \
                      default value — it holds an image — so every field is \
                      written out.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "shape",
                ty: ParameterType::named("AutoShape"),
                description: "The shape the image is drawn inside, and whose paint \
                              tints, fades, blends and outlines it.",
            },
            RecordProp {
                name: "clip",
                ty: ParameterType::named("PictureClip"),
                description: "Whether the shape's transform turns the image with it \
                              or only the outline it is cut to.",
            },
            RecordProp {
                name: "fit",
                ty: ParameterType::named("PictureFit"),
                description: "How the image is scaled to the shape: stretched to \
                              fill it, kept whole inside it, or made to cover it.",
            },
            RecordProp {
                name: "image",
                ty: ParameterType::res(ParameterType::named("canvas.Image")),
                description: "The image to draw. The item holds the image \
                              itself — you still close it, and closing it while \
                              a scene still names it draws nothing rather than \
                              failing.",
            },
        ],
    });

    // plan-172-K: three unions, each including the one before it. `Shape` is what can
    // be drawn with a paint of its own; `AutoShape` adds the one thing that DECORATES a
    // shape; `DrawItem` adds the two things that are not shapes at all. An included
    // union's members come first, in its own order, so the three agree on every tag
    // they share: Rectangle 0, Line 1, Polygon 2, Circle 3, Arc 4, Text 5, Ellipse 6,
    // PolyLine 7, PolyCurve 8, CurvePolygon 9, AutoPaint 10, Picture 11, Group 12.
    //
    // plan-150-A appended `ParticleSystem` to `DrawItem`'s own variants, at 13. No tag
    // moved.
    //
    // plan-164-A appended the two curve shapes to `Shape`, which moved `AutoPaint`,
    // `Picture` and `Group` by two (owner-approved, 2026-10-11): a shape has to be a
    // member of `Shape` for an `AutoPaint` to wrap it and a `Picture` to be cut to it.
    //
    // The order is `planning/new_canvas.md`'s and it is load-bearing once, here: a
    // variant's tag is its position, and the native walks ask the type model for a tag
    // by name (`func_geo_build::variant`), never by number.
    pkg.add_union(RegistryUnion {
        includes: vec![],
        name: "Shape",
        export: true,
        variants: vec![
            UnionVariant {
                name: "Rectangle",
                description: "A rectangle, with a radius for each corner.",
            },
            UnionVariant {
                name: "Line",
                description: "A straight segment.",
            },
            UnionVariant {
                name: "Polygon",
                description: "A closed polygon.",
            },
            UnionVariant {
                name: "Circle",
                description: "A circle.",
            },
            UnionVariant {
                name: "Arc",
                description: "A circular arc.",
            },
            UnionVariant {
                name: "Text",
                description: "A run of text.",
            },
            UnionVariant {
                name: "Ellipse",
                description: "An ellipse.",
            },
            UnionVariant {
                name: "PolyLine",
                description: "An open path, stroked, with a colour per point.",
            },
            UnionVariant {
                name: "PolyCurve",
                description: "An open path of curves, stroked.",
            },
            UnionVariant {
                name: "CurvePolygon",
                description: "A closed shape with a curved outline.",
            },
        ],
    });

    pkg.add_union(RegistryUnion {
        includes: vec!["Shape"],
        name: "AutoShape",
        export: true,
        variants: vec![UnionVariant {
            name: "AutoPaint",
            description: "A shape whose paint follows the mouse.",
        }],
    });

    pkg.add_union(RegistryUnion {
        includes: vec!["AutoShape"],
        name: "DrawItem",
        export: true,
        variants: vec![
            UnionVariant {
                name: "Picture",
                description: "An image drawn inside a shape.",
            },
            // `Group` is a CONTAINER, not a shape. It draws nothing itself and carries
            // no `paint` -- see `every_draw_item_variant_carries_a_paint`, which is
            // narrowed rather than weakened to admit it.
            UnionVariant {
                name: "Group",
                description: "A reference to a named group of items, placed and turned.",
            },
            // plan-150-A: the third thing that is not a shape. Appended to `DrawItem`'s
            // own variants -- not to `Shape` or `AutoShape`, whose members come first
            // and would renumber `AutoPaint`, `Picture` and `Group` -- so it takes the
            // next tag and moves no other.
            UnionVariant {
                name: "ParticleSystem",
                description: "Many copies of one shape, placed by a rule and a clock.",
            },
            UnionVariant {
                name: "Tile",
                description: "A rectangle of an image drawn into a rectangle on the canvas.",
            },
        ],
    });
}
