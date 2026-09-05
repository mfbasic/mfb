//! `color::isLight` — whether a colour reads as light.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Whether a colour reads as light — its relative luminance is `0.5` or above."#;

const DESC: &str = r#"`isLight` is exactly `NOT color::isDark(base)`. Every colour is one or the other
and never both, including the boundary: a colour whose relative luminance is
exactly `0.5` is light, not dark.

It exists so the common test reads the way it is meant rather than as a negation,
which is easy to misread at a glance.

Like `color::isDark` it is a **convenience, not a contrast check** — use
`color::contrastRatio` when the real question is readability — and it ignores
`alpha`, because `color::luminance` does."#;

const EX: &str = r##"The two are exact complements:

```
IMPORT color
IMPORT io

SUB main()
  LET c AS color::Color = color::fromHex("#3366cc")
  io::print(toString(color::isLight(c)))
  io::print(toString(color::isLight(c) = (NOT color::isDark(c))))
END SUB
```"##;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_isLight(base AS Color) AS Boolean
  RETURN NOT color::isDark(base)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isLight",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The colour to test. Its alpha is ignored.",
                aliases: &[],
                ty: ParameterType::named(super::COLOR_TYPE),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::mfb(BODY, "__color_isLight"),
        }],
    });
}
