//! `strings.graphemesCount` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_graphemes::lower_strings_graphemes;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    if args.len() != 1 {
        return Err("strings.graphemesCount: no native lowering for these arguments".to_string());
    }
    let value = &args[0];

    let scratch16 = builder.temporary_vreg();
    let list = lower_strings_graphemes(builder, value)?;
    let list_slot = builder.spill_to_slot("strings_graphemes_count_list", &list.location);
    let result = builder.allocate_register()?;
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), list_slot));
    builder.emit(abi::load_u64(&result, &scratch16, COLLECTION_OFFSET_COUNT));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(result.render()),
        text: "strings.graphemesCount".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "graphemesCount",
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
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(None, None, Some(lower)),
        }],
    });
}
