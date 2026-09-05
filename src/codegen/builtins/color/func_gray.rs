//! `color::gray` — build an opaque neutral grey from one level.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Build an opaque neutral grey with every channel set to the same level."#;

const DESC: &str = r#"`gray` builds a `color::Color` whose red, green and blue channels are all
`level`, with `alpha` fixed at `255`. `color::gray(0)` is black,
`color::gray(255)` is white, and `color::gray(128)` is the midpoint by channel
value.

`level` is clamped to `0`..`255` like every other component.

Note that a grey chosen this way is neutral by *channel value*, not by perceived
lightness — `color::gray(128)` is darker than "half as bright as white" looks.
For a grey that is half as bright to the eye, use `color::darken` on white, or
pick the level by `color::luminance`.

The spelling is `gray`, matching the CSS keyword. There is no `grey` member."#;

const EX: &str = r#"Every channel takes the same level:

```
IMPORT color
IMPORT io

SUB main()
  LET mid AS color::Color = color::gray(128)
  io::print(toString(mid.red) & " " & toString(mid.green) & " " & toString(mid.blue))
END SUB
```

`gray(0)` is black and `gray(255)` is white, and both are fully opaque:

```
IMPORT color
IMPORT io

SUB main()
  LET black AS color::Color = color::gray(0)
  LET white AS color::Color = color::gray(255)
  io::print(toString(black.red) & " " & toString(white.red) & " alpha " & toString(black.alpha))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_gray(level AS Integer) AS Color
  RETURN color::rgb(level, level, level)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "gray",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "level",
                desc: "The level every channel takes, clamped to `0`..`255`.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_gray"),
        }],
    });
}
