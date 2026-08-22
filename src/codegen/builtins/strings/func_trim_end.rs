//! `strings.trimEnd` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_trim;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    if args.len() != 1 {
        return Err("strings.trimEnd: no native lowering for these arguments".to_string());
    }
    gen_trim::lower_strings_trim(builder, &args[0], false, true)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "trimEnd",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native(None, None, Some(lower)),
        }],
    });
}
