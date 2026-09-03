//! `canvas::fillStroke` — a `Paint` that both fills and outlines.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Build a `canvas::Paint` that both fills an item and outlines it."#;

const DESC: &str = r#"`fillStroke` builds a `canvas::Paint` with both channels set: the item's interior is
`fill` and its outline is `width` pixels of `stroke`. `canvas::fill` and
`canvas::stroke` are the one-channel forms.

Blend, transform, clip and the fill gradient are left at their no-op values
(`Normal`, the identity, unclipped, and no stops). To set one of those, update the
result:

```
LET glow AS Paint = WITH canvas::fillStroke(core, halo, 2.0) { blend := BlendMode.Add }
```

These constructors exist because MFBASIC named construction requires **every**
field — `canvas::Paint[fill := c]` is a constructor-arity error, not a partial record — so
without them every item would have to spell out all seven `canvas::Paint` fields."#;

const EX: &str = r#"A filled circle with a contrasting outline:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  LET body AS canvas::Color = canvas::rgb(255, 220, 0)
  LET edge AS canvas::Color = canvas::rgb(80, 60, 0)
  LET dot AS canvas::DrawItem = canvas::Circle[x := 100.0, y := 100.0, radius := 40.0, paint := canvas::fillStroke(body, edge, 3.0)]
  canvas::present([dot])
END SUB
```

The same paint turned 45°. Give the item coordinates around `(0, 0)` and put its
position in `tx` and `ty`, because a rotation turns about the item's own origin —
the diamond below is centred at 220, 120. The two squares are drawn with one paint
apart from the transform, so the difference in the picture is the transform's alone:

```
IMPORT app
IMPORT canvas
IMPORT math

SUB main()
  app::setMode(app::Mode.Canvas)
  LET body AS canvas::Color = canvas::rgb(120, 190, 255)
  LET edge AS canvas::Color = canvas::rgb(20, 40, 90)
  LET look AS canvas::Paint = canvas::fillStroke(body, edge, 4.0)

  LET turn AS Float = math::pi / 4.0
  LET spin AS canvas::Transform = canvas::Transform[a := math::cos(turn), b := math::sin(turn), c := 0.0 - math::sin(turn), d := math::cos(turn), tx := 220.0, ty := 120.0]

  LET upright AS canvas::DrawItem = canvas::Rectangle[x := 40.0, y := 80.0, w := 80.0, h := 80.0, paint := look]
  LET diamond AS canvas::DrawItem = canvas::Rectangle[x := 0.0 - 40.0, y := 0.0 - 40.0, w := 80.0, h := 80.0, paint := WITH look { transform := spin }]

  canvas::present([upright, diamond])
END SUB
```

A rotation keeps lengths, so both outlines come out 4 pixels wide. Under a **scale**
they would not: a stroke is transformed with the shape it outlines, so doubling an
item draws its 4-pixel outline 8 pixels wide. Divide the width by the scale if you
want it to stay put."#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_fillStroke(fill AS Color, stroke AS Color, width AS Float) AS Paint
  RETURN Paint[fill := fill, stroke := stroke, strokeWidth := width, blend := BlendMode.Normal, transform := __canvas_noTransform(), clip := __canvas_noClip(), fillGradient := __canvas_noGradient()]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fillStroke",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "fill",
                    desc: "The interior colour.",
                    aliases: &[],
                    ty: ParameterType::named("Color"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "stroke",
                    desc: "The outline colour.",
                    aliases: &[],
                    ty: ParameterType::named("Color"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "width",
                    desc: "The outline width in pixels. `0.0` draws no outline.",
                    aliases: &[],
                    ty: ParameterType::Float,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named("Paint"),
            errors: vec![],
            body: Body::mfb(BODY, "__canvas_fillStroke"),
        }],
    });
}
