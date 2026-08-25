//! `astrings::italic` — `Attribute`-model flag constructor (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_italic` FUNC through the registry's `rewrite_target`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Construct the italic flag `Attribute`."#;

const DESC: &str = r#"`italic` returns an `Attribute` wrapping the `AttrFlag` with `kind` `AttrTypeFlag.Italic`. Pass it to
`astrings::addAttribute` to mark a scalar range italic. A flag attribute carries no value — a scalar
is italic when any covering span carries the italic flag."#;

const EX: &str = r#"```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::italic())
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_italic() AS Attribute
  RETURN AttrFlag[AttrTypeFlag.Italic]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "italic",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named("Attribute"),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_italic"),
        }],
    });
}
