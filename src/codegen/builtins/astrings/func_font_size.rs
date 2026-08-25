//! `astrings::fontSize` — Integer-valued `Attribute` constructor (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_fontSize` FUNC through the registry's `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Construct a font-size `Attribute`."#;

const DESC: &str = r#"`fontSize` returns an `Attribute` wrapping the `AttrNumber` with `kind` `AttrTypeNumber.FontSize` and
`value` `size`. Pass it to `astrings::addAttribute` to set the font size (e.g. in points) over a
scalar range. Font size is an Integer-valued attribute: overlapping font-size spans resolve by
higher-start-wins at read time."#;

const EX: &str = r#"```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::fontSize(14))
END SUB
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
                desc: "The font size.",
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
