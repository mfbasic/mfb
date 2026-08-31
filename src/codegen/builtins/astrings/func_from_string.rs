//! `astrings::fromString` — native-direct constructor (`Body::abi_inline`).
//!
//! The native lowering stays SHARED in `src/codegen/builtins/astrings/gen_astrings.rs`
//! (the `AttributedString` codegen carrier); this thin wrapper points the registry's
//! `Body::abi_inline` at the shared dispatcher
//! `CodeBuilder::lower_astrings_package_call`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Construct an `AttributedString` from plain text with no attributes."#;

const DESC: &str = r#"`fromString` builds an `AttributedString` whose visible text is its own copy of
`text` and whose attribute overlay is empty. The result is an ordinary value:
changing `text` afterwards cannot affect it, and it goes away with the scope
that holds it.

Recover the visible text with `toString(a)`; `io::print`/`io::write` emit it. The
constructed value has no attributes until `astrings::addAttribute` (and the other
mutation members) records some."#;

const EX: &str = r#"Build an attributed string and print its visible text:

```
IMPORT astrings
IMPORT io

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  io::print(toString(a))
END SUB
```"#;
/// Self-lowering inline body for `astrings.fromString` (`Body::abi_inline`),
/// delegating to the shared `AttributedString` codegen carrier
/// (`CodeBuilder::lower_astrings_package_call` in `gen_astrings.rs`). Type-aware over
/// its raw `NirValue` args, so it lowers them itself.
pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder
        .lower_astrings_package_call("astrings.fromString", args)?
        .ok_or_else(|| "astrings.fromString: no native lowering for these arguments".to_string())
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fromString",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "text",
                desc: "The visible text. Copied into the new value.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named("AttributedString"),
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
