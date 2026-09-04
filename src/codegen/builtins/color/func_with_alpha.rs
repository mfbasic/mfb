//! `color::withAlpha` — a copy of a colour with a different alpha channel.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Copy a colour with its alpha channel replaced."#;

const DESC: &str = r#"`withAlpha` returns a colour with `base`'s red, green and blue and the given
`alpha`, clamped to `0`..`255`. `base` is unchanged — you get a copy.

This is how a program makes an existing colour transparent without restating it:
`color::withAlpha(brand, 128)` is the half-transparent form of `brand` whatever
`brand` happens to be.

`alpha` is straight, not premultiplied, so the red, green and blue channels are
returned exactly as they were. A colour at `alpha` `0` still remembers its
hue — `color::withAlpha(color::withAlpha(c, 0), 255)` is `c`."#;

const EX: &str = r#"Make a colour half transparent without restating it:

```
IMPORT color
IMPORT io

SUB main()
  LET brand AS color::Color = color::rgb(0, 120, 200)
  LET wash AS color::Color = color::withAlpha(brand, 128)
  io::print(toString(brand.alpha) & " -> " & toString(wash.alpha))
END SUB
```

The colour channels survive a round trip through full transparency:

```
IMPORT color
IMPORT io

SUB main()
  LET brand AS color::Color = color::rgb(0, 120, 200)
  LET hidden AS color::Color = color::withAlpha(brand, 0)
  LET back AS color::Color = color::withAlpha(hidden, 255)
  io::print(toString(back.red) & " " & toString(back.green) & " " & toString(back.blue))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_withAlpha(base AS Color, alpha AS Integer) AS Color
  RETURN Color[base.red, base.green, base.blue, __color_clampByte(alpha)]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "withAlpha",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "base",
                    desc: "The colour whose red, green and blue are kept.",
                    aliases: &[],
                    ty: ParameterType::named(super::COLOR_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "alpha",
                    desc: "The replacement alpha, clamped to `0`..`255`: `0` fully \
                           transparent, `255` fully opaque.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_withAlpha"),
        }],
    });
}
