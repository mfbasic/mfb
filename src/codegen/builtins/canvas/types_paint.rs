use super::*;

/// plan-176-S: the paint model — `Transform`, `GradientStop`, `Gradient` and `Paint`. Registered by `canvas::register`, in this
/// position (registration order is render order).
pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_record(RegistryRecord {
        name: "Transform",
        export: true,
        description: "A 2×3 affine transform, applied as \
                      `x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`. **The all-zero \
                      value means the identity**, not the degenerate matrix that \
                      collapses every point to the origin — which is what lets an \
                      unset `transform` leave an item untransformed. \
                      A rotation by θ is `a := cos(θ), b := sin(θ), c := -sin(θ), \
                      d := cos(θ)`; a scale is `a := sx, d := sy` with `b` and `c` \
                      zero; an X shear is `c := tan(θ)`. Every shape is turned and scaled \
                      about its own `position` — for a `canvas::Rectangle`, the \
                      point its `bind` names — so a rotation spins it in place, and \
                      `tx` and `ty` move it from there. A `canvas::Group` is turned \
                      about its `position` in the same way, and everything in it \
                      turns with it.",
        props: vec![
            RecordProp {
                name: "a",
                ty: ParameterType::Float,
                description: "Row 0, column 0 — X scale.",
            },
            RecordProp {
                name: "b",
                ty: ParameterType::Float,
                description: "Row 1, column 0 — Y shear.",
            },
            RecordProp {
                name: "c",
                ty: ParameterType::Float,
                description: "Row 0, column 1 — X shear.",
            },
            RecordProp {
                name: "d",
                ty: ParameterType::Float,
                description: "Row 1, column 1 — Y scale.",
            },
            RecordProp {
                name: "tx",
                ty: ParameterType::Float,
                description: "The X translation in pixels.",
            },
            RecordProp {
                name: "ty",
                ty: ParameterType::Float,
                description: "The Y translation in pixels.",
            },
        ],
    });

    // `ImageRef` and `FontRef` used to be declared here, and the comment above them
    // explained why: a record field could not hold a resource, so the scene named one
    // through a plain `Integer` handle instead.
    //
    // plan-114-D lifted that ban and plan-116-I removed the workaround. `Picture.image`
    // and `Text.fonts` (a list since plan-162-C) now hold the **resources themselves**. What the handle bought — a
    // published scene having no opinion on the resource's lifetime — is preserved
    // rather than given up: the renderer reads the backend id through
    // `canvas::imageHandle`/`fontHandle`, which answer `0` for a destroyed resource
    // instead of raising, so an item naming something the program has since closed
    // draws nothing exactly as a zero handle did.
    //
    // What changed for a caller is that the compiler now knows a scene names a
    // resource. That is the point: `canvas::Picture[id := "", image := imageRef(img), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]` could
    // outlive `img` silently, and `image := img` cannot.

    pkg.add_record(RegistryRecord {
        name: "GradientStop",
        export: true,
        description: "One colour stop of a `canvas::Gradient`. Stops are used **in \
                      the order you give them** — they are not sorted, because \
                      reordering them would silently redraw something other than what \
                      the program asked for. An `offset` outside `0.0`..`1.0`, or one \
                      that goes backwards, is clamped instead: visible and \
                      predictable.",
        props: vec![
            RecordProp {
                name: "offset",
                ty: ParameterType::Float,
                description: "Where along the gradient this colour sits, `0.0` at the \
                              start and `1.0` at the end.",
            },
            RecordProp {
                name: "color",
                ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                description: "The colour at that offset. A `color::Color` — the \
                              program needs `IMPORT color` to name the type.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Gradient",
        export: true,
        description: "A colour ramp used as an item's interior instead of a flat \
                      `canvas::Paint.fill`, and with `strokeApply` its outline too. \
                      **Fewer than two stops is not a gradient** — the item falls \
                      back to `fill`, so the all-zero value is inert like every other \
                      `canvas::Paint` field. One stop is a flat colour you should \
                      write as `canvas::fill`, and zero stops name no colour at all. \
                      The ramp is placed in the item's own coordinates, from its own \
                      two points and never from the shape's bounds: the gradient's \
                      `transform` maps it first, then the item's `transform` and \
                      any group's, so a rotated or moved item carries its ramp \
                      with it. Colours between \
                      two stops are mixed in **linear light**, the same space \
                      everything else on the surface is blended in, so a \
                      black-to-white ramp is evenly bright across its width rather \
                      than dark for most of it — see `mfb spec app canvas`.",
        props: vec![
            RecordProp {
                name: "kind",
                ty: ParameterType::named("GradientKind"),
                description: "Linear along an axis, or radial outward from a centre.",
            },
            RecordProp {
                name: "startPoint",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Linear: where the ramp starts. Radial: the centre.",
            },
            RecordProp {
                name: "endPoint",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Linear: where the ramp ends. Radial: a point on the \
                              outer circle. Giving the same point as `startPoint` \
                              leaves the first stop's colour everywhere, rather than \
                              failing.",
            },
            RecordProp {
                name: "stops",
                ty: ParameterType::list_of(ParameterType::named("GradientStop")),
                description: "The colours along the ramp, in the order given.",
            },
            RecordProp {
                name: "spread",
                ty: ParameterType::named("GradientSpread"),
                description: "What the ramp does past its ends. `Pad`, the zero value, \
                              keeps the end colours; `Reflect` mirrors the ramp and \
                              `Repeat` starts it again.",
            },
            RecordProp {
                name: "transform",
                ty: ParameterType::named("Transform"),
                description: "Maps the ramp before the item's own \
                              an item's `transform` does: scale, turn or shear the \
                              gradient without touching the shape. Scaling a radial \
                              gradient by different amounts in x and y makes it \
                              elliptical. The all-zero value is the identity, and a \
                              transform that flattens the ramp to a line is treated as \
                              the identity, as for an item's `transform`.",
            },
            RecordProp {
                name: "focalOffset",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Radial only: where the ramp starts, as an offset from \
                              `startPoint` — an off-centre highlight on a sphere. The \
                              zero value starts it at the centre. An offset that would \
                              put the focus on or outside the outer circle is pulled \
                              just inside it.",
            },
            RecordProp {
                name: "focalRadius",
                ty: ParameterType::Float,
                description: "Radial only: the radius of the circle at the focus that \
                              the ramp starts from, so the first stop's colour fills a \
                              disc rather than a point. The zero value is a point. It \
                              is held between `0.0` and just under the outer radius.",
            },
            RecordProp {
                name: "strokeApply",
                ty: ParameterType::named("GradientApply"),
                description: "Whether the ramp colours the outline as well as the \
                              interior. `FillOnly`, the zero value, leaves the outline \
                              the flat `canvas::Paint.stroke`; `FillAndStroke` paints \
                              it with the ramp tinted by the stroke colour, as a \
                              `canvas::Picture` is tinted by its fill.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Paint",
        export: true,
        description: "How an item is filled, stroked, blended and clipped. \
                      A flat value threaded through each item — there is no \
                      ambient drawing state. Every field's zero value is that \
                      field's no-op — `miterLimit` aside, which only a `Miter` join \
                      reads — and `canvas::fill`, `canvas::stroke` and \
                      `canvas::fillStroke` leave every field they do not name at that \
                      zero, apart from a `miterLimit` of `4.0`, so `canvas::fill(c)` is \
                      a plain filled shape. Building a `canvas::Paint` by name must \
                      list every field, so start from one of those three and refine it \
                      with `WITH`.",
        props: vec![
            RecordProp {
                name: "fill",
                ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                description: "The interior colour, a `color::Color`. Transparent (the \
                              zero `color::Color`) leaves the item unfilled.",
            },
            RecordProp {
                name: "stroke",
                ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                description: "The outline colour, a `color::Color`. Transparent (the \
                              zero `color::Color`) leaves the item unstroked.",
            },
            RecordProp {
                name: "strokeWidth",
                ty: ParameterType::Float,
                description: "The outline width in pixels. `0.0` draws no outline \
                              regardless of `stroke`.",
            },
            RecordProp {
                name: "blend",
                ty: ParameterType::named("BlendMode"),
                description: "How the item composites onto what is already there. \
                              The zero value is `Normal`, ordinary source-over. \
                              Compositing happens on linear light, so `Multiply` of \
                              mid grey with itself is darker than the sRGB byte \
                              arithmetic would suggest. An item that both fills and \
                              strokes applies the mode twice — the fill onto what is \
                              beneath, then the outline onto that.",
            },
            RecordProp {
                name: "clip",
                ty: ParameterType::list_of(ParameterType::list_of(ParameterType::named(
                    crate::codegen::builtins::vector::FLOAT2_TYPE_ID,
                ))),
                description: "Restricts drawing to the area these rings enclose, \
                              under `clipRule` — each ring a closed path of points, \
                              as a `canvas::Polygon`'s `contours` are. The empty list, \
                              the zero value, means no clipping. A clip that encloses \
                              nothing, such as rings of fewer than two points, hides \
                              the whole item. The rings are in the item's own \
                              coordinates, measured from its `position`: the \
                              item's `transform` and any group's apply to the clip \
                              as well as to the item, so a clipped item moves with \
                              its clip. Its edges may \
                              fall between pixels, and a partly-covered pixel is \
                              drawn partly, exactly as a shape's own edge is.",
            },
            RecordProp {
                name: "clipRule",
                ty: ParameterType::named("FillRule"),
                description: "Which points the `clip` rings enclose, as a \
                              `canvas::Polygon`'s `rule` decides its fill: \
                              `EvenOdd`, the zero value, makes a ring inside another \
                              a hole; `NonZero` makes it a hole only when the two \
                              run opposite ways.",
            },
            RecordProp {
                name: "gradient",
                ty: ParameterType::named("Gradient"),
                description: "Fill the item with a colour ramp instead of the flat \
                              `fill`. A gradient with **fewer than two stops is \
                              ignored** and `fill` is used, which is what makes the \
                              zero value inert like every other field here. The ramp \
                              is in the item's own coordinates, so `transform` carries \
                              it: rotating an item turns its ramp with it. The outline \
                              stays the flat `stroke` colour unless the gradient's \
                              `strokeApply` is `FillAndStroke`. A `canvas::Text` item \
                              ignores this field, because a glyph is drawn from a \
                              cached coverage bitmap rather than from a distance \
                              field; a `canvas::PolyLine` takes its colours from \
                              `colors` and a `canvas::Picture` from its image.",
            },
            RecordProp {
                name: "cap",
                ty: ParameterType::named("CapStyle"),
                description: "How each open end of the stroke is shaped: the two ends \
                              of a `canvas::Line`, `canvas::Arc` or `canvas::PolyLine`, \
                              and both ends of every dash. `Butt`, the zero value, \
                              stops the stroke at the end; `Round` adds a half-disc \
                              and `Square` half a square. A closed outline with no \
                              dashes has no ends, so it ignores this.",
            },
            RecordProp {
                name: "join",
                ty: ParameterType::named("JoinStyle"),
                description: "How the stroke turns a corner: where two edges of a \
                              `canvas::Polygon` ring or a `canvas::PolyLine` meet, and \
                              at the four corners of a `canvas::Rectangle`. \
                              `Round`, the zero value, rounds the outside of the \
                              corner. Circles, ellipses, arcs and a rectangle with \
                              rounded corners have no corners and ignore it.",
            },
            RecordProp {
                name: "miterLimit",
                ty: ParameterType::Float,
                description: "How sharp a `Miter` corner may be before it is drawn as \
                              `Bevel` instead, as the longest the corner's point may \
                              reach measured in stroke widths: a corner of interior \
                              angle θ reaches `1 / sin(θ / 2)`. `canvas::fill`, \
                              `canvas::stroke` and `canvas::fillStroke` set `4.0`, \
                              which bevels corners sharper than about 29°. A value \
                              below `1.0`, the zero value included, bevels every \
                              corner.",
            },
            RecordProp {
                name: "dashes",
                ty: ParameterType::list_of(ParameterType::Float),
                description: "Alternating dash and gap lengths along the outline, in \
                              pixels before `transform`, starting with a dash. An odd \
                              number of lengths is repeated once to make it even. An \
                              empty list — the zero value — draws a solid stroke, and \
                              so does a list whose lengths add up to `0.0` or that \
                              holds a negative length. `cap` shapes both ends of \
                              every dash. The pattern runs along the outline from its \
                              start: a `canvas::Line`'s first point, a \
                              `canvas::PolyLine`'s first point, an `canvas::Arc`'s \
                              `startAngle`, each `canvas::Polygon` ring's own first \
                              point, a `canvas::Rectangle`'s top-left corner — or, when \
                              that corner is rounded, its top edge where it leaves the \
                              corner — and a `canvas::Circle`'s or `canvas::Ellipse`'s \
                              rightmost point (before its `transform`) — going clockwise \
                              round every closed shape.",
            },
            RecordProp {
                name: "dashOffset",
                ty: ParameterType::Float,
                description: "How far into the `dashes` pattern the outline starts, in \
                              pixels. It may be negative or longer than the pattern; \
                              the pattern repeats in both directions.",
            },
            RecordProp {
                name: "transparency",
                ty: ParameterType::Float,
                description: "How see-through the whole item is: `0.0`, the zero \
                              value, is opaque and `1.0` invisible, with values \
                              outside that range treated as the nearer end. It fades \
                              the fill, the stroke, a fill gradient, text and a \
                              picture alike. The fill and the stroke fade \
                              separately, so where they overlap the fill shows \
                              through a faded stroke.",
            },
        ],
    });
}
