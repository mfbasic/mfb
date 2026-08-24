//! `strings.count` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.count: no native lowering for these arguments".to_string());
    }
    let value = &args[0];
    let needle = &args[1];

    let scratch16 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch23 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let value = value.clone();
    builder.require_string("strings.count value", &value)?;
    let value_slot = builder.spill_to_slot("strings_count_value", &value.location);
    let needle = needle.clone();
    builder.require_string("strings.count needle", &needle)?;
    let needle_slot = builder.spill_to_slot("strings_count_needle", &needle.location);
    let count_slot = builder.allocate_stack_object("strings_count_result", 8);

    let invalid = builder.label("strings_count_invalid");
    let loop_label = builder.label("strings_count_loop");
    let match_label = builder.label("strings_count_match");
    let no_match = builder.label("strings_count_no_match");
    let done = builder.label("strings_count_done");

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch17, abi::stack_pointer(), needle_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::load_u64(&scratch10, &scratch17, 0));
    builder.emit(abi::compare_immediate(&scratch10, "0"));
    builder.emit(abi::branch_eq(&invalid));
    // x11 = value data, x12 = needle data, x14 = cursor index, x19 = count.
    builder.emit(abi::add_immediate(&scratch11, &scratch16, 8));
    builder.emit(abi::add_immediate(&scratch12, &scratch17, 8));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch14, "Integer", "0"));
    // needle longer than value -> 0 occurrences, before the unsigned
    // valueLen - needleLen below underflows and the loop reads past the
    // value buffer (audit-unicode #4); same guard shape as `contains`.
    builder.emit(abi::compare_registers(&scratch10, &scratch9));
    builder.emit(abi::branch_hi(&done));
    builder.emit(abi::label(&loop_label));
    // need x14 + needleLen <= valueLen, i.e. cursor <= valueLen - needleLen.
    builder.emit(abi::subtract_registers(&scratch13, &scratch9, &scratch10));
    builder.emit(abi::compare_registers(&scratch14, &scratch13));
    builder.emit(abi::branch_hi(&done));
    builder.emit(abi::add_registers(&scratch15, &scratch11, &scratch14));
    builder.emit_string_byte_range_equal_branch(
        &scratch15,
        &scratch12,
        &scratch10,
        &match_label,
        &no_match,
    );
    builder.emit(abi::label(&match_label));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    // non-overlapping: advance past the whole needle.
    builder.emit(abi::add_registers(&scratch14, &scratch14, &scratch10));
    builder.emit(abi::branch(&loop_label));
    builder.emit(abi::label(&no_match));
    builder.emit(abi::add_immediate(&scratch14, &scratch14, 1));
    builder.emit(abi::branch(&loop_label));
    builder.emit(abi::label(&done));
    builder.emit(abi::store_u64(&scratch23, abi::stack_pointer(), count_slot));
    let result = builder.allocate_register()?;
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), count_slot));
    let after = builder.label("strings_count_after");
    builder.emit(abi::branch(&after));
    builder.emit(abi::label(&invalid));
    builder.raise_error("strings.count", "ErrInvalidArgument")?;
    builder.emit(abi::label(&after));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(result.render()),
        text: "strings.count".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "count",
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
                    name: "needle",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_inline(lower),
        }],
    });
}
