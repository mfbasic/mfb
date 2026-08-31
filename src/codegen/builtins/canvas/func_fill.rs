//! `canvas::fill` — a `Paint` that fills an item with one colour.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Build a `Paint` that fills an item with one colour and draws no outline."#;

const DESC: &str = r#"`fill` is the common case: a solid shape with no outline. Every other `Paint`
field is left at its no-op value — a transparent stroke, zero stroke width,
`Normal` blend, the identity transform, and no clip.

Use `canvas::stroke` for the outline-only case and `canvas::fillStroke` for both.
To set blend, transform or clip, update the result:

```
LET spun AS Paint = WITH canvas::fill(red) { transform := tilt }
```

These constructors exist because MFBASIC named construction requires **every**
field — `Paint[fill := c]` is a constructor-arity error, not a partial record — so
without them every item would have to spell out all six `Paint` fields."#;

const EX: &str = r#"A solid yellow disc:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(Mode.Canvas)
  LET yellow AS Color = canvas::rgb(255, 255, 0)
  LET face AS DrawItem = Circle[x := 200.0, y := 200.0, radius := 150.0, paint := canvas::fill(yellow)]
  canvas::present([face])
END SUB
```"#;

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
