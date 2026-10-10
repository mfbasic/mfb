//! bug-772: native `strings::toScalars` / `strings::fromScalars`.
//!
//! Both were MFBASIC source bodies stacked three deep over `encoding::utf32Encode`
//! / `utf32Decode`: one or two arena round-trips per scalar (an inline-`TRAP`
//! `Result` block, a one-byte list literal, a one-scalar `String`) plus the
//! geometric regrows of two intermediate lists. Each now walks the UTF-8 once to
//! size its result, allocates that result exactly once, and fills it in a second
//! walk, the shape `gen_case_map` uses.
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Bytes per `Scalar` payload in a `List OF Scalar` (a kind-2 list).
const SCALAR_BYTES: usize = 4;

fn scalar_list_type() -> ParameterType {
    ParameterType::list_of(ParameterType::named("Scalar"))
}

/// `strings::toScalars(value)`: count the scalars, allocate one exactly-sized
/// `List OF Scalar`, then decode into it.
pub(crate) fn lower_to_scalars(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let [value] = args else {
        return Err("strings.toScalars: no native lowering for these arguments".to_string());
    };
    builder.require_string("strings.toScalars value", value)?;
    let list_type = scalar_list_type();
    let element = ParameterType::named("Scalar");
    if kind2_payload_size(&element) != Some(SCALAR_BYTES) || list_entry_stride(&element) != 0 {
        return Err("native strings.toScalars expects a 4-byte kind-2 Scalar list".to_string());
    }
    let layout = CollectionTypeLayout::from_type(&list_type)
        .ok_or_else(|| "native strings.toScalars cannot resolve List OF Scalar".to_string())?;
    let value_slot = builder.spill_to_slot("strings_to_scalars_value", &value.location);
    let count_slot = builder.allocate_stack_object("strings_to_scalars_count", 8);
    let result_slot = builder.allocate_stack_object("strings_to_scalars_result", 8);

    let data = builder.temporary_vreg();
    let length = builder.temporary_vreg();
    let index = builder.temporary_vreg();
    let count = builder.temporary_vreg();
    let cursor = builder.temporary_vreg();
    let codepoint = builder.temporary_vreg();
    let width = builder.temporary_vreg();

    // Pass 1: count the scalars.
    let count_loop = builder.label("strings_to_scalars_count_loop");
    let count_done = builder.label("strings_to_scalars_count_done");
    builder.emit(abi::load_u64(&data, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&length, &data, 0));
    builder.emit(abi::add_immediate(&data, &data, 8));
    builder.emit(abi::move_immediate(&index, "Integer", "0"));
    builder.emit(abi::move_immediate(&count, "Integer", "0"));
    builder.emit(abi::label(&count_loop));
    builder.emit(abi::compare_registers(&index, &length));
    builder.emit(abi::branch_ge(&count_done));
    builder.emit(abi::add_registers(&cursor, &data, &index));
    builder.emit_utf8_decode_next(&cursor, &codepoint, &width);
    builder.emit(abi::add_registers(&index, &index, &width));
    builder.emit(abi::add_immediate(&count, &count, 1));
    builder.emit(abi::branch(&count_loop));
    builder.emit(abi::label(&count_done));
    builder.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

    // One allocation: header + count * 4 (checked, like every arena size).
    let alloc_ok = builder.label("strings_to_scalars_alloc_ok");
    let size_overflow = builder.label("strings_to_scalars_size_overflow");
    let bytes = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &bytes,
        "Integer",
        &SCALAR_BYTES.to_string(),
    ));
    builder.emit_checked_size_multiply(&bytes, &count, &bytes, &size_overflow);
    builder.emit_checked_size_add_immediate(
        abi::return_register(),
        &bytes,
        COLLECTION_HEADER_SIZE,
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
    let list = builder.temporary_vreg();
    builder.emit(abi::move_register(&list, abi::mfb_return(1)));
    builder.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
    let data_len = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &data_len,
        "Integer",
        &SCALAR_BYTES.to_string(),
    ));
    builder.emit(abi::multiply_registers(&data_len, &count, &data_len));
    builder.emit_write_list_header_from_registers(&layout, &list, &count, &data_len);

    // Pass 2: decode each scalar into its 4-byte slot.
    let out = builder.temporary_vreg();
    builder.emit_collection_data_pointer_for(&out, &list, &element);
    let write_loop = builder.label("strings_to_scalars_write_loop");
    let write_done = builder.label("strings_to_scalars_write_done");
    builder.emit(abi::load_u64(&data, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&length, &data, 0));
    builder.emit(abi::add_immediate(&data, &data, 8));
    builder.emit(abi::move_immediate(&index, "Integer", "0"));
    builder.emit(abi::label(&write_loop));
    builder.emit(abi::compare_registers(&index, &length));
    builder.emit(abi::branch_ge(&write_done));
    builder.emit(abi::add_registers(&cursor, &data, &index));
    builder.emit_utf8_decode_next(&cursor, &codepoint, &width);
    builder.emit(abi::store_u32(&codepoint, &out, 0));
    builder.emit(abi::add_immediate(&out, &out, SCALAR_BYTES));
    builder.emit(abi::add_registers(&index, &index, &width));
    builder.emit(abi::branch(&write_loop));
    builder.emit(abi::label(&write_done));

    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    Ok(ValueResult {
        origin: None,
        type_: list_type,
        location: Operand::from(result),
        text: "strings.toScalars".to_string(),
    })
}

/// `strings::fromScalars(scalars)`: sum the UTF-8 widths, allocate the exact
/// `String` once, then encode into it.
pub(crate) fn lower_from_scalars(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let [scalars] = args else {
        return Err("strings.fromScalars: no native lowering for these arguments".to_string());
    };
    let element = ParameterType::named("Scalar");
    if kind2_payload_size(&element) != Some(SCALAR_BYTES) {
        return Err("native strings.fromScalars expects a 4-byte Scalar payload".to_string());
    }
    let list_slot = builder.spill_to_slot("strings_from_scalars_list", &scalars.location);
    let length_slot = builder.allocate_stack_object("strings_from_scalars_length", 8);
    let result_slot = builder.allocate_stack_object("strings_from_scalars_result", 8);

    let list = builder.temporary_vreg();
    let data = builder.temporary_vreg();
    let count = builder.temporary_vreg();
    let index = builder.temporary_vreg();
    let total = builder.temporary_vreg();
    let codepoint = builder.temporary_vreg();
    let width = builder.temporary_vreg();

    // Pass 1: the UTF-8 byte length.
    let count_loop = builder.label("strings_from_scalars_count_loop");
    let count_done = builder.label("strings_from_scalars_count_done");
    builder.emit(abi::load_u64(&list, abi::stack_pointer(), list_slot));
    builder.emit(abi::load_u64(&count, &list, COLLECTION_OFFSET_COUNT));
    builder.emit_collection_data_pointer_for(&data, &list, &element);
    builder.emit(abi::move_immediate(&index, "Integer", "0"));
    builder.emit(abi::move_immediate(&total, "Integer", "0"));
    builder.emit(abi::label(&count_loop));
    builder.emit(abi::compare_registers(&index, &count));
    builder.emit(abi::branch_ge(&count_done));
    builder.emit(abi::load_u32(&codepoint, &data, 0));
    builder.emit_utf8_encoded_width(&codepoint, &width);
    builder.emit(abi::add_registers(&total, &total, &width));
    builder.emit(abi::add_immediate(&data, &data, SCALAR_BYTES));
    builder.emit(abi::add_immediate(&index, &index, 1));
    builder.emit(abi::branch(&count_loop));
    builder.emit(abi::label(&count_done));
    builder.emit(abi::store_u64(&total, abi::stack_pointer(), length_slot));

    // One allocation: length + 9 (8-byte length header + NUL terminator).
    let alloc_ok = builder.label("strings_from_scalars_alloc_ok");
    let size_overflow = builder.label("strings_from_scalars_size_overflow");
    builder.emit_checked_size_add_immediate(abi::return_register(), &total, 9, &size_overflow);
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
    builder.emit(abi::load_u64(&total, abi::stack_pointer(), length_slot));
    builder.emit(abi::store_u64(&total, abi::mfb_return(1), 0));
    let out = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&out, abi::mfb_return(1), 8));

    // Pass 2: encode.
    let write_loop = builder.label("strings_from_scalars_write_loop");
    let write_done = builder.label("strings_from_scalars_write_done");
    builder.emit(abi::load_u64(&list, abi::stack_pointer(), list_slot));
    builder.emit(abi::load_u64(&count, &list, COLLECTION_OFFSET_COUNT));
    builder.emit_collection_data_pointer_for(&data, &list, &element);
    builder.emit(abi::move_immediate(&index, "Integer", "0"));
    builder.emit(abi::label(&write_loop));
    builder.emit(abi::compare_registers(&index, &count));
    builder.emit(abi::branch_ge(&write_done));
    builder.emit(abi::load_u32(&codepoint, &data, 0));
    builder.emit_utf8_encode_next(&out, &codepoint);
    builder.emit(abi::add_immediate(&data, &data, SCALAR_BYTES));
    builder.emit(abi::add_immediate(&index, &index, 1));
    builder.emit(abi::branch(&write_loop));
    builder.emit(abi::label(&write_done));
    builder.emit(abi::store_u8(abi::ZERO, &out, 0));

    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    // bug-992: the block allocated above is this call's result and nothing else
    // names it, so an unbound result (`len(strings::fromScalars(xs))`) is the
    // statement's to free. Unmarked, it leaked `round16(8 + byteLen)` per call.
    // Every path here allocates: there is no shared-constant fast path.
    builder.mark_fresh_string(Operand::from(result));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::String,
        location: Operand::from(result),
        text: "strings.fromScalars".to_string(),
    })
}
