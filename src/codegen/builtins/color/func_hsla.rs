//! `color::hsla` — build a colour from hue, saturation, lightness and alpha.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Build a colour from hue, saturation, lightness and an alpha component."#;

const DESC: &str = r#"`hsla` is `color::hsl` with the alpha channel supplied rather than fixed at fully
opaque. Every rule from that page carries over: `hue` **wraps**, `saturation` and
`lightness` **clamp** to `0.0`..`1.0`, and the model describes the **sRGB** colour
rather than linear light.

`alpha` is an `Integer` `0`..`255` — not a `0.0`..`1.0` fraction like CSS's
`hsla()` — because it is the same alpha every other `color` constructor takes, and
one spelling for one concept is worth more than matching CSS's mixed units. It is
clamped like the rest.

`alpha` is straight, not premultiplied, so it does not affect the colour the hue,
saturation and lightness describe."#;

const EX: &str = r#"A half-transparent vivid red:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::toHexAlpha(color::hsla(0.0, 1.0, 0.5, 128)))
END SUB
```

`hsla` with a fully opaque alpha is exactly `hsl`:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::toHexAlpha(color::hsla(210.0, 0.6, 0.5, 255)))
  io::print(color::toHexAlpha(color::hsl(210.0, 0.6, 0.5)))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_hsla(hue AS Float, saturation AS Float, lightness AS Float, alpha AS Integer) AS Color
  RETURN __color_hslToColor(hue, saturation, lightness, alpha)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hsla",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                fraction("hue", "The hue in degrees. Wraps, so any value is valid."),
                fraction(
                    "saturation",
                    "How colourful, clamped to `0.0` (grey) .. `1.0` (full).",
                ),
                fraction(
                    "lightness",
                    "How light, clamped to `0.0` (black) .. `1.0` (white).",
                ),
                Parameter {
                    name: "alpha",
                    desc: "The alpha component, clamped to `0`..`255`: `0` fully \
                           transparent, `255` fully opaque.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_hsla"),
        }],
    });
}

fn fraction(name: &'static str, desc: &'static str) -> Parameter {
    Parameter {
        name,
        desc,
        aliases: &[],
        ty: ParameterType::Float,
        default: DefaultValue::None,
    }
}
