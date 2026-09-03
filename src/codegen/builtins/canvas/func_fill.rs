//! `canvas::fill` — a `Paint` that fills an item with one colour.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Build a `canvas::Paint` that fills an item with one colour and draws no outline."#;

const DESC: &str = r#"`fill` is the common case: a solid shape with no outline. Every other `canvas::Paint`
field is left at its no-op value — a transparent stroke, zero stroke width,
`Normal` blend, the identity transform, and no clip.

Use `canvas::stroke` for the outline-only case and `canvas::fillStroke` for both.
To set blend, transform or clip, update the result:

```
LET spun AS Paint = WITH canvas::fill(red) { transform := tilt }
```

These constructors exist because MFBASIC named construction requires **every**
field — `canvas::Paint[fill := c]` is a constructor-arity error, not a partial record — so
without them every item would have to spell out all seven `canvas::Paint` fields."#;

const EX: &str = r#"A solid yellow disc:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  LET yellow AS canvas::Color = canvas::rgb(255, 255, 0)
  LET face AS canvas::DrawItem = canvas::Circle[x := 200.0, y := 200.0, radius := 150.0, paint := canvas::fill(yellow)]
  canvas::present([face])
END SUB
```

A gradient instead of a flat colour. `fillGradient` replaces the fill's *colour* and
nothing else, so start from `canvas::fill` and set the field — the colour you pass is
what shows if the gradient turns out to have fewer than two stops:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := canvas::rgb(255, 64, 32)], canvas::GradientStop[offset := 1.0, color := canvas::rgb(32, 96, 255)]]
  LET ramp AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := canvas::Point[x := 40.0, y := 0.0], endPoint := canvas::Point[x := 360.0, y := 0.0], stops := stops]
  LET bar AS canvas::DrawItem = canvas::Rectangle[x := 40.0, y := 40.0, w := 320.0, h := 120.0, paint := WITH canvas::fill(canvas::rgb(255, 64, 32)) { fillGradient := ramp }]

  ' The same stops, radial. `startPoint` is the centre and `endPoint` a point on the
  ' outer circle, so this ramp is circular whatever shape it fills.
  LET glow AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Radial, startPoint := canvas::Point[x := 200.0, y := 300.0], endPoint := canvas::Point[x := 290.0, y := 300.0], stops := stops]
  LET orb AS canvas::DrawItem = canvas::Circle[x := 200.0, y := 300.0, radius := 90.0, paint := WITH canvas::fill(canvas::rgb(255, 64, 32)) { fillGradient := glow }]

  canvas::present([bar, orb])
END SUB
```

The ramp is measured in surface pixels, from its own two points and not from the
shape's bounds — which is why `bar`'s axis names 40 and 360 rather than 0 and 1, and why
moving the rectangle without moving the gradient slides the shape along the ramp."#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_fill(color AS Color) AS Paint
  RETURN __canvas_fillStroke(color, __canvas_transparent(), 0.0)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fill",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "color",
                desc: "The colour to fill the item's interior with.",
                aliases: &[],
                ty: ParameterType::named("Color"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named("Paint"),
            errors: vec![],
            body: Body::mfb(BODY, "__canvas_fill"),
        }],
    });
}
