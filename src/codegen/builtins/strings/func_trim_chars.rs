//! `strings.trimChars` — descriptor + clean-room native lowering.

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
        return Err("strings.trimChars: no native lowering for these arguments".to_string());
    }
    let value = &args[0];
    let chars = &args[1];

    let scratch16 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let scratch23 = builder.temporary_vreg();
    let value = value.clone();
    builder.require_string("strings.trimChars value", &value)?;
    let value_slot = builder.spill_to_slot("strings_trim_chars_value", &value.location);
    let chars = chars.clone();
    builder.require_string("strings.trimChars chars", &chars)?;
    let chars_slot = builder.spill_to_slot("strings_trim_chars_chars", &chars.location);
    let start_slot = builder.allocate_stack_object("strings_trim_chars_start", 8);
    let end_slot = builder.allocate_stack_object("strings_trim_chars_end", 8);

    // start = 0, end = valueLen.
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::move_immediate(&scratch10, "Integer", "0"));
    builder.emit(abi::store_u64(&scratch10, abi::stack_pointer(), start_slot));
    builder.emit(abi::store_u64(&scratch9, abi::stack_pointer(), end_slot));

    // Leading trim: while start < end, take scalar [start, scalarEnd); if it is
    // in the chars set, set start = scalarEnd, else stop.
    {
        let loop_label = builder.label("strings_trim_chars_lead_loop");
        let done = builder.label("strings_trim_chars_lead_done");
        builder.emit(abi::label(&loop_label));
        builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), start_slot));
        builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), end_slot));
        builder.emit(abi::compare_registers(&scratch10, &scratch11));
        builder.emit(abi::branch_ge(&done));
        // scalar bytes: [x10, x12) where x12 = scalarEnd (advance one lead +
        // continuation bytes).
        builder.emit(abi::add_immediate(&scratch14, &scratch16, 8));
        builder.emit(abi::add_registers(&scratch14, &scratch14, &scratch10)); // scalar start ptr
        builder.emit(abi::move_register(&scratch12, &scratch10));
        builder.emit(abi::add_immediate(&scratch12, &scratch12, 1));
        builder.emit(abi::move_immediate(&scratch17, "Integer", "192"));
        let cont = builder.label("strings_trim_chars_lead_cont");
        let cont_done = builder.label("strings_trim_chars_lead_cont_done");
        builder.emit(abi::label(&cont));
        builder.emit(abi::compare_registers(&scratch12, &scratch11));
        builder.emit(abi::branch_ge(&cont_done));
        builder.emit(abi::add_immediate(&scratch15, &scratch16, 8));
        builder.emit(abi::add_registers(&scratch15, &scratch15, &scratch12));
        builder.emit(abi::load_u8(&scratch13, &scratch15, 0));
        builder.emit(abi::and_registers(&scratch13, &scratch13, &scratch17));
        builder.emit(abi::compare_immediate(&scratch13, "128"));
        builder.emit(abi::branch_ne(&cont_done));
        builder.emit(abi::add_immediate(&scratch12, &scratch12, 1));
        builder.emit(abi::branch(&cont));
        builder.emit(abi::label(&cont_done));
        // scalar byte length = x12 - x10, ptr = x14.
        builder.emit(abi::subtract_registers(&scratch23, &scratch12, &scratch10));
        let in_set = builder.label("strings_trim_chars_lead_in_set");
        let not_in_set = builder.label("strings_trim_chars_lead_not_in_set");
        builder.emit_chars_set_contains_branch(
            &scratch14,
            &scratch23,
            chars_slot,
            &in_set,
            &not_in_set,
        );
        builder.emit(abi::label(&not_in_set));
        builder.emit(abi::branch(&done));
        builder.emit(abi::label(&in_set));
        builder.emit(abi::store_u64(&scratch12, abi::stack_pointer(), start_slot));
        builder.emit(abi::branch(&loop_label));
        builder.emit(abi::label(&done));
    }

    // Trailing trim: while end > start, take the last scalar [scalarStart, end);
    // if in set, end = scalarStart, else stop.
    {
        let loop_label = builder.label("strings_trim_chars_trail_loop");
        let done = builder.label("strings_trim_chars_trail_done");
        builder.emit(abi::label(&loop_label));
        builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), start_slot));
        builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), end_slot));
        builder.emit(abi::compare_registers(&scratch11, &scratch10));
        builder.emit(abi::branch_le(&done));
        // find scalar start: step back from end over continuation bytes.
        builder.emit(abi::move_register(&scratch12, &scratch11));
        builder.emit(abi::move_immediate(&scratch17, "Integer", "192"));
        let back = builder.label("strings_trim_chars_trail_back");
        let back_done = builder.label("strings_trim_chars_trail_back_done");
        builder.emit(abi::label(&back));
        builder.emit(abi::subtract_immediate(&scratch12, &scratch12, 1));
        builder.emit(abi::compare_registers(&scratch12, &scratch10));
        builder.emit(abi::branch_le(&back_done));
        builder.emit(abi::add_immediate(&scratch15, &scratch16, 8));
        builder.emit(abi::add_registers(&scratch15, &scratch15, &scratch12));
        builder.emit(abi::load_u8(&scratch13, &scratch15, 0));
        builder.emit(abi::and_registers(&scratch13, &scratch13, &scratch17));
        builder.emit(abi::compare_immediate(&scratch13, "128"));
        builder.emit(abi::branch_eq(&back));
        builder.emit(abi::label(&back_done));
        // scalar = [x12, x11), ptr = value+8+x12, len = x11 - x12.
        builder.emit(abi::add_immediate(&scratch14, &scratch16, 8));
        builder.emit(abi::add_registers(&scratch14, &scratch14, &scratch12));
        builder.emit(abi::subtract_registers(&scratch23, &scratch11, &scratch12));
        let in_set = builder.label("strings_trim_chars_trail_in_set");
        let not_in_set = builder.label("strings_trim_chars_trail_not_in_set");
        builder.emit_chars_set_contains_branch(
            &scratch14,
            &scratch23,
            chars_slot,
            &in_set,
            &not_in_set,
        );
        builder.emit(abi::label(&not_in_set));
        builder.emit(abi::branch(&done));
        builder.emit(abi::label(&in_set));
        builder.emit(abi::store_u64(&scratch12, abi::stack_pointer(), end_slot));
        builder.emit(abi::branch(&loop_label));
        builder.emit(abi::label(&done));
    }

    // Build result from [start, end).
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), start_slot));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), end_slot));
    builder.emit(abi::subtract_registers(&scratch12, &scratch11, &scratch10));
    builder.emit(abi::add_immediate(&scratch13, &scratch16, 8));
    builder.emit(abi::add_registers(&scratch13, &scratch13, &scratch10));
    let result = builder.emit_materialize_string_from_bytes(&scratch13, &scratch12)?;
    Ok(ValueResult {
        origin: None,
        type_: "String".to_string(),
        location: Operand::from(result.render()),
        text: "strings.trimChars".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "trimChars",
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
                    name: "chars",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
