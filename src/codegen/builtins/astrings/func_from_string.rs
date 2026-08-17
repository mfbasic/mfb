//! `astrings::fromString` — native-direct constructor (`Body::Native` `common`).
//!
//! The native lowering stays SHARED in `src/target/shared/code/builder_astrings.rs`
//! (the `AttributedString` codegen carrier, kept in place like `vector`'s SIMD
//! carrier and `strings`' string carrier); this thin wrapper points the registry's
//! `Body::Native` `common` slot at the shared dispatcher
//! `CodeBuilder::lower_astrings_package_call`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

/// Target-generic native lowering for `astrings.fromString` (registry `Body::Native`
/// `common` slot), delegating to the shared `AttributedString` codegen carrier
/// (`CodeBuilder::lower_astrings_package_call` in `src/target/shared/code`).
pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    builder
        .lower_astrings_package_call("astrings.fromString", args)?
        .ok_or_else(|| "astrings.fromString: no native lowering for these arguments".to_string())
}

pub(super) fn register(pkg: &mut RegistryPackage) {
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
            return_type: ParameterType::Named("AttributedString"),
            errors: vec![],
            body: Body::native(None, None, Some(lower)),
        }],
    });
}
