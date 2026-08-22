//! `strings.graphemes` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_graphemes;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() == 1 {
        if let Some(value) = builder.static_string_value(&args[0]) {
            let values = crate::unicode::backend::graphemes(&value)
                .into_iter()
                .map(|value| NirValue::Const {
                    type_: "String".to_string(),
                    value,
                })
                .collect::<Vec<_>>();
            return builder.lower_list_literal("List OF String", &values);
        }
    }
    if args.len() != 1 {
        return Err("strings.graphemes: no native lowering for these arguments".to_string());
    }
    gen_graphemes::lower_strings_graphemes(builder, &args[0])
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "graphemes",
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
            return_type: ParameterType::list_of(ParameterType::String),
            errors: vec![],
            body: Body::abi_inline_self(lower),
        }],
    });
}
