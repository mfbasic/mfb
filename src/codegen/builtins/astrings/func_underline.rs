//! `astrings::underline` — `Attribute`-model flag constructor (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_underline` FUNC through the registry's `rewrite_target`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Construct the underline flag `astrings::Attribute`."#;

const DESC: &str = r#"`underline` returns an `astrings::Attribute` wrapping the `astrings::AttrFlag` with `kind` `astrings::AttrTypeFlag.Underline`. Pass
it to `astrings::addAttribute` to mark a scalar range underlined. A flag attribute carries no value —
a scalar is underlined when any covering span carries the underline flag."#;

const EX: &str = r#"```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::underline())
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_underline() AS Attribute
  RETURN AttrFlag[AttrTypeFlag.Underline]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "underline",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named("Attribute"),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_underline"),
        }],
    });
}
