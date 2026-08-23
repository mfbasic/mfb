//! `astrings::scalarLen` — internal-only native overlay bridge (`Body::abi_inline_self`).
//!
//! Never user-callable (`internal_only`): the source companion (`package.mfb`, an
//! `internal` file) reads the visible scalar count of an opaque `AttributedString`
//! through this native primitive for inclusive-range bounds validation. The lowering
//! stays SHARED in `src/codegen/builtins/astrings/gen_astrings.rs`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
/// Self-lowering inline body for `astrings.scalarLen` (`Body::abi_inline_self`),
/// delegating to the shared `AttributedString` codegen carrier.
pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder
        .lower_astrings_package_call("astrings.scalarLen", args)?
        .ok_or_else(|| "astrings.scalarLen: no native lowering for these arguments".to_string())
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "scalarLen",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "",
                aliases: &[],
                ty: ParameterType::Named("AttributedString"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline_self(lower),
        }],
    });
}
