//! `color::grayscale` — the neutral grey of the same perceived brightness.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Convert a colour to the neutral grey of the same perceived brightness."#;

const DESC: &str = r#"`grayscale` sets every colour channel to the colour's relative luminance, so the
result is a neutral grey that looks as bright as the original did. The weights are
the WCAG ones `color::luminance` uses — green counts for far more than blue —
computed on the **linear-light** channels.

This is what a correct desaturation looks like. Averaging the three sRGB bytes
instead treats a pure blue and a pure green as equally bright, which they are
nowhere near: the average turns a vivid green and a deep blue into the same grey.

**Alpha is left untouched.**

`grayscale(c)` is not the same as `color::desaturate(c, 1.0)`. This one is a
luminance projection; `desaturate` drives the HSL saturation to zero, which
preserves HSL *lightness* instead of perceived brightness. Both are reasonable
answers to "remove the colour", and they differ."#;

const EX: &str = r##"A vivid green and a deep blue do not become the same grey:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::toHex(color::grayscale(color::rgb(0, 255, 0))))
  io::print(color::toHex(color::grayscale(color::rgb(0, 0, 255))))
END SUB
```

Greys are already neutral, so the operation is idempotent on them:

```
IMPORT color
IMPORT io

SUB main()
  LET g AS color::Color = color::grayscale(color::fromHex("#3366cc"))
  io::print(color::toHex(g))
  io::print(color::toHex(color::grayscale(g)))
END SUB
```"##;

/// The luminance is computed in the linear domain as an `Integer` `0..65535` rather
/// than going through `color::luminance`'s `Float` `0.0..1.0` and back, so the
/// endpoints stay exact: white's weighted sum is exactly `10000 * 65535`, which
/// divides to exactly `65535`, and `fromLinear(65535)` is `255`.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_grayscale(base AS Color) AS Color
  LET r AS Integer = color::toLinear(base.red)
  LET g AS Integer = color::toLinear(base.green)
  LET b AS Integer = color::toLinear(base.blue)
  LET lum AS Byte = color::fromLinear((2126 * r + 7152 * g + 722 * b) / 10000)
  RETURN Color[lum, lum, lum, base.alpha]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "grayscale",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The colour to desaturate. Its alpha is carried through unchanged.",
                aliases: &[],
                ty: ParameterType::named(super::COLOR_TYPE),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_grayscale"),
        }],
    });
}
