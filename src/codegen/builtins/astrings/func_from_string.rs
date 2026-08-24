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
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "text",
                desc: "",
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
