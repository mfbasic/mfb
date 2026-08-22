//! `strings.lower` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_case_map;
use crate::codegen::builtins::strings::UnicodeCaseMap;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    if let Some(value) = builder.static_strings_package_string("strings.lower", args)? {
        let register = builder.load_string_constant(&value)?;
        return Ok(ValueResult {
            type_: "String".to_string(),
            location: Operand::from(register.render()),
            text: "strings.lower".to_string(),
        });
    }
    if args.len() != 1 {
        return Err("strings.lower: no native lowering for these arguments".to_string());
    }
    gen_case_map::lower_strings_case_map(builder, &args[0], UnicodeCaseMap::Lower)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "lower",
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
