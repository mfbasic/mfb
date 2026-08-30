//! `canvas::rgb` — build an opaque `Color` from three components.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Build an opaque `Color` from red, green and blue components."#;

const DESC: &str = r#"`rgb` builds a fully opaque `Color` — `canvas::rgba` with `alpha` fixed at
`255`. Each component is clamped to `0`..`255` rather than rejected, for the
reason given on `canvas::rgba`: colours are routinely computed, and a value that
lands one past an end is a rounding artefact, not a program bug.

Building a `Color` field by field is possible but reads poorly in source, which is
why these two constructors exist at all.

`rgb` and `rgba` are the two `canvas::` calls that do **not** require
`Mode.Canvas`. They touch no surface — they only build a value — so a program can
compute its palette before it ever presents anything."#;

const EX: &str = r#"A yellow face on a canvas:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(Mode.Canvas)
  LET yellow AS Color = canvas::rgb(255, 255, 0)
  canvas::present([
    Circle[x := 200.0, y := 200.0, radius := 150.0, paint := Paint[fill := yellow]]
  ])
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_rgb(red AS Integer, green AS Integer, blue AS Integer) AS Color
  RETURN __canvas_rgba(red, green, blue, 255)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "rgb",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                component("red", "The red component, clamped to `0`..`255`."),
                component("green", "The green component, clamped to `0`..`255`."),
                component("blue", "The blue component, clamped to `0`..`255`."),
            ],
            return_type: ParameterType::named("Color"),
            errors: vec![],
            body: Body::mfb(BODY, "__canvas_rgb"),
        }],
    });
}

fn component(name: &'static str, desc: &'static str) -> Parameter {
    Parameter {
        name,
        desc,
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::None,
    }
}
