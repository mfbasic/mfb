//! `color::toHsl` — decompose a colour into hue, saturation and lightness.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Decompose a colour into its hue, saturation and lightness."#;

const DESC: &str = r#"`toHsl` returns a `color::Hsl` describing `base`: `hue` in degrees `0.0`..`360.0`,
`saturation` and `lightness` in `0.0`..`1.0`. `color::hsl` is the inverse.

**HSL describes the sRGB colour, not linear light.** That is what CSS `hsl()` and
every design tool mean, so the numbers here match what a designer's tool shows for
the same hex. It is a deliberate asymmetry with `color::brighten` and
`color::darken`, which work in **linear light**: the `lightness` reported here is
not the perceived brightness `color::luminance` measures, and bumping one is not
the same as brightening the other.

**A fully unsaturated colour has no meaningful hue.** For any grey — including
black and white — `saturation` is `0.0` and `hue` is reported as `0.0` rather than
as an arbitrary angle. The round trip is still exact: `hsl` ignores hue entirely
when saturation is `0.0`, so `hsl(toHsl(grey))` returns the grey.

`alpha` is not part of the HSL model and is not returned. Keep it from the
original colour, or rebuild with `color::hsla`."#;

const EX: &str = r#"Decompose a colour and read its parts:

```
IMPORT color
IMPORT io

SUB main()
  LET parts AS color::Hsl = color::toHsl(color::rgb(0, 255, 0))
  io::print(toString(parts.hue))
  io::print(toString(parts.saturation))
  io::print(toString(parts.lightness))
END SUB
```

A grey reports hue `0.0`, and still round-trips exactly:

```
IMPORT color
IMPORT io

SUB main()
  LET grey AS color::Color = color::gray(128)
  LET parts AS color::Hsl = color::toHsl(grey)
  io::print("hue " & toString(parts.hue) & " sat " & toString(parts.saturation))
  io::print(color::toHex(color::hsl(parts.hue, parts.saturation, parts.lightness)))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_toHsl(base AS Color) AS Hsl
  RETURN __color_colorToHsl(base)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toHsl",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The colour to decompose. Its alpha is not part of the result.",
                aliases: &[],
                ty: ParameterType::named(super::COLOR_TYPE),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::HSL_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_toHsl"),
        }],
    });
}
