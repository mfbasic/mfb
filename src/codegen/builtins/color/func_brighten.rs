//! `color::brighten` — lighten a colour perceptually, in linear light.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Lighten a colour towards white by a fraction, in linear light."#;

const DESC: &str = r#"`brighten` moves each channel a fraction of the way from where it is to full
brightness, working on the **linear-light** values rather than the encoded sRGB
bytes. `amount` is a fraction: `0.0` returns the colour unchanged, `1.0` returns
white, `0.5` closes half the remaining distance.

Doing this in linear light is the whole point. Scaling the sRGB bytes instead —
`red * 1.5` — is not perceptually uniform: the same multiplier lifts a dark colour
far more than a light one, and a ramp built that way looks lumpy. `brighten`
converts through `color::toLinear`, moves, and converts back, so equal `amount`
steps look like equal steps.

`amount` is **clamped** to `0.0`..`1.0` rather than rejected, matching the
clamping rule the constructors follow.

**Alpha is left untouched**, deliberately: lightening a colour must not also make
it more opaque. If you want both, follow with `color::withAlpha`.

The endpoints are exact — `brighten(c, 0.0)` is `c` and `brighten(c, 1.0)` is
white with `c`'s alpha — not one step short. `color::darken` is the mirror
operation towards black.

Note that hue, saturation and lightness describe the **sRGB** colour
(`color::toHsl`), while `brighten` and `darken` work in **linear light**. The two
are different spaces on purpose, and a colour brightened here will not match a
lightness bumped there."#;

const EX: &str = r##"The endpoints, and a step between them:

```
IMPORT color
IMPORT io

SUB main()
  LET c AS color::Color = color::fromHex("#3366cc")
  io::print(color::toHex(color::brighten(c, 0.0)))
  io::print(color::toHex(color::brighten(c, 0.5)))
  io::print(color::toHex(color::brighten(c, 1.0)))
END SUB
```

Alpha survives untouched, so a translucent colour stays translucent:

```
IMPORT color
IMPORT io

SUB main()
  LET wash AS color::Color = color::rgba(20, 40, 60, 128)
  io::print(color::toHexAlpha(color::brighten(wash, 0.75)))
END SUB
```"##;

/// `lin + (65535 - lin) * amount`, per channel. The endpoints are exact without a
/// rounding fudge: at `1.0` the product is exactly `65535 - lin` (both operands are
/// far below 2^24, so the `Float` multiply is exact), and at `0.0` it is `0`, which
/// leaves `fromLinear(toLinear(c))` — itself exact for all 256 channels.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_brightenChannel(channel AS Byte, amount AS Float) AS Byte
  LET lin AS Integer = color::toLinear(channel)
  RETURN color::fromLinear(lin + toInt(toFloat(65535 - lin) * amount))
END FUNC

FUNC __color_brighten(base AS Color, amount AS Float) AS Color
  LET a AS Float = __color_clampFraction(amount)
  RETURN Color[__color_brightenChannel(base.red, a), __color_brightenChannel(base.green, a), __color_brightenChannel(base.blue, a), base.alpha]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "brighten",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "base",
                    desc: "The colour to lighten. Its alpha is carried through unchanged.",
                    aliases: &[],
                    ty: ParameterType::named(super::COLOR_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "amount",
                    desc: "How far to move towards white, clamped to `0.0`..`1.0`.",
                    aliases: &[],
                    ty: ParameterType::Float,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_brighten"),
        }],
    });
}
