//! `color::rgba` — build a `Color` from four components.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Build a `color::Color` from red, green, blue and alpha components."#;

const DESC: &str = r#"`rgba` builds a `color::Color` from four components, each clamped to `0`..`255`.
It is the general form of `color::rgb`, which is the same call with `alpha` fixed
at `255`.

Components are **clamped, not rejected**: a value below `0` becomes `0` and a
value above `255` becomes `255`. Colours are routinely computed — a base plus a
delta, a channel scaled by a fraction, an interpolation between two colours — and
a result that lands one past an end is a rounding artefact rather than a mistake
worth stopping the program for. The components are `Integer` rather than `Byte`
precisely so that out-of-range arithmetic can reach the clamp instead of failing
at the call site.

`alpha` is straight (not premultiplied): `0` is fully transparent and `255` fully
opaque, and the colour's red, green and blue are unaffected by it. The all-zero
`color::Color` is therefore fully transparent."#;

const EX: &str = r#"A half-transparent red:

```
IMPORT color

SUB main()
  LET wash AS color::Color = color::rgba(255, 0, 0, 128)
END SUB
```

Components are clamped, so arithmetic that overshoots is safe:

```
IMPORT color
IMPORT io

SUB main()
  ' 300 clamps to 255, -20 clamps to 0.
  LET c AS color::Color = color::rgba(300, -20, 128, 255)
  io::print(toString(c.red) & " " & toString(c.green))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_rgba(red AS Integer, green AS Integer, blue AS Integer, alpha AS Integer) AS Color
  RETURN Color[__color_clampByte(red), __color_clampByte(green), __color_clampByte(blue), __color_clampByte(alpha)]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "rgba",
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
                component(
                    "alpha",
                    "The alpha component, clamped to `0`..`255`: `0` fully \
                     transparent, `255` fully opaque.",
                ),
            ],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_rgba"),
        }],
    });
}

/// The components are `Integer`, not `Byte`, precisely so that out-of-range
/// arithmetic can *reach* the clamp. Declaring them `Byte` would push the failure
/// back to the call site as a conversion error, which is the opposite of the
/// clamping contract.
fn component(name: &'static str, desc: &'static str) -> Parameter {
    Parameter {
        name,
        desc,
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::None,
    }
}
