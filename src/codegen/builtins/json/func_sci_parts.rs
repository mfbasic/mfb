//! `json::sciParts` — the significant-digit stream `__json_stringifyNumber`
//! renders from (plan-120-G).
//!
//! Internal: it has no `mfb man` page and a program cannot call it, exactly as
//! `strings::genCat` is internal to the string predicates. It exists because
//! MFBASIC has no way to reach a double's significant digits — `toString` gives
//! fixed-point places and caps at 255 of them, which cannot express the leading
//! digits of a subnormal.
//!
//! Returns `"<sticky><18 digits>e<exponent>"` for the magnitude of `value`;
//! see `float_format_sci.rs` for why it truncates rather than rounds.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let value = builder.materialize_float(args[0].clone())?;
    let bits = builder.allocate_register();
    builder.emit(abi::move_register(&bits, &value.location));
    // Magnitude only: the sign is the caller's to place, and the helper's
    // digit stream is defined on |v|.
    let mask = builder.allocate_register();
    builder.emit(abi::move_immediate(&mask, "Integer", "9223372036854775807"));
    builder.emit(abi::and_registers(&bits, &bits, &mask));
    builder.emit_sci_parts_call(&bits)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sciParts",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "",
                aliases: &[],
                ty: ParameterType::Float,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
