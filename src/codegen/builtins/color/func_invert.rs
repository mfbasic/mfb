//! `color::invert` — the photographic negative of a colour.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Return the photographic negative of a colour, keeping its alpha."#;

const DESC: &str = r#"`invert` replaces each of red, green and blue with `255 -` that channel and
returns `base`'s `alpha` unchanged. Black inverts to white, red to cyan, and any
colour inverted twice is itself.

`alpha` is deliberately **not** inverted. Inverting a colour is a statement about
its hue, not about how much of it shows; flipping transparency at the same time
would make `invert` unusable for the thing it is for — finding a contrasting
colour for the same mark.

Inversion is a channel-value operation, not a perceptual one, so the inverse of a
mid-grey is another mid-grey and is *not* a readable contrast against it. When
you need a colour that reads against another, `color::contrastRatio` answers the
question `invert` does not."#;

const EX: &str = r#"Each channel becomes `255 -` itself, and alpha is carried through:

```
IMPORT color
IMPORT io

SUB main()
  LET c AS color::Color = color::rgba(10, 200, 30, 128)
  LET n AS color::Color = color::invert(c)
  io::print(toString(n.red) & " " & toString(n.green) & " " & toString(n.blue) & " alpha " & toString(n.alpha))
END SUB
```

A colour inverted twice is itself:

```
IMPORT color
IMPORT io

SUB main()
  LET c AS color::Color = color::rgba(10, 200, 30, 128)
  LET back AS color::Color = color::invert(color::invert(c))
  io::print(toString(back.red) & " " & toString(back.green) & " " & toString(back.blue))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_invert(base AS Color) AS Color
  RETURN Color[toByte(255 - toInt(base.red)), toByte(255 - toInt(base.green)), toByte(255 - toInt(base.blue)), base.alpha]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "invert",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The colour to invert. Its alpha is carried through unchanged.",
                aliases: &[],
                ty: ParameterType::named(super::COLOR_TYPE),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_invert"),
        }],
    });
}
