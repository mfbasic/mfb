//! `color::toHex` — render a colour as `#rrggbb`, dropping alpha.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Render a colour as `#rrggbb`, dropping its alpha channel."#;

const DESC: &str = r#"`toHex` returns the six-digit form — a leading `#` and two lowercase hex digits
each for red, green and blue. The width is fixed: a channel below `16` keeps its
leading zero, so the result is always exactly seven characters and a caller
writing a fixed-width field never has to pad.

**Alpha is dropped, not encoded.** `toHex` is for the places that take a
CSS-style colour and have no notion of transparency. When alpha matters, use
`color::toHexAlpha`, which is the lossless form. That the two are separate
members rather than one alpha-sensitive one is deliberate: the output width is a
property of the call, so a caller never has to branch on the data to know what it
will get.

Digits are lowercase, matching `encoding::hexEncode`, so two programs' `toHex`
output compares equal. `color::fromHex` accepts either case, so
`fromHex(toHex(c))` round-trips — though only for an opaque colour, since the
alpha `toHex` dropped comes back as `255`."#;

const EX: &str = r#"Always seven characters, with the leading zero kept:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::toHex(color::rgb(51, 102, 204)))
  io::print(color::toHex(color::rgb(1, 2, 3)))
END SUB
```

Alpha is dropped — use `toHexAlpha` to keep it:

```
IMPORT color
IMPORT io

SUB main()
  LET wash AS color::Color = color::rgba(255, 0, 0, 128)
  io::print(color::toHex(wash))
  io::print(color::toHexAlpha(wash))
END SUB
```"#;

// `r##"…"##`, not `r#"…"#`: the body contains the literal `"#`, which would
// otherwise close the raw string mid-expression.
#[rustfmt::skip]
const BODY: &str =
r##"FUNC __color_toHex(base AS Color) AS String
  RETURN "#" & __color_hexByte(base.red) & __color_hexByte(base.green) & __color_hexByte(base.blue)
END FUNC"##;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toHex",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The colour to render. Its alpha channel is not encoded.",
                aliases: &[],
                ty: ParameterType::named(super::COLOR_TYPE),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::mfb(BODY, "__color_toHex"),
        }],
    });
}
