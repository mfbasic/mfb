//! `color::mix` — blend two colours in linear light.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Blend two colours by a fraction, in linear light."#;

const DESC: &str = r#"`mix` interpolates between two colours: `amount` `0.0` returns `first`, `1.0`
returns `second`, and `0.5` is the midpoint. The colour channels are interpolated
in **linear light**, so the midpoint of black and white is the one that *looks*
halfway rather than the one that is halfway between the encoded bytes.

That distinction is the whole reason this function exists. Averaging sRGB bytes
gives `#808080` for black-and-white, which reads as noticeably dark; the linear
midpoint is around `#bcbcbc`. A gradient built by averaging bytes spends half its
length below 22% of the light, which is why such ramps look dark-heavy.

`amount` is **clamped** to `0.0`..`1.0`, so `mix` interpolates and never
extrapolates past either end.

**Alpha is interpolated too**, unlike `color::brighten` and `color::darken` which
leave it alone. The difference is deliberate: `mix` is a blend of two whole
colours, so every channel including alpha takes part, whereas brightening is a
statement about one colour's light only. Alpha is interpolated on its raw value,
not through the linear transfer — alpha is a coverage fraction, not a light
intensity, and is not gamma-encoded.

The endpoints are exact: `mix(a, b, 0.0)` is `a` and `mix(a, b, 1.0)` is `b`, in
every channel including alpha."#;

const EX: &str = r#"The linear midpoint of black and white is not `#808080`:

```
IMPORT color
IMPORT io

SUB main()
  LET black AS color::Color = color::rgb(0, 0, 0)
  LET white AS color::Color = color::rgb(255, 255, 255)
  io::print(color::toHex(color::mix(black, white, 0.5)))
END SUB
```

The endpoints return their operand exactly, alpha included:

```
IMPORT color
IMPORT io

SUB main()
  LET a AS color::Color = color::rgba(10, 20, 30, 40)
  LET b AS color::Color = color::rgba(200, 210, 220, 230)
  io::print(color::toHexAlpha(color::mix(a, b, 0.0)))
  io::print(color::toHexAlpha(color::mix(a, b, 1.0)))
END SUB
```"#;

/// Colour channels lerp in linear light; alpha lerps on its raw value. Both
/// endpoints are exact — at `0.0` the delta term is `0`, and at `1.0` the `Float`
/// product is exactly the difference (operands far below 2^24), so the sum is
/// exactly the second operand.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_mixChannel(first AS Byte, second AS Byte, amount AS Float) AS Byte
  LET a AS Integer = color::toLinear(first)
  LET b AS Integer = color::toLinear(second)
  RETURN color::fromLinear(a + toInt(toFloat(b - a) * amount))
END FUNC

FUNC __color_mix(first AS Color, second AS Color, amount AS Float) AS Color
  LET t AS Float = __color_clampFraction(amount)
  LET aa AS Integer = toInt(first.alpha)
  LET ba AS Integer = toInt(second.alpha)
  RETURN Color[__color_mixChannel(first.red, second.red, t), __color_mixChannel(first.green, second.green, t), __color_mixChannel(first.blue, second.blue, t), toByte(aa + toInt(toFloat(ba - aa) * t))]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "mix",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "first",
                    desc: "The colour `amount` `0.0` returns.",
                    aliases: &[],
                    ty: ParameterType::named(super::COLOR_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "second",
                    desc: "The colour `amount` `1.0` returns.",
                    aliases: &[],
                    ty: ParameterType::named(super::COLOR_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "amount",
                    desc: "How far from `first` to `second`, clamped to `0.0`..`1.0`.",
                    aliases: &[],
                    ty: ParameterType::Float,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_mix"),
        }],
    });
}
