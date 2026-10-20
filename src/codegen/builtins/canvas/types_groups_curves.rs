use super::*;

/// plan-176-S: groups and curve shapes — `Group`, `Ellipse`, `PolyLine`, the Bézier and NURBS curves, and
/// `DrawLayer`. Registered by `canvas::register`, in this
/// position (registration order is render order).
pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_record(RegistryRecord {
        name: "Group",
        export: true,
        description: "A reference to a sub-scene installed under a name by \
                      `canvas::setGroup`, drawn with its own origin at `position` and \
                      turned, scaled or sheared about that point by `transform`. A \
                      group inside a group is placed by both, the outer one last, so \
                      a figure built from groups moves and turns as one thing. \
                      Presenting a scene copies this node — an `id`, a position, a \
                      transform and a name — rather than the items it stands for, which is the \
                      point: a large static sub-picture referenced from many scenes \
                      is copied once when you install it, not once per frame. A name \
                      with no group installed draws nothing and does not raise, so a \
                      scene can reference a group that has not been built yet. A \
                      group has no `paint`: it is a container, and the items inside \
                      it carry their own. Every field \
                      has a usable default, so the short way to make one is to \
                      declare it and set what you need: `MUT panel AS canvas::Group`, \
                      then `WITH panel { name := \"panel\", position := \
                      vector::Float2[x := 40.0, y := 40.0] }`. Its `id` names this \
                      USE of the group, not the group: draw one group twice under \
                      two `id`s and `canvas::MouseEvent.overID` says which of the \
                      two the mouse is over, by putting the `id` in front of the \
                      item's own.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Where the group's own origin lands, in pixels: an item \
                              the group holds at `(0, 0)` is drawn here. The transform \
                              turns the group about it.",
            },
            RecordProp {
                name: "transform",
                ty: ParameterType::named("Transform"),
                description: "Turns, scales or shears everything the group holds, \
                              about `position`; its `tx` and `ty` add to `position`. \
                              It scales the items' strokes with them and carries \
                              their clips and gradients. All zero, or a transform \
                              with no area, leaves the group as it was built.",
            },
            RecordProp {
                name: "name",
                ty: ParameterType::String,
                description: "Which installed group to draw. A name you have not \
                              passed to `canvas::setGroup`, or one you have since \
                              removed, draws nothing.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Ellipse",
        export: true,
        description: "An ellipse given by its centre and two radii, along the X and Y \
                      axes. `radiusX = radiusY` is a circle, and draws identically to \
                      `canvas::Circle` of that radius — so an animation that squashes \
                      a circle can use one item type throughout. To tilt it, give it \
                      a rotation in `transform`. A radius of `0.0` or less draws \
                      nothing, the same rule `canvas::Circle` follows. Like a circle, it is \
                      shortest made from its default: `MUT egg AS canvas::Ellipse`, then \
                      `WITH egg { position := …, radiusX := …, radiusY := …, paint := … }`.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The centre, in pixels. The transform turns the ellipse about it.",
            },
            RecordProp {
                name: "radiusX",
                ty: ParameterType::Float,
                description: "The radius along the X axis, in pixels, before `transform` \
                              turns it.",
            },
            RecordProp {
                name: "radiusY",
                ty: ParameterType::Float,
                description: "The radius along the Y axis, in pixels.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    // plan-154-A. Declared after `Ellipse` and appended LAST to `DrawItem` below, for
    // the tag-order reason given there.
    pkg.add_record(RegistryRecord {
        name: "PolyLine",
        export: true,
        description: "An open path through a list of points, stroked as one item. \
                      `paint.join` shapes every corner where two segments meet, and \
                      `paint.cap` the two ends. Give `colors` a colour per point and the \
                      stroke blends from each point's colour to the next along the \
                      segment between them, so a trail that fades out is one item \
                      whose colours' alpha falls to zero, not one `canvas::Line` per \
                      segment. Colours mix in linear light, as a `canvas::Gradient`'s \
                      do. A path has no interior, so it is drawn from \
                      `paint.strokeWidth` and ignores `paint.fill` and \
                      `paint.gradient`. Fewer than two points draws nothing. Every \
                      point is measured from `position`, so moving the path is \
                      changing `position` alone, and its `transform` turns it about \
                      `position`. Every field has a usable default, so the short way \
                      to make one is to declare it and set what you need: `MUT trail \
                      AS canvas::PolyLine`, then `WITH trail { points := \
                      [vector::Float2[x := 20.0, y := 20.0], vector::Float2[x := \
                      120.0, y := 60.0], vector::Float2[x := 220.0, y := 20.0]], \
                      paint := canvas::stroke(ink, 3.0) }`.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The point every point of the path is measured from, in \
                              pixels. The transform turns the path about it.",
            },
            RecordProp {
                name: "points",
                ty: ParameterType::list_of(ParameterType::named(
                    crate::codegen::builtins::vector::FLOAT2_TYPE_ID,
                )),
                description: "The points in order, each an offset from \
                              `position`. The path does not close — use \
                              `canvas::Polygon` for a closed shape. A point repeated \
                              in place adds nothing; if every point is the same one, \
                              the path draws what a zero-length `canvas::Line` with \
                              the same `paint.cap` draws.",
            },
            RecordProp {
                name: "colors",
                ty: ParameterType::list_of(ParameterType::named(
                    crate::codegen::builtins::color::COLOR_TYPE_ID,
                )),
                description: "The stroke colour at each point, in the same order as \
                              `points` — `color::Color` values, so the program needs \
                              `IMPORT color` to name them. An empty list draws the \
                              whole path in `paint.stroke`. A point past the end of \
                              this list takes `paint.stroke`, and colours past the \
                              last point are ignored. With any colours given, the path \
                              draws even when `paint.stroke` is transparent.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    // plan-164-A. One curve segment: a record, never a `DrawItem`. The two shapes
    // below hold lists of them.
    pkg.add_record(RegistryRecord {
        name: "BezierCurve",
        export: true,
        description: "One Bézier curve segment: a list of control points, and \
                      optionally a weight for each. It is not drawn by itself — a \
                      `canvas::PolyCurve` strokes a chain of them and a \
                      `canvas::CurvePolygon` fills closed contours of them. The \
                      number of points sets the kind: two is a straight line, three \
                      a quadratic curve, four a cubic, and so on up to seventeen. \
                      The curve starts at its first point and ends at its last; the \
                      points between pull it towards them without being touched. \
                      With no weights it is an ordinary polynomial curve. With one \
                      weight per point it is a rational curve, which is what draws \
                      an exact circle or ellipse arc: a quarter circle is three \
                      points — start, corner, end — with weights `1.0`, `0.7071067811865476` \
                      and `1.0`. A curve that breaks a rule here is skipped: it \
                      draws nothing and the curves either side of it are joined by \
                      a straight segment. `canvas::nurbsToBeziers` makes a list of \
                      these from a NURBS curve, and `canvas::flatten` turns one \
                      into points.",
        props: vec![
            RecordProp {
                name: "points",
                ty: ParameterType::list_of(ParameterType::named(
                    crate::codegen::builtins::vector::FLOAT2_TYPE_ID,
                )),
                description: "The control points in order, each an offset from the \
                              `position` of the shape that holds the curve. Two to \
                              seventeen of them; fewer or more and the curve is \
                              skipped.",
            },
            RecordProp {
                name: "weights",
                ty: ParameterType::list_of(ParameterType::Float),
                description: "Empty for a polynomial curve. Otherwise exactly one \
                              weight per point, in the same order, each greater than \
                              `0.0`: a larger weight pulls the curve harder towards \
                              its point. A list of any other length, or one holding \
                              a weight of `0.0` or less, makes the curve skipped.",
            },
        ],
    });

    // plan-164-A. The curved `PolyLine`. Appended to `Shape` below, after `PolyLine`.
    pkg.add_record(RegistryRecord {
        name: "PolyCurve",
        export: true,
        description: "An open path made of curves joined end to end, stroked as one \
                      item: the curved counterpart of `canvas::PolyLine`. Each \
                      `canvas::BezierCurve` in `curves` starts where the one before \
                      it ended; when it does not, a straight segment joins the two \
                      and is stroked with the rest. `paint.join` shapes each corner \
                      where two curves meet at an angle, `paint.cap` the two ends, \
                      and `paint.dashes` run along the whole path without restarting \
                      at a curve. A path has no interior, so it is drawn from \
                      `paint.strokeWidth` and ignores `paint.fill`. It is never \
                      closed: a path that ends where it began has two caps there, \
                      not a corner — use `canvas::CurvePolygon` for a closed \
                      outline. A curve that breaks a `canvas::BezierCurve` rule is \
                      skipped, and a path with no usable curve draws nothing. The \
                      curves stay smooth however far the shape's own `transform` \
                      enlarges them. Enlarging a `canvas::Group` the path is in \
                      does not redraw the curves at the new size, so a cubic or \
                      higher curve can show slight flats there; scale the shape's \
                      own `transform` to zoom in. Every field has a usable default, \
                      so the short way to make one is to declare it and set what \
                      you need: `MUT wave AS canvas::PolyCurve`, then `WITH wave { \
                      curves := […], paint := canvas::stroke(ink, 3.0) }`.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The point every curve point is measured from, in \
                              pixels. The transform turns the path about it.",
            },
            RecordProp {
                name: "curves",
                ty: ParameterType::list_of(ParameterType::named("BezierCurve")),
                description: "The curves in order, joined end to end. An empty list \
                              draws nothing.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    // plan-164-A. The curved `Polygon`: its `contours` and `rule` are `Polygon`'s
    // fields with curves where it has points, on purpose.
    pkg.add_record(RegistryRecord {
        name: "CurvePolygon",
        export: true,
        description: "A closed shape whose outline is made of curves, filled and \
                      stroked as one item: the curved counterpart of \
                      `canvas::Polygon`. Each entry of `contours` is one closed \
                      outline, a list of `canvas::BezierCurve` joined end to end. \
                      Where a curve does not start at the end of the one before, a \
                      straight segment joins them, and a contour whose last curve \
                      does not end at its first curve's start is closed by a \
                      straight segment too; those segments are part of the outline, \
                      filled to and stroked. Several contours make holes and \
                      islands, decided by `rule` exactly as for a polygon. \
                      `paint.join` shapes each corner where two curves meet at an \
                      angle, and `paint.dashes` restart at each contour. A curve \
                      that breaks a `canvas::BezierCurve` rule is skipped, and a \
                      contour with no usable curve adds nothing. The curves stay \
                      smooth however far the shape's own `transform` enlarges \
                      them; enlarging a `canvas::Group` the shape is in does not \
                      redraw them at the new size, so scale the shape's own \
                      `transform` to zoom in. Every field has a usable default, so \
                      the short way to make one is to declare it and set what you \
                      need: `MUT blob AS canvas::CurvePolygon`, then `WITH blob { \
                      contours := [[…]], paint := canvas::fill(ink) }`.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The point every curve point is measured from, in \
                              pixels. The transform turns the shape about it.",
            },
            RecordProp {
                name: "contours",
                ty: ParameterType::list_of(ParameterType::list_of(ParameterType::named(
                    "BezierCurve",
                ))),
                description: "The closed outlines, each a list of curves in order. \
                              An empty list draws nothing.",
            },
            RecordProp {
                name: "rule",
                ty: ParameterType::named("FillRule"),
                description: "Which regions count as inside when contours overlap \
                              or nest, as for `canvas::Polygon`.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "DrawLayer",
        export: true,
        description: "An ordered group of items composited as one layer. Layers \
                      given to `canvas::presentLayers` composite in order, and a \
                      layer whose contents did not change reuses its cached \
                      geometry wholesale.",
        props: vec![RecordProp {
            name: "items",
            ty: ParameterType::list_of(ParameterType::named("DrawItem")),
            description: "The layer's items, drawn in order.",
        }],
    });
}
