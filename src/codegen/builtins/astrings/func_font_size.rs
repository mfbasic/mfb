//! `astrings::fontSize` — Integer-valued `Attribute` constructor (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_fontSize` FUNC through the registry's `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Construct a font-size `astrings::Attribute`."#;

const DESC: &str = r#"`fontSize` returns an `astrings::Attribute` wrapping the `astrings::AttrNumber` with `kind` `astrings::AttrTypeNumber.FontSize` and
`value` `size`. Pass it to `astrings::addAttribute` to set the font size (e.g. in points) over a
scalar range. Font size is an Integer-valued attribute. Where font-size spans overlap, the one with the higher
start wins at read time; when their starts are equal, the one added later wins."#;

const EX: &str = r#"```
IMPORT astrings

FUNC main() AS Nothing
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::fontSize(14))
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_fontSize(size AS Integer) AS Attribute
  RETURN AttrNumber[AttrTypeNumber.FontSize, size]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fontSize",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "size",
                desc: "The font size in points. Any Integer is accepted and kept as given, including zero and negative values.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named("Attribute"),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_fontSize"),
        }],
    });
}
