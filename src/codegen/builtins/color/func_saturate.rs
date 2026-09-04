//! `color::saturate` — make a colour more vivid.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Make a colour more vivid by raising its HSL saturation."#;

const DESC: &str = r#"`saturate` moves the colour's saturation a fraction of the way towards fully
saturated, keeping its hue and lightness. `amount` `0.0` returns the colour
unchanged and `1.0` returns the fully saturated colour of the same hue and
lightness.

`amount` is **clamped** to `0.0`..`1.0`. `alpha` is carried through unchanged.

Saturation is an **HSL** property, so this works on the sRGB colour rather than in
linear light — the same space `color::toHsl` reports and CSS `hsl()` means. It is
a deliberate asymmetry with `color::brighten`/`color::darken`.

**A grey does not stay grey.** A colour with no saturation has no *meaningful*
hue, but `color::toHsl` reports its hue as `0.0`, and `0.0` degrees is red — so
`saturate(color::gray(128), 1.0)` is very nearly pure red, not a grey. This is
the HSL model's own answer and it is what CSS and Sass `saturate()` do.

It is deliberately **not** special-cased to return the grey. Doing so would make
the function discontinuous: a colour with saturation `0.001` would come back
almost fully red while one with saturation exactly `0.0` came back grey. If you
want a grey to stay grey, test `color::toHsl(c).saturation` first.

`color::desaturate` is the mirror operation, and it *is* well behaved on a grey —
reducing the saturation of a colour that has none changes nothing."#;

const EX: &str = r##"Push a muted colour towards its vivid form:

```
IMPORT color
IMPORT io

SUB main()
  LET muted AS color::Color = color::fromHex("#6688aa")
  io::print(color::toHex(color::saturate(muted, 0.0)))
  io::print(color::toHex(color::saturate(muted, 0.5)))
  io::print(color::toHex(color::saturate(muted, 1.0)))
END SUB
```

A grey does **not** stay grey — its reported hue is `0.0`, which is red:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::toHex(color::saturate(color::gray(128), 1.0)))
  ' Test the saturation first if a grey must stay grey.
  io::print(toString(color::toHsl(color::gray(128)).saturation))
END SUB
```"##;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_saturate(base AS Color, amount AS Float) AS Color
  LET a AS Float = __color_clampFraction(amount)
  LET parts AS Hsl = __color_colorToHsl(base)
  LET s AS Float = parts.saturation + (1.0 - parts.saturation) * a
  RETURN __color_hslToColor(parts.hue, s, parts.lightness, toInt(base.alpha))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "saturate",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "base",
                    desc: "The colour to make more vivid. Its alpha is carried through.",
                    aliases: &[],
                    ty: ParameterType::named(super::COLOR_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "amount",
                    desc: "How far towards fully saturated, clamped to `0.0`..`1.0`.",
                    aliases: &[],
                    ty: ParameterType::Float,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_saturate"),
        }],
    });
}
