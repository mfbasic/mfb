//! `strings::byteLen` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

/// Target-generic native lowering for `strings.byteLen` (registry `Body::Native`
/// `common` slot): the byte length is the leading `u64` count word of the string
/// block, so a single load yields it.
pub(crate) fn lower(builder: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String> {
    if args.len() != 1 {
        return Err("strings.byteLen: no native lowering for these arguments".to_string());
    }
    let value = builder.lower_value(&args[0])?;
    builder.require_string("strings.byteLen value", &value)?;
    let register = builder.allocate_register()?;
    builder.emit(abi::load_u64(&register, &value.location, 0));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(register.render()),
        text: format!("strings.byteLen({})", value.text),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "byteLen",
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
