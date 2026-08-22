//! `strings.startsWith` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::nir::*;
use crate::types::ParameterType;

pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.startsWith: no native lowering for these arguments".to_string());
    }
    let value = &args[0];
    let prefix = &args[1];

    let value = builder.lower_value(value)?;
    builder.require_string("strings.startsWith value", &value)?;
    let value_slot = builder.spill_to_slot("strings_starts_with_value", &value.location);
    let prefix = builder.lower_value(prefix)?;
    builder.require_string("strings.startsWith prefix", &prefix)?;
    let prefix_slot = builder.spill_to_slot("strings_starts_with_prefix", &prefix.location);
    builder.lower_string_prefix_predicate("strings.startsWith", value_slot, prefix_slot, false)
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
