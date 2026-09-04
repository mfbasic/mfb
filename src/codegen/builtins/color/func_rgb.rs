//! `color::rgb` — build an opaque `Color` from three components.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Build an opaque `color::Color` from red, green and blue components."#;

const DESC: &str = r#"`rgb` builds a `color::Color` from three components, each clamped to `0`..`255`,
with `alpha` fixed at `255` — fully opaque. It is `color::rgba` with the fourth
argument supplied, and it is the constructor most programs want: a colour with no
transparency.

Components are **clamped, not rejected**: a value below `0` becomes `0` and a
value above `255` becomes `255`. The components are `Integer` rather than `Byte`
precisely so that out-of-range arithmetic can reach the clamp instead of failing
at the call site.

To build a colour that is partly transparent, use `color::rgba`, or take an
existing colour and change one channel with `color::withAlpha`."#;

const EX: &str = r#"An opaque orange:

```
IMPORT color

SUB main()
  LET tangerine AS color::Color = color::rgb(255, 140, 0)
END SUB
```

`rgb` fixes alpha at fully opaque, and `withAlpha` reopens it:

```
IMPORT color
IMPORT io

SUB main()
  LET solid AS color::Color = color::rgb(255, 140, 0)
  LET ghost AS color::Color = color::withAlpha(solid, 64)
  io::print(toString(solid.alpha) & " " & toString(ghost.alpha))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_rgb(red AS Integer, green AS Integer, blue AS Integer) AS Color
  RETURN Color[__color_clampByte(red), __color_clampByte(green), __color_clampByte(blue), toByte(255)]
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
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_rgb"),
        }],
    });
}

/// `Integer`, not `Byte`, so out-of-range arithmetic reaches the clamp — see
/// `func_rgba`'s note.
fn component(name: &'static str, desc: &'static str) -> Parameter {
    Parameter {
        name,
        desc,
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::None,
    }
}
