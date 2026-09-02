//! `canvas::rgba` — build a `Color` from four components.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Build a `canvas::Color` from red, green, blue and alpha components."#;

const DESC: &str = r#"`rgba` builds a `canvas::Color` from four components, each clamped to `0`..`255`. It is
the general form of `canvas::rgb`, which is the same call with `alpha` fixed at
`255`.

Components are **clamped, not rejected**: a value below `0` becomes `0` and a
value above `255` becomes `255`. Colours are routinely computed — a base plus a
delta, a channel scaled by a fraction, an interpolation between two colours — and
a result that lands one past an end is a rounding artefact rather than a mistake
worth stopping the program for.

`alpha` is straight (not premultiplied): `0` is fully transparent and `255` fully
opaque, and the colour's red/green/blue are unaffected by it. The all-zero
`canvas::Color` is therefore fully transparent, which is what makes it the no-op default
for a `canvas::Paint` channel — `canvas::Paint[fill := c]` leaves the stroke transparent because
an unset `canvas::Color` field is exactly `rgba(0, 0, 0, 0)`.

Unlike the rest of `canvas`, `rgb` and `rgba` do **not** require `app::Mode.Canvas`.
They touch no surface — they only build a value — so a program is free to compute
a palette before it ever presents anything."#;

const EX: &str = r#"A half-transparent red:

```
IMPORT canvas

SUB main()
  LET wash AS canvas::Color = canvas::rgba(255, 0, 0, 128)
END SUB
```

Components are clamped, so arithmetic that overshoots is safe:

```
IMPORT canvas
IMPORT io

SUB main()
  ' 300 clamps to 255, -20 clamps to 0.
  LET c AS canvas::Color = canvas::rgba(300, -20, 128, 255)
  io::print(toString(c.red) & " " & toString(c.green))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_rgba(red AS Integer, green AS Integer, blue AS Integer, alpha AS Integer) AS Color
  RETURN Color[__canvas_clampByte(red), __canvas_clampByte(green), __canvas_clampByte(blue), __canvas_clampByte(alpha)]
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
            return_type: ParameterType::named("Color"),
            errors: vec![],
            body: Body::mfb(BODY, "__canvas_rgba"),
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
