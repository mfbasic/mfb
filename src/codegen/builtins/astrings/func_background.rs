//! `astrings::background` — (r, g, b) Byte-triple `Attribute` constructor
//! (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_background` FUNC (which packs the triple into a `0xRRGGBB`
//! numeric payload) through the registry's `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Construct a background-color `astrings::Attribute`."#;

const DESC: &str = r#"`background` returns an `astrings::Attribute` wrapping the `astrings::AttrNumber` with `kind`
`astrings::AttrTypeNumber.Background` and a `value` that packs the `(r, g, b)` channels into
a single `0xRRGGBB` Integer — `r` in the high byte, `b` in the low byte. Each
channel is a `Byte`, so the packing is lossless. Pass it to
`astrings::addAttribute` to set the text background color over a scalar range;
overlapping background spans resolve by higher-start-wins at read time.

When such an `AttributedString` is drawn with `term::drawText(x, y, value)` (both
`term` and `astrings` imported), the color is emitted as a truecolor background.
Renderers that do not model color — such as `astrings::toMarkdown` — ignore it."#;

const EX: &str = r#"```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::background(0, 0, 255))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_background(r AS Byte, g AS Byte, b AS Byte) AS Attribute
  RETURN AttrNumber[AttrTypeNumber.Background, __astrings_packColor(r, g, b, toByte(255))]
END FUNC"#;

fn color_params() -> Vec<Parameter> {
    [
        ("r", "The red channel (0–255)."),
        ("g", "The green channel (0–255)."),
        ("b", "The blue channel (0–255)."),
    ]
    .into_iter()
    .map(|(name, desc)| Parameter {
        name,
        desc,
        aliases: &[],
        ty: ParameterType::Byte,
        default: DefaultValue::None,
    })
    .collect()
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "background",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: color_params(),
            return_type: ParameterType::named("Attribute"),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_background"),
        }],
    });
}
