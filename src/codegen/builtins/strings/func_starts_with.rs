//! `strings::startsWith` — descriptor + native-lowering wrapper.
//!
//! The native lowering stays SHARED in `src/target/shared/code/builder_strings*`
//! (the string codegen carrier, kept in place like `vector`'s SIMD carrier); this
//! thin wrapper points the registry's `Body::Native` `common` slot at the shared
//! dispatcher `CodeBuilder::lower_strings_package_call`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
/// Target-generic native lowering for `strings.startsWith` (registry `Body::Native`
/// `common` slot), delegating to the shared string codegen carrier
/// (`CodeBuilder::lower_strings_package_call` in `src/target/shared/code`).
pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    builder
        .lower_strings_package_call("strings.startsWith", args)?
        .ok_or_else(|| "strings.startsWith: no native lowering for these arguments".to_string())
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "startsWith",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "prefix",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::native(None, None, Some(lower)),
        }],
    });
}
