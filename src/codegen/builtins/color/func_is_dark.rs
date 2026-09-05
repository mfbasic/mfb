//! `color::isDark` — whether a colour reads as dark.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Whether a colour reads as dark — its relative luminance is below `0.5`."#;

const DESC: &str = r#"`isDark` is `color::luminance(base) < 0.5`. It is the quick question a program
asks when it has a background and needs to pick a foreground: dark background,
light text.

It is a **convenience, not a contrast check**. Two colours can both answer `FALSE`
and still be unreadable against each other. When the question is "can this text be
read on that background?", the answer is `color::contrastRatio` against the WCAG
thresholds (`4.5` for body text, `3.0` for large text), not this.

The threshold is on **relative luminance**, so it accounts for the eye's uneven
channel sensitivity — a saturated blue at full strength is dark, a saturated
yellow is not, even though both have two channels at `255`.

`alpha` is ignored, because `luminance` ignores it. A half-transparent colour's
apparent lightness depends on what is behind it, which this function cannot see.

`color::isLight` is exactly its negation, so every colour is one or the other and
never both."#;

const EX: &str = r##"Channel sensitivity, not channel count, decides:

```
IMPORT color
IMPORT io

SUB main()
  io::print("blue   " & toString(color::isDark(color::rgb(0, 0, 255))))
  io::print("yellow " & toString(color::isDark(color::rgb(255, 255, 0))))
END SUB
```

Pick readable text for a background:

```
IMPORT color
IMPORT io

SUB main()
  LET bg AS color::Color = color::fromHex("#222222")
  MUT fg AS color::Color = color::rgb(0, 0, 0)
  IF color::isDark(bg) THEN
    fg = color::rgb(255, 255, 255)
  END IF
  io::print(color::toHex(fg))
END SUB
```"##;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_isDark(base AS Color) AS Boolean
  RETURN color::luminance(base) < 0.5
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isDark",
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
            body: Body::mfb(BODY, "__color_isDark"),
        }],
    });
}
