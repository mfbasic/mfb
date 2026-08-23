//! `strings.join` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
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
        return Err("strings.join: no native lowering for these arguments".to_string());
    }
    let parts = &args[0];
    let delimiter = &args[1];

    let scratch16 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let parts = parts.clone();
    if list_element_type(&parts.type_).as_deref() != Some("String") {
        return Err(format!(
            "strings.join parts must be List OF String, got {}",
            parts.type_
        ));
    }
    let parts_slot = builder.spill_to_slot("strings_join_parts", &parts.location);
    let delimiter = delimiter.clone();
    builder.require_string("strings.join delimiter", &delimiter)?;
    let delimiter_slot = builder.spill_to_slot("strings_join_delimiter", &delimiter.location);
    let output_len_slot = builder.allocate_stack_object("strings_join_output_len", 8);
    let result_slot = builder.allocate_stack_object("strings_join_result", 8);
    let length_loop = builder.label("strings_join_length_loop");
    let length_no_delim = builder.label("strings_join_length_no_delim");
    let length_done = builder.label("strings_join_length_done");
    let alloc_ok = builder.label("strings_join_alloc_ok");
    let overflow = builder.label("strings_join_overflow");
    let copy_loop = builder.label("strings_join_copy_loop");
    let copy_no_delim = builder.label("strings_join_copy_no_delim");
    let copy_done = builder.label("strings_join_copy_done");

    // Copy-loop scratch as vregs, so the allocator colors them per-ISA. They
    // must not be pinned to a role: a Ret- or argument-role register is a
    // distinct physical register per backend and collides on x86-64.
    let cursor_v = builder.temporary_vreg();
    let remaining_v = builder.temporary_vreg();
    let byte_v = builder.temporary_vreg();
    let cursor = &cursor_v;
    let remaining = &remaining_v;
    let byte = &byte_v;

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), parts_slot));
    builder.emit(abi::load_u64(
        &scratch17,
        abi::stack_pointer(),
        delimiter_slot,
    ));
    builder.emit(abi::load_u64(
        &scratch9,
        &scratch16,
        COLLECTION_OFFSET_COUNT,
    ));
    builder.emit(abi::load_u64(&scratch10, &scratch17, 0));
    builder.emit(abi::move_immediate(&scratch11, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch12, "Integer", "0"));
    builder.emit(abi::add_immediate(
        &scratch13,
        &scratch16,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::label(&length_loop));
    builder.emit(abi::compare_registers(&scratch12, &scratch9));
    builder.emit(abi::branch_ge(&length_done));
    builder.emit(abi::compare_immediate(&scratch12, "0"));
    builder.emit(abi::branch_eq(&length_no_delim));
    // output_len += delim_len (between parts) then += part_len; trap a 64-bit
    // wrap so the copy pass cannot overrun the (undersized) allocation (bug-60).
    builder.emit_checked_size_add(&scratch11, &scratch11, &scratch10, &overflow);
    builder.emit(abi::label(&length_no_delim));
    builder.emit(abi::load_u64(
        &scratch14,
        &scratch13,
        COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
    ));
    builder.emit_checked_size_add(&scratch11, &scratch11, &scratch14, &overflow);
    builder.emit(abi::add_immediate(
        &scratch13,
        &scratch13,
        COLLECTION_ENTRY_SIZE,
    ));
    builder.emit(abi::add_immediate(&scratch12, &scratch12, 1));
    builder.emit(abi::branch(&length_loop));
    builder.emit(abi::label(&length_done));
    builder.emit(abi::store_u64(
        &scratch11,
        abi::stack_pointer(),
        output_len_slot,
    ));

    // allocate output_len + 9 (block header), trapping the header add's wrap.
    builder.emit_checked_size_add_immediate(abi::return_register(), &scratch11, 9, &overflow);
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    // A size wrap reports the same 77010001 an impossible allocation would
    // (x0 does not hold an error code before the call, so the register-based
    // return above cannot be shared). The checked-size helper deposits the
    // partially-computed size into the return register before branching here,
    // so `emit_allocation_error_return` would surface that size as the error
    // code (bug-60 detection, bug-352 code fix).
    builder.emit(abi::label(&overflow));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    builder.emit(abi::load_u64(
        &scratch11,
        abi::stack_pointer(),
        output_len_slot,
    ));
    builder.emit(abi::store_u64(&scratch11, abi::mfb_return(1), 0));

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), parts_slot));
    builder.emit(abi::load_u64(
        &scratch17,
        abi::stack_pointer(),
        delimiter_slot,
    ));
    builder.emit(abi::load_u64(
        &scratch9,
        &scratch16,
        COLLECTION_OFFSET_COUNT,
    ));
    builder.emit(abi::load_u64(&scratch10, &scratch17, 0));
    builder.emit(abi::add_immediate(&scratch11, &scratch17, 8));
    // Carry the result pointer in a vreg, not physical x1 (a reload with no
    // call context maps unreliably on x86; the concat/split pattern).
    let out_ptr_v = builder.temporary_vreg();
    let out_ptr = &out_ptr_v;
    builder.emit(abi::load_u64(out_ptr, abi::stack_pointer(), result_slot));
    builder.emit(abi::add_immediate(&scratch13, out_ptr, 8));
    builder.emit_collection_data_pointer_for(&scratch14, &scratch16, "String");
    builder.emit(abi::add_immediate(
        &scratch15,
        &scratch16,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::move_immediate(&scratch12, "Integer", "0"));
    builder.emit(abi::label(&copy_loop));
    builder.emit(abi::compare_registers(&scratch12, &scratch9));
    builder.emit(abi::branch_ge(&copy_done));
    builder.emit(abi::compare_immediate(&scratch12, "0"));
    builder.emit(abi::branch_eq(&copy_no_delim));
    builder.emit(abi::move_register(cursor, &scratch11));
    builder.emit(abi::move_register(remaining, &scratch10));
    // plan-86 F2: 8-byte word-copy (+ byte tail) of the delimiter into scratch13.
    builder.emit_block_copy_advance(&scratch13, cursor, remaining, byte, "strings_join_delim");
    builder.emit(abi::label(&copy_no_delim));
    builder.emit(abi::load_u64(
        cursor,
        &scratch15,
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
    ));
    builder.emit(abi::load_u64(
        remaining,
        &scratch15,
        COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
    ));
    builder.emit(abi::add_registers(cursor, &scratch14, cursor));
    // plan-86 F2: 8-byte word-copy (+ byte tail) of the value into scratch13.
    builder.emit_block_copy_advance(&scratch13, cursor, remaining, byte, "strings_join_value");
    builder.emit(abi::add_immediate(
        &scratch15,
        &scratch15,
        COLLECTION_ENTRY_SIZE,
    ));
    builder.emit(abi::add_immediate(&scratch12, &scratch12, 1));
    builder.emit(abi::branch(&copy_loop));
    builder.emit(abi::label(&copy_done));
    builder.emit(abi::move_immediate(byte, "Integer", "0"));
    builder.emit(abi::store_u8(byte, &scratch13, 0));
    let result = builder.allocate_register()?;
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    Ok(ValueResult {
        origin: None,
        type_: "String".to_string(),
        location: Operand::from(result.render()),
        text: "strings.join".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "join",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "parts",
                    desc: "",
                    aliases: &["values"],
                    ty: ParameterType::list_of(ParameterType::String),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "delimiter",
                    desc: "",
                    aliases: &["separator"],
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
