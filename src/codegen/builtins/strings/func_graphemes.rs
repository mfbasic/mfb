//! `strings::graphemes` — descriptor + native-lowering wrapper.
//!
//! The native lowering stays SHARED in `src/target/shared/code/builder_strings*`
//! (the string codegen carrier, kept in place like `vector`'s SIMD carrier); this
//! thin wrapper points the registry's `Body::Native` `common` slot at the shared
//! dispatcher `CodeBuilder::lower_strings_package_call`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

/// Target-generic native lowering for `strings.graphemes` (registry `Body::Native`
/// `common` slot), delegating to the shared string codegen carrier
/// (`CodeBuilder::lower_strings_package_call` in `src/target/shared/code`).
pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    builder
        .lower_strings_package_call("strings.graphemes", args)?
        .ok_or_else(|| "strings.graphemes: no native lowering for these arguments".to_string())
}

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "graphemes",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::String),
            errors: vec![],
            body: Body::native(None, None, Some(lower)),
        }],
    });
}
