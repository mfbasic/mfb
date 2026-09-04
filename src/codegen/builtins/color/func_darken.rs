//! `color::darken` — darken a colour perceptually, in linear light.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Darken a colour towards black by a fraction, in linear light."#;

const DESC: &str = r#"`darken` scales each channel down by a fraction, working on the **linear-light**
values rather than the encoded sRGB bytes. `amount` is a fraction: `0.0` returns
the colour unchanged, `1.0` returns black, `0.5` removes half the light.

It is the mirror of `color::brighten`, and works in linear light for the same
reason: equal `amount` steps should look like equal steps, which scaling the
encoded bytes does not give.

`amount` is **clamped** to `0.0`..`1.0` rather than rejected.

**Alpha is left untouched**, deliberately: darkening a colour must not also change
how much of it shows.

The endpoints are exact — `darken(c, 0.0)` is `c` and `darken(c, 1.0)` is black
with `c`'s alpha.

`darken` and `brighten` are **not** inverses. `darken(brighten(c, 0.5), 0.5)` does
not return `c`: each works on a fraction of the *current* value, so the round trip
lands lower. To go back, keep the original."#;

const EX: &str = r##"The endpoints, and a step between them:

```
IMPORT color
IMPORT io

SUB main()
  LET c AS color::Color = color::fromHex("#3366cc")
  io::print(color::toHex(color::darken(c, 0.0)))
  io::print(color::toHex(color::darken(c, 0.5)))
  io::print(color::toHex(color::darken(c, 1.0)))
END SUB
```

Build a two-tone pair from one brand colour:

```
IMPORT color
IMPORT io

SUB main()
  LET brand AS color::Color = color::fromHex("#3366cc")
  io::print("hover  " & color::toHex(color::brighten(brand, 0.2)))
  io::print("active " & color::toHex(color::darken(brand, 0.2)))
END SUB
```"##;

/// `lin - lin * amount`, per channel. At `1.0` the product is exactly `lin`, so the
/// result is exactly `0`; at `0.0` it is `0`, leaving the exact
/// `fromLinear(toLinear(c))` round trip.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_darkenChannel(channel AS Byte, amount AS Float) AS Byte
  LET lin AS Integer = color::toLinear(channel)
  RETURN color::fromLinear(lin - toInt(toFloat(lin) * amount))
END FUNC

FUNC __color_darken(base AS Color, amount AS Float) AS Color
  LET a AS Float = __color_clampFraction(amount)
  RETURN Color[__color_darkenChannel(base.red, a), __color_darkenChannel(base.green, a), __color_darkenChannel(base.blue, a), base.alpha]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "darken",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "base",
                    desc: "The colour to darken. Its alpha is carried through unchanged.",
                    aliases: &[],
                    ty: ParameterType::named(super::COLOR_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "amount",
                    desc: "How far to move towards black, clamped to `0.0`..`1.0`.",
                    aliases: &[],
                    ty: ParameterType::Float,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_darken"),
        }],
    });
}
