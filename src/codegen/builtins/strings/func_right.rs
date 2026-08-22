//! `strings.right` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_left_right;
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
    if args.len() != 2 {
        return Err("strings.right: no native lowering for these arguments".to_string());
    }
    gen_left_right::lower_strings_left_right(builder, &args[0], &args[1], true)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "right",
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
                    name: "count",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline_self(lower),
        }],
    });
}
