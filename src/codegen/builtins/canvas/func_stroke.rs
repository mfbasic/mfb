//! `canvas::stroke` — a `Paint` that outlines an item without filling it.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Build a `canvas::Paint` that outlines an item and leaves its interior empty."#;

const DESC: &str = r#"`stroke` is the outline-only case — an unfilled shape, and the only sensible
`canvas::Paint` for the four items that have no interior at all, `canvas::Line`,
`canvas::Arc`, `canvas::PolyLine` and `canvas::PolyCurve`. Every
other field is left at its no-op value: a transparent fill, `Normal` blend,
no clip, no gradient, `Butt` ends, `Round` corners, no dashes and opaque —
with a `miterLimit` of `4.0`. A `canvas::Line` that should end round says so with
`WITH canvas::stroke(c, w) { cap := canvas::CapStyle.Round }`.

A `width` of `0.0` draws nothing, since the stroke has no thickness.

Use `canvas::fill` for the fill-only case and `canvas::fillStroke` for both. To
set blend or clip, update the result with `WITH`.

These constructors exist because MFBASIC named construction requires **every**
field — `canvas::Paint[stroke := c]` is a constructor-arity error, not a partial record —
so without them every item would have to spell out all thirteen `canvas::Paint` fields."#;

const EX: &str = r#"A smile — the lower half of a circle, stroked. Angles are radians clockwise from
+X, and Y increases downward, so `0.0`..`PI` sweeps *below* the centre:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET green AS color::Color = color::rgb(0, 160, 0)
  LET smile AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 200.0, y := 215.0], radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(green, 14.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([smile])
END FUNC
```

A trail that fades out: one `canvas::PolyLine` with a colour per point, whose alpha
falls to zero at the tail. With colours given, the path draws in them rather than in
the paint's own colour, so the paint here only sets the width:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT collections
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT points AS List OF vector::Float2 = []
  MUT colors AS List OF color::Color = []
  MUT k AS Integer = 0
  WHILE k < 8
    LET x AS Float = 60.0 + toFloat(k) * 40.0
    LET y AS Float = 200.0 + toFloat((k MOD 2) * 60)
    points = collections::append(points, vector::Float2[x := x, y := y])
    colors = collections::append(colors, color::rgba(255, 140, 40, 255 - (k * 255) / 7))
    k = k + 1
  END WHILE
  LET trail AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := points, colors := colors, paint := WITH canvas::stroke(color::rgb(255, 140, 40), 12.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([trail])
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_stroke(color AS color::Color, width AS Float) AS Paint
  RETURN __canvas_fillStroke(__canvas_transparent(), color, width)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "stroke",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "color",
                    desc: "The outline colour. A `color::Color` — the program \
                           needs `IMPORT color`.",
                    aliases: &[],
                    ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "width",
                    desc: "The outline width in pixels. `0.0` draws nothing.",
                    aliases: &[],
                    ty: ParameterType::Float,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named("Paint"),
            errors: vec![],
            body: Body::mfb(BODY, "__canvas_stroke"),
        }],
    });
}
