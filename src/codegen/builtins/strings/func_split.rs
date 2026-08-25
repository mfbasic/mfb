//! `strings.split` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
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
        return Err("strings.split: no native lowering for these arguments".to_string());
    }
    let value = &args[0];
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
    let scratch20 = builder.temporary_vreg();
    let scratch21 = builder.temporary_vreg();
    let scratch22 = builder.temporary_vreg();
    let scratch23 = builder.temporary_vreg();
    let scratch24 = builder.temporary_vreg();
    let value = value.clone();
    builder.require_string("strings.split value", &value)?;
    let value_slot = builder.spill_to_slot("strings_split_value", &value.location);
    let delimiter = delimiter.clone();
    builder.require_string("strings.split delimiter", &delimiter)?;
    let delimiter_slot = builder.spill_to_slot("strings_split_delimiter", &delimiter.location);
    let count_slot = builder.allocate_stack_object("strings_split_count", 8);
    let data_len_slot = builder.allocate_stack_object("strings_split_data_len", 8);
    let result_slot = builder.allocate_stack_object("strings_split_result", 8);
    let layout = CollectionTypeLayout::from_type(&ParameterType::list_of(ParameterType::String))
        .ok_or_else(|| "native strings.split cannot resolve List OF String layout".to_string())?;

    let invalid_delimiter = builder.label("strings_split_invalid_delimiter");
    let length_loop = builder.label("strings_split_length_loop");
    let length_compare = builder.label("strings_split_length_compare");
    let length_match = builder.label("strings_split_length_match");
    let length_next = builder.label("strings_split_length_next");
    let length_done = builder.label("strings_split_length_done");
    let alloc_ok = builder.label("strings_split_alloc_ok");
    let write_loop = builder.label("strings_split_write_loop");
    let write_compare = builder.label("strings_split_write_compare");
    let write_match = builder.label("strings_split_write_match");
    let write_next = builder.label("strings_split_write_next");
    let write_final = builder.label("strings_split_write_final");
    let write_done = builder.label("strings_split_write_done");
    let done = builder.label("strings_split_done");

    // Inner delimiter-scan scratch as vregs, so the allocator colors them
    // per-ISA rather than colliding with the x86-64 ABI argument registers.
    let scan_i_v = builder.temporary_vreg();
    let scan_ptr_v = builder.temporary_vreg();
    let delim_ptr_v = builder.temporary_vreg();
    let sbyte_v = builder.temporary_vreg();
    let dbyte_v = builder.temporary_vreg();
    let scan_i = &scan_i_v;
    let scan_ptr = &scan_ptr_v;
    let delim_ptr = &delim_ptr_v;
    let sbyte = &sbyte_v;
    let dbyte = &dbyte_v;

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(
        &scratch17,
        abi::stack_pointer(),
        delimiter_slot,
    ));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::load_u64(&scratch10, &scratch17, 0));
    builder.emit(abi::compare_immediate(&scratch10, "0"));
    builder.emit(abi::branch_eq(&invalid_delimiter));
    builder.emit(abi::move_immediate(&scratch11, "Integer", "1"));
    builder.emit(abi::store_u64(&scratch11, abi::stack_pointer(), count_slot));
    builder.emit(abi::store_u64(
        &scratch9,
        abi::stack_pointer(),
        data_len_slot,
    ));
    builder.emit(abi::compare_registers(&scratch10, &scratch9));
    builder.emit(abi::branch_hi(&length_done));
    builder.emit(abi::subtract_registers(&scratch12, &scratch9, &scratch10));
    builder.emit(abi::move_immediate(&scratch13, "Integer", "0"));
    builder.emit(abi::add_immediate(&scratch14, &scratch16, 8));
    builder.emit(abi::add_immediate(&scratch15, &scratch17, 8));
    builder.emit(abi::label(&length_loop));
    builder.emit(abi::compare_registers(&scratch13, &scratch12));
    builder.emit(abi::branch_hi(&length_done));
    builder.emit(abi::move_immediate(scan_i, "Integer", "0"));
    builder.emit(abi::add_registers(scan_ptr, &scratch14, &scratch13));
    builder.emit(abi::move_register(delim_ptr, &scratch15));
    builder.emit(abi::label(&length_compare));
    builder.emit(abi::compare_registers(scan_i, &scratch10));
    builder.emit(abi::branch_eq(&length_match));
    builder.emit(abi::load_u8(sbyte, scan_ptr, 0));
    builder.emit(abi::load_u8(dbyte, delim_ptr, 0));
    builder.emit(abi::compare_registers(sbyte, dbyte));
    builder.emit(abi::branch_ne(&length_next));
    builder.emit(abi::add_immediate(scan_i, scan_i, 1));
    builder.emit(abi::add_immediate(scan_ptr, scan_ptr, 1));
    builder.emit(abi::add_immediate(delim_ptr, delim_ptr, 1));
    builder.emit(abi::branch(&length_compare));
    builder.emit(abi::label(&length_match));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), count_slot));
    builder.emit(abi::add_immediate(&scratch11, &scratch11, 1));
    builder.emit(abi::store_u64(&scratch11, abi::stack_pointer(), count_slot));
    builder.emit(abi::load_u64(
        &scratch11,
        abi::stack_pointer(),
        data_len_slot,
    ));
    builder.emit(abi::subtract_registers(&scratch11, &scratch11, &scratch10));
    builder.emit(abi::store_u64(
        &scratch11,
        abi::stack_pointer(),
        data_len_slot,
    ));
    builder.emit(abi::add_registers(&scratch13, &scratch13, &scratch10));
    builder.emit(abi::branch(&length_loop));
    builder.emit(abi::label(&length_next));
    builder.emit(abi::add_immediate(&scratch13, &scratch13, 1));
    builder.emit(abi::branch(&length_loop));
    builder.emit(abi::label(&length_done));

    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), count_slot));
    builder.emit(abi::load_u64(
        &scratch12,
        abi::stack_pointer(),
        data_len_slot,
    ));
    builder.emit(abi::move_immediate(
        &scratch13,
        "Integer",
        &COLLECTION_ENTRY_SIZE.to_string(),
    ));
    // bug-175 B: the split result size (count * entry + header + data bytes,
    // where `count` is the most expansion-prone term) is routed through the
    // checked helpers so an adversarial input cannot wrap the allocation size,
    // matching graphemes/to_bytes/nfc/replace/join.
    let size_overflow = builder.label("strings_split_size_overflow");
    builder.emit_checked_size_multiply(&scratch13, &scratch13, &scratch11, &size_overflow);
    builder.emit_checked_size_add_immediate(
        abi::return_register(),
        &scratch13,
        COLLECTION_HEADER_SIZE,
        &size_overflow,
    );
    builder.emit_checked_size_add(
        abi::return_register(),
        abi::return_register(),
        &scratch12,
        &size_overflow,
    );
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&size_overflow));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), count_slot));
    builder.emit(abi::load_u64(
        &scratch12,
        abi::stack_pointer(),
        data_len_slot,
    ));
    builder.emit_write_list_header_from_registers(
        &layout,
        abi::mfb_return(1),
        &scratch11,
        &scratch12,
    );

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(
        &scratch17,
        abi::stack_pointer(),
        delimiter_slot,
    ));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::load_u64(&scratch10, &scratch17, 0));
    builder.emit(abi::add_immediate(&scratch14, &scratch16, 8));
    builder.emit(abi::add_immediate(&scratch15, &scratch17, 8));
    // Carry the list pointer in a vreg, not physical x1 (a reload with no
    // call context maps unreliably on x86; the concat/repeat pattern).
    let list_ptr_v = builder.temporary_vreg();
    let list_ptr = &list_ptr_v;
    builder.emit(abi::load_u64(list_ptr, abi::stack_pointer(), result_slot));
    builder.emit(abi::add_immediate(
        &scratch20,
        list_ptr,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit_collection_data_pointer_for(&scratch21, list_ptr, "String");
    builder.emit(abi::move_immediate(&scratch22, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch24, "Integer", "0"));
    builder.emit(abi::compare_registers(&scratch10, &scratch9));
    builder.emit(abi::branch_hi(&write_final));
    builder.emit(abi::subtract_registers(&scratch12, &scratch9, &scratch10));
    builder.emit(abi::label(&write_loop));
    builder.emit(abi::compare_registers(&scratch23, &scratch12));
    builder.emit(abi::branch_hi(&write_final));
    builder.emit(abi::move_immediate(scan_i, "Integer", "0"));
    builder.emit(abi::add_registers(scan_ptr, &scratch14, &scratch23));
    builder.emit(abi::move_register(delim_ptr, &scratch15));
    builder.emit(abi::label(&write_compare));
    builder.emit(abi::compare_registers(scan_i, &scratch10));
    builder.emit(abi::branch_eq(&write_match));
    builder.emit(abi::load_u8(sbyte, scan_ptr, 0));
    builder.emit(abi::load_u8(dbyte, delim_ptr, 0));
    builder.emit(abi::compare_registers(sbyte, dbyte));
    builder.emit(abi::branch_ne(&write_next));
    builder.emit(abi::add_immediate(scan_i, scan_i, 1));
    builder.emit(abi::add_immediate(scan_ptr, scan_ptr, 1));
    builder.emit(abi::add_immediate(delim_ptr, delim_ptr, 1));
    builder.emit(abi::branch(&write_compare));
    builder.emit(abi::label(&write_match));
    builder.emit_string_split_write_entry(
        &scratch20, &scratch21, &scratch22, &scratch24, &scratch23, &scratch14,
    )?;
    builder.emit(abi::add_registers(&scratch23, &scratch23, &scratch10));
    builder.emit(abi::move_register(&scratch24, &scratch23));
    builder.emit(abi::branch(&write_loop));
    builder.emit(abi::label(&write_next));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&write_loop));
    builder.emit(abi::label(&write_final));
    builder.emit_string_split_write_entry(
        &scratch20, &scratch21, &scratch22, &scratch24, &scratch9, &scratch14,
    )?;
    builder.emit(abi::label(&write_done));
    let result = builder.allocate_register()?;
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&invalid_delimiter));
    builder.raise_error("strings.split", "ErrInvalidArgument")?;
    builder.emit(abi::label(&done));

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::parse("List OF String"),
        location: Operand::from(result.render()),
        text: "strings.split".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "split",
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
                    name: "delimiter",
                    desc: "",
                    aliases: &["separator"],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::String),
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_inline(lower),
        }],
    });
}
