//! `astrings::font` — String-valued `Attribute` constructor (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_font` FUNC through the registry's `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Construct a font-family `astrings::Attribute`."#;

const DESC: &str = r#"`font` returns an `astrings::Attribute` wrapping the `astrings::AttrText` with `kind` `astrings::AttrTypeText.Font` and `value`
`name`. Pass it to `astrings::addAttribute` to set the font family over a scalar range. Font is a
String-valued attribute. Where font spans overlap, the one with the higher start wins at read
time; when their starts are equal, the one added later wins."#;

const EX: &str = r#"```
IMPORT astrings

FUNC main() AS Nothing
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::font("Serif"))
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_font(name AS String) AS Attribute
  RETURN AttrText[AttrTypeText.Font, name]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "font",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "name",
                desc: "The font family name, stored as given; it may be empty.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named("Attribute"),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_font"),
        }],
    });
}
