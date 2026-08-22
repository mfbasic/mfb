//! `strings.startsWithAny` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_with_any;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.startsWithAny: no native lowering for these arguments".to_string());
    }
    gen_with_any::lower_strings_with_any(builder, &args[0], &args[1], false)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "startsWithAny",
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
                    name: "prefixes",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::String),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::native(None, None, Some(lower)),
        }],
    });
}
