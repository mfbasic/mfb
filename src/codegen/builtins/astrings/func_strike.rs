//! `astrings::strike` — `Attribute`-model flag constructor (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_strike` FUNC through the registry's `rewrite_target`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Construct the strikethrough flag `Attribute`."#;

const DESC: &str = r#"`strike` returns an `Attribute` wrapping the `AttrFlag` with `kind` `AttrTypeFlag.Strike`. Pass it to
`astrings::addAttribute` to mark a scalar range struck through. A flag attribute carries no value — a
scalar is struck when any covering span carries the strike flag."#;

const EX: &str = r#"```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::strike())
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_strike() AS Attribute
  RETURN AttrFlag[AttrTypeFlag.Strike]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "strike",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named("Attribute"),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_strike"),
        }],
    });
}
