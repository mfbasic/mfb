//! `canvas::flatten` — points along a Bézier curve, within a tolerance (plan-164-A).
//!
//! The canvas does not flatten a curve to draw it; this is for a program that wants the
//! points — to build a `Polygon` or a `PolyLine` of its own, or to measure with. The
//! work is `helper_curves.rs:__canvas_cvFlatten`: de Casteljau halving until every
//! control point of a span is within the tolerance of its chord. That and this
//! function's own body are `helper_curves.rs:CURVE_FUNCS`, a chunk only a program that
//! calls this (or `canvas::nurbsToBeziers`) carries.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Turn a `canvas::BezierCurve` into a list of points along it."#;

const DESC: &str = r#"`flatten` walks a curve and returns points on it, close enough together that the
straight lines between them never stray further than `tolerance` from the curve.
The first point returned is the curve's first point and the last is its last, so
the points of one curve followed by the points of the next join up.

A flat stretch gets few points and a tight bend many: the count follows the curve,
not a fixed step. A straight curve — two points — returns just its two ends.

`tolerance` is a distance in the same units as the curve's points, pixels when you
draw them unscaled. A smaller tolerance returns more points. Anything below `0.001`
is treated as `0.001`.

The points are in the curve's own space, the same one its control points are in. So
they are ready to be a contour of a `canvas::Polygon`, or the `points` of a
`canvas::PolyLine`, at the same `position` as the shape the curve came from.

A curve that breaks a `canvas::BezierCurve` rule — fewer than two points or more
than seventeen, a weight list of the wrong length, a weight of `0.0` or less —
returns an empty list and does not raise.

You do not need `flatten` to draw a curve: `canvas::PolyCurve` and
`canvas::CurvePolygon` draw curves as curves, and stay smooth when enlarged. Reach
for it when you want the points themselves."#;

const EX: &str = r#"A leaf: two cubic curves, flattened and joined into one polygon contour. The second
curve's first point is the first curve's last, so it is dropped when the two lists
are joined:

```
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT vector
IMPORT color

FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET up AS canvas::BezierCurve = canvas::BezierCurve[points := [at(0.0, 0.0), at(60.0, 0.0 - 120.0), at(200.0, 0.0 - 120.0), at(260.0, 0.0)], weights := []]
  LET down AS canvas::BezierCurve = canvas::BezierCurve[points := [at(260.0, 0.0), at(200.0, 120.0), at(60.0, 120.0), at(0.0, 0.0)], weights := []]
  MUT outline AS List OF vector::Float2 = canvas::flatten(up, 0.25)
  LET back AS List OF vector::Float2 = canvas::flatten(down, 0.25)
  MUT i AS Integer = 1
  WHILE i < len(back)
    outline = collections::append(outline, collections::getOr(back, i, at(0.0, 0.0)))
    i = i + 1
  END WHILE
  MUT leaf AS canvas::Polygon
  leaf = WITH leaf { position := at(120.0, 240.0), contours := [outline], paint := canvas::fill(color::rgb(60, 160, 80)) }
  LET item AS canvas::DrawItem = leaf
  canvas::present([item])
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_flatten(curve AS BezierCurve, tolerance AS Float) AS List OF vector::Float2
  RETURN __canvas_flattenImpl(curve, tolerance)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "flatten",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "curve",
                    desc: "The curve to walk. One that breaks a `canvas::BezierCurve` \
                           rule returns an empty list.",
                    aliases: &[],
                    ty: ParameterType::named("BezierCurve"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "tolerance",
                    desc: "The furthest the straight lines between the returned points \
                           may be from the curve, in the units of the curve's points. \
                           Smaller returns more points; below `0.001` — zero and \
                           negative included — is treated as `0.001`.",
                    aliases: &[],
                    ty: ParameterType::Float,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::named(
                crate::codegen::builtins::vector::FLOAT2_TYPE_ID,
            )),
            errors: vec![],
            body: Body::mfb(BODY, "__canvas_flatten"),
        }],
    });
}
