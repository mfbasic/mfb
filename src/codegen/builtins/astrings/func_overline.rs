//! `astrings::overline` — `Attribute`-model flag constructor (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_overline` FUNC through the registry's `rewrite_target`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Construct the overline flag `Attribute`."#;

const DESC: &str = r#"`overline` returns an `Attribute` wrapping the `AttrFlag` with `kind` `AttrTypeFlag.Overline`. Pass it
to `astrings::addAttribute` to mark a scalar range overlined. A flag attribute carries no value — a
scalar is overlined when any covering span carries the overline flag."#;

const EX: &str = r#"```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::overline())
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_overline() AS Attribute
  RETURN AttrFlag[AttrTypeFlag.Overline]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "overline",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named("Attribute"),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_overline"),
        }],
    });
}
