//! `canvas::fillStroke` — a `Paint` that both fills and outlines.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Build a `canvas::Paint` that both fills an item and outlines it."#;

const DESC: &str = r#"`fillStroke` builds a `canvas::Paint` with both channels set: the item's interior is
`fill` and its outline is `width` pixels of `stroke`. `canvas::fill` and
`canvas::stroke` are the one-channel forms.

Blend, transform and clip are left at their no-op values (`Normal`, the identity,
unclipped). To set one of those, update the result:

```
LET glow AS Paint = WITH canvas::fillStroke(core, halo, 2.0) { blend := BlendMode.Add }
```

These constructors exist because MFBASIC named construction requires **every**
field — `canvas::Paint[fill := c]` is a constructor-arity error, not a partial record — so
without them every item would have to spell out all six `canvas::Paint` fields."#;

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
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_fillStroke(fill AS Color, stroke AS Color, width AS Float) AS Paint
  RETURN Paint[fill := fill, stroke := stroke, strokeWidth := width, blend := BlendMode.Normal, transform := __canvas_noTransform(), clip := __canvas_noClip()]
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
