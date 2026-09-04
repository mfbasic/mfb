//! `color::rotateHue` — turn a colour around the colour wheel.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Turn a colour around the colour wheel by a number of degrees."#;

const DESC: &str = r#"`rotateHue` adds `degrees` to the colour's hue, keeping its saturation and
lightness. It is how a palette of related colours gets built from one: the
complement is `180.0` away, a triad is `120.0` apart, and analogous colours are a
few tens of degrees either side.

**`degrees` wraps rather than clamping.** `rotateHue(c, 400.0)` is
`rotateHue(c, 40.0)`, and a negative value turns the other way, so walking around
the wheel needs no bookkeeping and no range check.

Hue is an **HSL** property, so this works on the sRGB colour rather than in linear
light. `alpha` is carried through unchanged.

A colour with no saturation — any grey — has no hue to turn, so `rotateHue`
returns it unchanged whatever `degrees` says.

Rotating by `360.0` returns the original colour, and rotating twice is the same as
rotating once by the sum."#;

const EX: &str = r##"A triad: three colours 120 degrees apart:

```
IMPORT color
IMPORT io

SUB main()
  LET base AS color::Color = color::fromHex("#3366cc")
  io::print(color::toHex(base))
  io::print(color::toHex(color::rotateHue(base, 120.0)))
  io::print(color::toHex(color::rotateHue(base, 240.0)))
END SUB
```

Degrees wrap, so past a full turn is the same colour:

```
IMPORT color
IMPORT io

SUB main()
  LET base AS color::Color = color::fromHex("#3366cc")
  io::print(color::toHex(color::rotateHue(base, 400.0)))
  io::print(color::toHex(color::rotateHue(base, 40.0)))
  io::print(color::toHex(color::rotateHue(base, 360.0)))
END SUB
```"##;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_rotateHue(base AS Color, degrees AS Float) AS Color
  LET parts AS Hsl = __color_colorToHsl(base)
  RETURN __color_hslToColor(parts.hue + degrees, parts.saturation, parts.lightness, toInt(base.alpha))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "rotateHue",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "base",
                    desc: "The colour to turn. Its alpha is carried through.",
                    aliases: &[],
                    ty: ParameterType::named(super::COLOR_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "degrees",
                    desc: "How far around the wheel. Wraps, so any value is valid, and \
                           a negative value turns the other way.",
                    aliases: &[],
                    ty: ParameterType::Float,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_rotateHue"),
        }],
    });
}
