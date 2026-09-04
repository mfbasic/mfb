//! `color::toHexAlpha` — render a colour as `#rrggbbaa`, losslessly.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Render a colour as `#rrggbbaa`, keeping its alpha channel."#;

const DESC: &str = r#"`toHexAlpha` returns the eight-digit form — a leading `#` and two lowercase hex
digits each for red, green, blue and alpha. The width is fixed at nine characters
whatever the channels are, so a channel below `16` keeps its leading zero.

This is the **lossless** text form: every colour has exactly one `toHexAlpha`
spelling and `color::fromHex` reads it back exactly, so
`fromHex(toHexAlpha(c))` is `c` for every colour including transparent ones. It
is what to write to a config file or a wire format. `color::toHex` is the
alpha-dropping form for the places that only take six digits.

`toString` on a colour produces this same `#rrggbbaa` form, for the same reason:
it is the spelling that loses nothing."#;

const EX: &str = r#"Alpha is kept, and the width is always nine characters:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::toHexAlpha(color::rgba(255, 0, 0, 128)))
  io::print(color::toHexAlpha(color::rgba(1, 2, 3, 4)))
END SUB
```

The round trip through `fromHex` is exact, transparency included:

```
IMPORT color
IMPORT io

SUB main()
  LET c AS color::Color = color::rgba(51, 102, 204, 64)
  io::print(color::toHexAlpha(color::fromHex(color::toHexAlpha(c))))
END SUB
```"#;

// `r##"…"##`, not `r#"…"#`: the body contains the literal `"#`, which would
// otherwise close the raw string mid-expression.
#[rustfmt::skip]
const BODY: &str =
r##"FUNC __color_toHexAlpha(base AS Color) AS String
  RETURN "#" & __color_hexByte(base.red) & __color_hexByte(base.green) & __color_hexByte(base.blue) & __color_hexByte(base.alpha)
END FUNC"##;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toHexAlpha",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The colour to render, alpha included.",
                aliases: &[],
                ty: ParameterType::named(super::COLOR_TYPE),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::mfb(BODY, "__color_toHexAlpha"),
        }],
    });
}
