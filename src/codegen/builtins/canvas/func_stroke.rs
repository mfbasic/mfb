//! `canvas::stroke` — a `Paint` that outlines an item without filling it.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Build a `canvas::Paint` that outlines an item and leaves its interior empty."#;

const DESC: &str = r#"`stroke` is the outline-only case — an unfilled shape, and the only sensible
`canvas::Paint` for the two items that have no interior at all, `canvas::Line` and `canvas::Arc`. Every
other field is left at its no-op value: a transparent fill, `Normal` blend, the
identity transform, and no clip.

A `width` of `0.0` draws nothing, since the stroke has no thickness.

Use `canvas::fill` for the fill-only case and `canvas::fillStroke` for both. To
set blend, transform or clip, update the result with `WITH`.

These constructors exist because MFBASIC named construction requires **every**
field — `canvas::Paint[stroke := c]` is a constructor-arity error, not a partial record —
so without them every item would have to spell out all six `canvas::Paint` fields."#;

const EX: &str = r#"A smile — the lower half of a circle, stroked. Angles are radians clockwise from
+X, and Y increases downward, so `0.0`..`PI` sweeps *below* the centre:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  LET green AS canvas::Color = canvas::rgb(0, 160, 0)
  LET smile AS canvas::DrawItem = canvas::Arc[x := 200.0, y := 215.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Butt, paint := canvas::stroke(green, 14.0)]
  canvas::present([smile])
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_stroke(color AS Color, width AS Float) AS Paint
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
                    desc: "The outline colour.",
                    aliases: &[],
                    ty: ParameterType::named("Color"),
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
