//! `strings.padLeft` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_pad;
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
    if !(args.len() == 2 || args.len() == 3) {
        return Err("strings.padLeft: no native lowering for these arguments".to_string());
    }
    gen_pad::lower_strings_pad(builder, args, false)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "padLeft",
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
                    name: "width",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "padChar",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Optional,
                },
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline_self(lower),
        }],
    });
}
