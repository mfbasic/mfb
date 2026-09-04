//! `color::desaturate` — fade a colour towards grey.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Fade a colour towards grey by reducing its HSL saturation."#;

const DESC: &str = r#"`desaturate` moves the colour's saturation a fraction of the way towards zero,
keeping its hue and lightness. `amount` `0.0` returns the colour unchanged and
`1.0` returns a neutral grey.

`amount` is **clamped** to `0.0`..`1.0`. `alpha` is carried through unchanged.

Saturation is an **HSL** property, so this works on the sRGB colour rather than in
linear light.

**`desaturate(c, 1.0)` is not `color::grayscale(c)`.** They are two different
answers to "remove the colour", and they disagree. This one drives HSL saturation
to zero, which preserves HSL *lightness* — the midpoint between the colour's
brightest and dimmest channel. `grayscale` projects onto **relative luminance**,
which weights green far above blue, so it preserves *perceived brightness*. For a
pure blue the two differ substantially. Use `grayscale` when the grey must look as
bright as the colour did; use this when you want the HSL model's answer.

`color::saturate` is the mirror operation."#;

const EX: &str = r##"Fade a colour towards grey:

```
IMPORT color
IMPORT io

SUB main()
  LET vivid AS color::Color = color::fromHex("#3366cc")
  io::print(color::toHex(color::desaturate(vivid, 0.0)))
  io::print(color::toHex(color::desaturate(vivid, 0.5)))
  io::print(color::toHex(color::desaturate(vivid, 1.0)))
END SUB
```

Fully desaturating is **not** the same as `grayscale` — HSL lightness against
perceived brightness:

```
IMPORT color
IMPORT io

SUB main()
  LET blue AS color::Color = color::rgb(0, 0, 255)
  io::print("desaturate " & color::toHex(color::desaturate(blue, 1.0)))
  io::print("grayscale  " & color::toHex(color::grayscale(blue)))
END SUB
```"##;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_desaturate(base AS Color, amount AS Float) AS Color
  LET a AS Float = __color_clampFraction(amount)
  LET parts AS Hsl = __color_colorToHsl(base)
  LET s AS Float = parts.saturation - parts.saturation * a
  RETURN __color_hslToColor(parts.hue, s, parts.lightness, toInt(base.alpha))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "desaturate",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "base",
                    desc: "The colour to fade. Its alpha is carried through.",
                    aliases: &[],
                    ty: ParameterType::named(super::COLOR_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "amount",
                    desc: "How far towards grey, clamped to `0.0`..`1.0`.",
                    aliases: &[],
                    ty: ParameterType::Float,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_desaturate"),
        }],
    });
}
