//! `canvas::nurbsToBeziers` — a NURBS curve as the chain of Bézier curves it is
//! (plan-164-A).
//!
//! The conversion is exact, by Boehm's knot insertion: every knot of the curve's domain
//! is raised to multiplicity `degree` (`helper_curves.rs:__canvas_cvRefine`), after
//! which the control points fall into runs of `degree + 1` that are each one rational
//! Bézier segment of the same degree. Nothing is approximated, which is why the canvas
//! needs no NURBS item of its own.
//!
//! The work is `helper_curves.rs:CURVE_FUNCS`, a chunk only a program that calls this
//! (or `canvas::flatten`) carries; the body here is the one call into it.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Convert a NURBS curve into the list of `canvas::BezierCurve` that draws it."#;

const DESC: &str = r#"`nurbsToBeziers` takes a NURBS curve the way a CAD file or a font tool hands it
over — control points, a weight for each, a knot vector and a degree — and returns
the same curve as Bézier segments, one for each stretch between two different
knots. The conversion is exact: the segments trace the curve itself, not an
approximation of it, and each has the degree you passed.

The result is a chain, in order, each curve starting where the one before ended. It
is ready to be a `canvas::PolyCurve`'s `curves`, or one contour of a
`canvas::CurvePolygon` when the curve closes on itself, as a NURBS circle does. The
points are in whatever space your control points were in, so give the shape the
`position` you measured them from.

`weights` may be empty, which makes every weight `1.0` (a plain B-spline); the
curves returned then have no weights either. Otherwise there must be one weight per
control point.

The curve drawn is the part of the knot vector every NURBS has a full set of
control points for: from `knots[degree]` to `knots[len(controlPoints)]`. With the
usual clamped vector — the first and last knot each repeated `degree + 1` times —
that is the whole vector, and the curve starts on the first control point and ends
on the last.

Input that does not describe a curve returns an empty list and does not raise:

- a `degree` below `1` or above `16`;
- fewer control points than `degree + 1`;
- a knot count other than `len(controlPoints) + degree + 1`;
- a knot smaller than the one before it;
- a `weights` list that is neither empty nor one per control point, or that holds a
  weight of `0.0` or less;
- a knot vector whose `knots[degree]` is not below `knots[len(controlPoints)]`,
  which leaves no curve at all.

Use `canvas::flatten` when you want points along one of the curves instead of
drawing it."#;

const EX: &str = r#"A full circle as a NURBS — nine control points round a square, degree 2 — filled as
one `canvas::CurvePolygon`. The corner points have the weight `0.7071067811865476`
(the square root of one half), which is what makes each quarter a true arc:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET r AS Float = 120.0
  LET corner AS Float = 0.7071067811865476
  LET points AS List OF vector::Float2 = [at(r, 0.0), at(r, r), at(0.0, r), at(0.0 - r, r), at(0.0 - r, 0.0), at(0.0 - r, 0.0 - r), at(0.0, 0.0 - r), at(r, 0.0 - r), at(r, 0.0)]
  LET weights AS List OF Float = [1.0, corner, 1.0, corner, 1.0, corner, 1.0, corner, 1.0]
  LET knots AS List OF Float = [0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0]
  LET arcs AS List OF canvas::BezierCurve = canvas::nurbsToBeziers(points, weights, knots, 2)
  MUT disc AS canvas::CurvePolygon
  disc = WITH disc { position := at(300.0, 240.0), contours := [arcs], paint := canvas::fill(color::rgb(40, 120, 220)) }
  LET item AS canvas::DrawItem = disc
  canvas::present([item])
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_nurbsToBeziers(controlPoints AS List OF vector::Float2, weights AS List OF Float, knots AS List OF Float, degree AS Integer) AS List OF BezierCurve
  RETURN __canvas_nurbsImpl(controlPoints, weights, knots, degree)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "nurbsToBeziers",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "controlPoints",
                    desc: "The control points in order, at least `degree + 1` of them. \
                           The returned curves' points are in the same space.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::named(
                        crate::codegen::builtins::vector::FLOAT2_TYPE_ID,
                    )),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "weights",
                    desc: "One weight per control point, each above `0.0`, or an empty \
                           list for a curve with no weights. Any other length, or a \
                           weight of `0.0` or less, returns an empty list.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::Float),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "knots",
                    desc: "The knot vector: exactly `len(controlPoints) + degree + 1` \
                           numbers, none smaller than the one before it. Repeating a \
                           knot is how a NURBS makes a corner or a clamped end.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::Float),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "degree",
                    desc: "The curve's degree, `1` to `16`: `2` for the conic arcs a \
                           NURBS circle is made of, `3` for the usual cubic spline. \
                           Outside that range returns an empty list.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::named("BezierCurve")),
            errors: vec![],
            body: Body::mfb(BODY, "__canvas_nurbsToBeziers"),
        }],
    });
}
