//! `color::contrastRatio` — the WCAG contrast ratio between two colours.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"The WCAG contrast ratio between two colours, `1.0` to `21.0`."#;

const DESC: &str = r#"`contrastRatio` returns `(lighter + 0.05) / (darker + 0.05)`, where each side is
the colour's `color::luminance`. It is the WCAG 2.2 definition, and it answers the
question "can text in one of these be read against the other?".

The range is `1.0` (the two colours are equally bright — the text is invisible) to
`21.0` (black on white, or white on black). The `0.05` on both sides models
ambient screen glare, and it is what stops the ratio running to infinity against
pure black.

The order of the arguments does not matter: the function sorts them itself, so
`contrastRatio(fg, bg)` and `contrastRatio(bg, fg)` are the same number.

The WCAG thresholds worth remembering: **4.5** for body text, **3.0** for large
text and for user-interface components. A ratio is a floor, not a target — a
ratio that only just clears 4.5 is still hard to read for many people.

`alpha` is ignored on both sides, because `luminance` ignores it. A contrast
ratio involving a transparent colour is not meaningful until it has been
composited over something; do that first, with `color::mix`."#;

const EX: &str = r##"The two values the definition fixes: identical colours give `1.0`, and
black against white gives `21.0`:

```
IMPORT color
IMPORT io

SUB main()
  LET white AS color::Color = color::rgb(255, 255, 255)
  LET black AS color::Color = color::rgb(0, 0, 0)
  io::print(toString(color::contrastRatio(white, white)))
  io::print(toString(color::contrastRatio(black, white)))
END SUB
```

Check a foreground against a background for body text:

```
IMPORT color
IMPORT io

SUB main()
  LET fg AS color::Color = color::fromHex("#767676")
  LET bg AS color::Color = color::fromHex("#ffffff")
  LET ratio AS Float = color::contrastRatio(fg, bg)
  io::print("passes body text: " & toString(ratio >= 4.5))
END SUB
```"##;

/// Sorting inside the function rather than documenting an order is what makes the
/// two argument orders agree. `contrastRatio(x, x)` is exactly `1.0` because the
/// numerator and denominator are the same expression, and `(white, black)` is
/// exactly `21.0`: `1.05 / 0.05` is 21 to within far less than an `f64` step at
/// that magnitude, so it rounds to exactly `21.0`.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_contrastRatio(first AS Color, second AS Color) AS Float
  LET a AS Float = color::luminance(first)
  LET b AS Float = color::luminance(second)
  MUT hi AS Float = a
  MUT lo AS Float = b
  IF b > a THEN
    hi = b
    lo = a
  END IF
  RETURN (hi + 0.05) / (lo + 0.05)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "contrastRatio",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "first",
                    desc: "One of the two colours. Order does not matter.",
                    aliases: &[],
                    ty: ParameterType::named(super::COLOR_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "second",
                    desc: "The other colour. Order does not matter.",
                    aliases: &[],
                    ty: ParameterType::named(super::COLOR_TYPE),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Float,
            errors: vec![],
            body: Body::mfb(BODY, "__color_contrastRatio"),
        }],
    });
}
