//! Helper-tier record marshaller — the free-function sibling of
//! [`super::byte_list::emit_build_byte_list`].
//!
//! A `Body::abi_function` runtime-helper emitter (`crypto`, `net`, …) runs below the
//! `CodeBuilder`, so it cannot call the call-site record builder
//! (`CodeBuilder::emit_build_inlined_record`). This module gives it a
//! byte-level constructor that produces the **spec-canonical** record image
//! (`spec/memory/03_heap-values.md` §Record): `8 * fieldCount` slots followed by
//! a trailing 8-aligned data region into which every inlined `String`/flat-
//! composite field's block is copied, its slot holding the block-relative offset.
//! A whole-block `memcpy` is therefore a correct deep copy of the result, exactly
//! as the call-site builder guarantees.
//!
//! Field classification goes through the shared `&TypeModel` predicates
//! (`record_field_is_inlined` / `type_is_flat` / `is_pointer_string_record`), so a
//! natively-built record and a source-built one have identical layout. The
//! `TypeModel` reaches the emitter through the shared `TypeModel` lookup.
//!
//! The emitter works entirely through stack slots (no value is held in a register
//! across a sub-step), matching the `%v9`..`%v15` discipline of the other helper-
//! tier marshallers: callers spill everything live to frame slots first, so the
//! scratch vregs used here are free.

use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Four caller-reserved 8-byte frame slots the marshaller uses as scratch:
/// the running block size, the allocated-block pointer, the data-region cursor,
/// and the per-field inlined sub-block size. Distinct offsets in the helper's
/// frame; their contents do not outlive the call.
pub(crate) struct RecordBuildScratch {
    pub(crate) size: usize,
    pub(crate) result: usize,
    pub(crate) cursor: usize,
    pub(crate) block_size: usize,
}

/// Build a record of `record_type` from `field_slots` (one frame slot per field,
/// in declaration order) and leave the new record pointer in `result_reg`.
///
/// A slot for an **inlined** field (an inlined `String` or a flat composite —
/// `record_field_is_inlined`) holds a **pointer to the source sub-block**, whose
/// bytes are copied into the record's data region and whose slot then stores the
/// block-relative offset. Every other field slot holds the scalar value or
/// pointer, written inline at `8 * index`.
///
/// Branches to `alloc_fail` on allocation failure. Uses `%v9`..`%v15` plus the
/// four `scratch` slots. Mirrors `CodeBuilder::emit_build_inlined_record`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_build_inlined_record(
    symbol: &str,
    tag: &str,
    record_type: &ParameterType,
    type_model: &TypeModel,
    field_slots: &[usize],
    scratch: &RecordBuildScratch,
    result_reg: impl Into<Operand>,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let fields = type_model
        .record_fields
        .get(record_type)
        .cloned()
        .ok_or_else(|| format!("native record type '{record_type}' does not resolve"))?;
    if fields.len() != field_slots.len() {
        return Err(format!(
            "native record '{record_type}' construction expected {} fields, got {}",
            fields.len(),
            field_slots.len()
        ));
    }
    let fixed = 8 * fields.len();

    // Pass 1: total size = fixed slots + each inlined sub-block (8-aligned).
    instructions.extend([
        abi::move_immediate("%v9", "Integer", &fixed.to_string()),
        abi::store_u64("%v9", abi::stack_pointer(), scratch.size),
    ]);
    for (index, (_, field_type)) in fields.iter().enumerate() {
        if !record_field_is_inlined(type_model, record_type, field_type) {
            continue;
        }
        emit_align_slot(scratch.size, instructions);
        emit_inlined_block_size(
            type_model,
            field_type,
            field_slots[index],
            scratch.block_size,
            record_type,
            instructions,
        )?;
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), scratch.size),
            abi::load_u64("%v10", abi::stack_pointer(), scratch.block_size),
            abi::add_registers("%v9", "%v9", "%v10"),
            abi::store_u64("%v9", abi::stack_pointer(), scratch.size),
        ]);
    }

    // Allocate the record block: x0 = size, x1 = 8-byte alignment.
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), scratch.size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        scratch.result,
    ));

    // Pass 2: write slots; inline each flat sub-block into the data region.
    instructions.extend([
        abi::move_immediate("%v9", "Integer", &fixed.to_string()),
        abi::store_u64("%v9", abi::stack_pointer(), scratch.cursor),
    ]);
    for (index, (_, field_type)) in fields.iter().enumerate() {
        if record_field_is_inlined(type_model, record_type, field_type) {
            emit_align_slot(scratch.cursor, instructions);
            // Slot stores the block-relative offset of the inlined sub-block.
            instructions.extend([
                abi::load_u64("%v10", abi::stack_pointer(), scratch.result),
                abi::load_u64("%v9", abi::stack_pointer(), scratch.cursor),
                abi::store_u64("%v9", "%v10", 8 * index),
            ]);
            emit_inlined_block_size(
                type_model,
                field_type,
                field_slots[index],
                scratch.block_size,
                record_type,
                instructions,
            )?;
            // dest = recordBase + cursor; copy `block_size` bytes from the source.
            instructions.extend([
                abi::load_u64("%v10", abi::stack_pointer(), scratch.result),
                abi::load_u64("%v9", abi::stack_pointer(), scratch.cursor),
                abi::add_registers("%v11", "%v10", "%v9"), // %v11 = dest
                abi::load_u64("%v12", abi::stack_pointer(), field_slots[index]), // %v12 = src
                abi::load_u64("%v13", abi::stack_pointer(), scratch.block_size), // %v13 = len
            ]);
            emit_byte_copy(
                "%v11",
                "%v12",
                "%v13",
                &format!("{symbol}_{tag}_f{index}"),
                instructions,
            );
            // Advance the cursor past the copied block.
            instructions.extend([
                abi::load_u64("%v13", abi::stack_pointer(), scratch.block_size),
                abi::load_u64("%v9", abi::stack_pointer(), scratch.cursor),
                abi::add_registers("%v9", "%v9", "%v13"),
                abi::store_u64("%v9", abi::stack_pointer(), scratch.cursor),
            ]);
        } else {
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), field_slots[index]),
                abi::load_u64("%v10", abi::stack_pointer(), scratch.result),
                abi::store_u64("%v9", "%v10", 8 * index),
            ]);
        }
    }
    instructions.push(abi::load_u64(
        result_reg,
        abi::stack_pointer(),
        scratch.result,
    ));
    Ok(())
}

/// Round the unsigned offset in `slot` up to an 8-byte boundary in place.
/// Uses `%v9`/`%v10`.
fn emit_align_slot(slot: usize, instructions: &mut Vec<CodeInstruction>) {
    let mask = !7u64; // clear the low 3 bits after adding 7.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), slot),
        abi::add_immediate("%v9", "%v9", 7),
        abi::move_immediate("%v10", "Integer", &mask.to_string()),
        abi::and_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), slot),
    ]);
}

/// Emit the total byte size of the inlined sub-block of `field_type` whose source
/// pointer is in `ptr_slot`, into `out_slot`. Mirrors
/// `CodeBuilder::emit_inlined_block_size_from_ptr_slot`. A nested inlined **record**
/// field is rejected: sizing it needs a per-depth slot walk the fixed helper frame
/// cannot host, so a record with an inlined record field must build at the call
/// site (`emit_build_inlined_record`) rather than natively. `record_type` names
/// the enclosing record for that diagnostic. Uses `%v9`..`%v11`.
fn emit_inlined_block_size(
    type_model: &TypeModel,
    field_type: &ParameterType,
    ptr_slot: usize,
    out_slot: usize,
    record_type: &ParameterType,
    instructions: &mut Vec<CodeInstruction>,
) -> Result<(), String> {
    if *field_type == ParameterType::String {
        // byteLength(+0) + 8 (length word) + 1 (trailing NUL).
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), ptr_slot),
            abi::load_u64("%v10", "%v9", 0),
            abi::add_immediate("%v10", "%v10", 9),
            abi::store_u64("%v10", abi::stack_pointer(), out_slot),
        ]);
        Ok(())
    } else if typed_is_collection_type(field_type) {
        instructions.push(abi::load_u64("%v9", abi::stack_pointer(), ptr_slot));
        emit_collection_flat_size(field_type, "%v9", "%v10", "%v11", instructions);
        instructions.push(abi::store_u64("%v10", abi::stack_pointer(), out_slot));
        Ok(())
    } else if union_is_data(type_model, field_type)
        || matches!(field_type, ParameterType::ResultOf(_))
    {
        // A data union and a flat `Result` are self-describing: `size` word @+8.
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), ptr_slot),
            abi::load_u64("%v10", "%v9", 8),
            abi::store_u64("%v10", abi::stack_pointer(), out_slot),
        ]);
        Ok(())
    } else if type_model.record_fields.contains_key(field_type) {
        Err(format!(
            "helper-tier record marshaller cannot inline a nested record field \
             '{field_type}' of '{record_type}'; build it at the call site instead"
        ))
    } else {
        Err(format!(
            "native inlined field size not available for type '{field_type}'"
        ))
    }
}

/// Byte size of a flat collection block at `ptr_reg` into `out_reg`
/// (`scratch_reg` clobbered): `header + capacity * entryStride + dataCapacity`
/// (+ a `Map`/`Set`'s `capacity << 4` bucket region). The stride MUST match what
/// the allocator reserved — a wrong stride here frees/copies past the block and
/// corrupts the arena (bug-02). Mirrors the collection arm of
/// `CodeBuilder::emit_flat_block_size`.
fn emit_collection_flat_size(
    collection_type: &ParameterType,
    ptr_reg: &str,
    out_reg: &str,
    scratch_reg: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let element = typed_list_element_type(collection_type)
        .cloned()
        .unwrap_or_else(|| ParameterType::named(""));
    let stride = list_entry_stride(&element);
    instructions.extend([
        abi::load_u64(out_reg, ptr_reg, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(scratch_reg, "Integer", &stride.to_string()),
        abi::multiply_registers(out_reg, out_reg, scratch_reg),
        abi::add_immediate(out_reg, out_reg, COLLECTION_HEADER_SIZE),
        abi::load_u64(scratch_reg, ptr_reg, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers(out_reg, out_reg, scratch_reg),
    ]);
    if collection_has_buckets(&collection_type) {
        instructions.extend([
            abi::load_u64(scratch_reg, ptr_reg, COLLECTION_OFFSET_CAPACITY),
            abi::shift_left_immediate(scratch_reg, scratch_reg, 4),
            abi::add_registers(out_reg, out_reg, scratch_reg),
        ]);
    }
}

/// Copy `len_reg` bytes from `src_reg` to `dst_reg` (a plain byte loop; the
/// inlined key/DER blocks are small). `src`/`dst`/`len` registers are preserved;
/// uses `%v9`/`%v10`/`%v14`. `tag` disambiguates the loop labels.
fn emit_byte_copy(
    dst_reg: &str,
    src_reg: &str,
    len_reg: &str,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let loop_l = format!("{tag}_rcp");
    let done_l = format!("{tag}_rcpd");
    instructions.extend([
        abi::move_immediate("%v9", "Integer", "0"),
        abi::label(&loop_l),
        abi::compare_registers("%v9", len_reg),
        abi::branch_ge(&done_l),
        abi::add_registers("%v14", src_reg, "%v9"),
        abi::load_u8("%v10", "%v14", 0),
        abi::add_registers("%v14", dst_reg, "%v9"),
        abi::store_u8("%v10", "%v14", 0),
        abi::add_immediate("%v9", "%v9", 1),
        abi::branch(&loop_l),
        abi::label(&done_l),
    ]);
}
