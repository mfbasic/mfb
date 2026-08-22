//! `strings.endsWith` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::nir::*;
use crate::types::ParameterType;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.endsWith: no native lowering for these arguments".to_string());
    }
    let value = &args[0];
    let suffix = &args[1];

    let value = builder.lower_value(value)?;
    builder.require_string("strings.endsWith value", &value)?;
    let value_slot = builder.spill_to_slot("strings_ends_with_value", &value.location);
    let suffix = builder.lower_value(suffix)?;
    builder.require_string("strings.endsWith suffix", &suffix)?;
    let suffix_slot = builder.spill_to_slot("strings_ends_with_suffix", &suffix.location);
    builder.lower_string_prefix_predicate("strings.endsWith", value_slot, suffix_slot, true)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "endsWith",
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
                    name: "suffix",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_inline_self(lower),
        }],
    });
}
