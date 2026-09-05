//! `color::luminance` — WCAG relative luminance.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"The WCAG relative luminance of a colour, `0.0` (black) to `1.0` (white)."#;

const DESC: &str = r#"`luminance` returns how bright a colour is to the eye, as the WCAG 2.2 relative
luminance: the weighted sum `0.2126 * red + 0.7152 * green + 0.0722 * blue` over
the **linear-light** channels, normalised so black is `0.0` and white is `1.0`.

The weights are not equal because the eye is not equally sensitive: green carries
roughly seven times the perceived brightness of blue at the same channel value.
This is why a pure green reads as far lighter than a pure blue even though both
are `255` in one channel.

The channels are converted with `color::toLinear` first. Computing the same sum
on the encoded sRGB bytes is a common mistake and gives a materially different
answer — it is the reason a naive "is this dark?" check misjudges mid-tones.

`alpha` is ignored. Luminance is a property of the colour, not of how much of it
shows; a half-transparent white is still a light colour, and what it ends up
looking like depends on what is behind it.

`color::contrastRatio` is built on this, and `color::isDark`/`color::isLight` are
the two-way split of it."#;

const EX: &str = r#"The endpoints are exact, and green far outweighs blue:

```
IMPORT color
IMPORT io

SUB main()
  io::print(toString(color::luminance(color::rgb(0, 0, 0))))
  io::print(toString(color::luminance(color::rgb(255, 255, 255))))
  io::print(toString(color::luminance(color::rgb(0, 255, 0))))
  io::print(toString(color::luminance(color::rgb(0, 0, 255))))
END SUB
```"#;

/// The weights are integers over 10000 rather than `0.2126`-style literals so the
/// numerator is exact integer arithmetic: they sum to exactly 10000, which is what
/// makes white come out at exactly `1.0` rather than a rounding step below. The
/// largest possible numerator is `10000 * 65535`, nowhere near an `Integer`
/// overflow.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_luminance(base AS Color) AS Float
  LET r AS Integer = color::toLinear(base.red)
  LET g AS Integer = color::toLinear(base.green)
  LET b AS Integer = color::toLinear(base.blue)
  RETURN toFloat(2126 * r + 7152 * g + 722 * b) / 10000.0 / 65535.0
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "luminance",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The colour to measure. Its alpha is ignored.",
                aliases: &[],
                ty: ParameterType::named(super::COLOR_TYPE),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Float,
            errors: vec![],
            body: Body::mfb(BODY, "__color_luminance"),
        }],
    });
}
