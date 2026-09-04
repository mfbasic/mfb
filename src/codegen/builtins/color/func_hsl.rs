//! `color::hsl` — build an opaque colour from hue, saturation and lightness.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Build an opaque colour from hue, saturation and lightness."#;

const DESC: &str = r#"`hsl` builds a `color::Color` from the HSL model: `hue` in degrees around the
colour wheel, `saturation` from grey to full colour, and `lightness` from black
through the pure hue to white. `alpha` is fully opaque; `color::hsla` is the same
call with alpha supplied.

`hue` **wraps** rather than clamping — `hsl(400.0, ...)` is `hsl(40.0, ...)`, and
a negative hue wraps up from `360.0`, so rotating around the wheel needs no
bookkeeping. `saturation` and `lightness` **clamp** to `0.0`..`1.0`, matching the
clamping rule the rest of the package follows. The asymmetry is the model's: an
angle is periodic and a fraction is not.

Landmarks worth knowing: `lightness` `0.0` is black and `1.0` is white **at every
hue and saturation**, and `0.5` is where a fully saturated hue is most vivid.
`saturation` `0.0` is a neutral grey whose level is `lightness`.

**HSL describes the sRGB colour, not linear light.** That is what CSS `hsl()` and
every design tool mean, so a colour built here matches the hex a designer would
write. It is a deliberate asymmetry with `color::brighten` and `color::darken`,
which work in linear light — bumping `lightness` here and brightening there give
different colours, and neither is wrong."#;

const EX: &str = r#"The three primaries are 120 degrees apart:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::toHex(color::hsl(0.0, 1.0, 0.5)))
  io::print(color::toHex(color::hsl(120.0, 1.0, 0.5)))
  io::print(color::toHex(color::hsl(240.0, 1.0, 0.5)))
END SUB
```

Hue wraps, so going past a full turn is the same colour:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::toHex(color::hsl(400.0, 1.0, 0.5)))
  io::print(color::toHex(color::hsl(40.0, 1.0, 0.5)))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_hsl(hue AS Float, saturation AS Float, lightness AS Float) AS Color
  RETURN __color_hslToColor(hue, saturation, lightness, 255)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hsl",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                component("hue", "The hue in degrees. Wraps, so any value is valid."),
                component(
                    "saturation",
                    "How colourful, clamped to `0.0` (grey) .. `1.0` (full).",
                ),
                component(
                    "lightness",
                    "How light, clamped to `0.0` (black) .. `1.0` (white).",
                ),
            ],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_hsl"),
        }],
    });
}

fn component(name: &'static str, desc: &'static str) -> Parameter {
    Parameter {
        name,
        desc,
        aliases: &[],
        ty: ParameterType::Float,
        default: DefaultValue::None,
    }
}
