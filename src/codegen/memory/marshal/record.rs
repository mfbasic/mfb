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
//! (`record_field_is_inlined` / `type_is_memcpy_copyable`), so a natively-built
//! record and a source-built one have identical layout. A helper building a builtin
//! record classifies it with `TypeModel::builtin_records()` — the same layouts every
//! program's model registers — so the result does not depend on what the program
//! imports (plan-132).
//!
//! The emitter works entirely through stack slots (no value is held in a register
//! across a sub-step). The six scratch vregs it writes are a [`MarshalRegs`]: a
//! helper that keeps any vreg of its own live across the build draws them fresh
//! from its allocator, so the marshaller cannot overwrite one.

use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Four caller-reserved 8-byte frame slots the marshaller uses as scratch:
/// the running block size, the allocated-block pointer, the data-region cursor,
/// and the per-field inlined sub-block size. Distinct offsets in the helper's
/// frame.
///
/// After a build, `size` holds the new record's byte size — the size it was
/// allocated with, and the size `CodeBuilder::emit_record_block_size_to_slot`
/// computes for it — so a caller that embeds the record in a list, inlines it into
/// another record, or frees it reads its size from there.
pub(crate) struct RecordBuildScratch {
    pub(crate) size: usize,
    pub(crate) result: usize,
    pub(crate) cursor: usize,
    pub(crate) block_size: usize,
}

/// The six scratch vregs a helper-tier marshaller writes.
///
/// A helper's vregs are numbered by its own `Vregs` allocator from `%v0`, so a
/// FIXED set of names is safe only for a helper that keeps nothing live across the
/// build in that range: a `HelperScratch` pointer declared later in the body, or a
/// register loaded before the build and read after it, would be silently
/// overwritten — and a free of that pointer at `done` would release garbage.
/// [`MarshalRegs::fresh`] makes that collision impossible by construction.
pub(crate) struct MarshalRegs {
    regs: [String; 6],
}

impl MarshalRegs {
    /// The `%v9`..`%v14` this marshaller has always written. Kept for its existing
    /// callers (`crypto::generate`'s record builds), which spill every live value to
    /// frame slots first; a new caller uses [`MarshalRegs::fresh`].
    pub(crate) fn fixed() -> Self {
        Self {
            regs: ["%v9", "%v10", "%v11", "%v12", "%v13", "%v14"].map(str::to_string),
        }
    }

    /// Six vregs no other part of the helper has been given.
    pub(crate) fn fresh(vregs: &mut Vregs) -> Self {
        Self {
            regs: std::array::from_fn(|_| vregs.next()),
        }
    }

    pub(crate) fn r(&self, index: usize) -> &str {
        &self.regs[index]
    }
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
/// Branches to `alloc_fail` on allocation failure. Writes the fixed
/// [`MarshalRegs`] plus the four `scratch` slots. Mirrors
/// `CodeBuilder::emit_build_inlined_record`. A record with an inlined nested
/// **record** field needs its size from the caller: see
/// [`emit_build_inlined_record_sized`].
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
    emit_build_inlined_record_sized(
        symbol,
        tag,
        record_type,
        type_model,
        field_slots,
        &vec![None; field_slots.len()],
        scratch,
        &MarshalRegs::fixed(),
        result_reg,
        alloc_fail,
        instructions,
        relocations,
    )
}

/// [`emit_build_inlined_record`], where `known_sizes[i]` may name a frame slot that
/// already holds the byte size of inlined field `i`'s source block, and `regs` names
/// the scratch vregs the build writes.
///
/// A known size is how a helper builds a record with an inlined nested **record**
/// field (plan-132: `udp::Datagram.from` and `net::PingResult.address` are a flat
/// `net::Address`). Sizing a nested record from its pointer needs a per-depth slot
/// walk the fixed helper frame cannot host, but the helper built that nested record
/// itself, and the marshaller left its size in [`RecordBuildScratch::size`]; the
/// caller parks it in a slot and names it here.
///
/// A known size is accepted only for a field the layout inlines. Naming one for any
/// other field is an error: the caller believes a field is inlined that the layout
/// stores in its slot, and building on that belief would write a pointer where an
/// offset belongs.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_build_inlined_record_sized(
    symbol: &str,
    tag: &str,
    record_type: &ParameterType,
    type_model: &TypeModel,
    field_slots: &[usize],
    known_sizes: &[Option<usize>],
    scratch: &RecordBuildScratch,
    regs: &MarshalRegs,
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
    if fields.len() != field_slots.len() || fields.len() != known_sizes.len() {
        return Err(format!(
            "native record '{record_type}' construction expected {} fields, got {} slots \
             and {} known sizes",
            fields.len(),
            field_slots.len(),
            known_sizes.len()
        ));
    }
    for (index, (name, field_type)) in fields.iter().enumerate() {
        if known_sizes[index].is_some()
            && !record_field_is_inlined(type_model, field_type)
        {
            return Err(format!(
                "native record '{record_type}' field '{name}' ({field_type}) is not inlined, \
                 so it takes no known size"
            ));
        }
    }
    let fixed = 8 * fields.len();
    let (r0, r1, r2, r3, r4) = (regs.r(0), regs.r(1), regs.r(2), regs.r(3), regs.r(4));

    // Pass 1: total size = fixed slots + each inlined sub-block (8-aligned).
    instructions.extend([
        abi::move_immediate(r0, "Integer", &fixed.to_string()),
        abi::store_u64(r0, abi::stack_pointer(), scratch.size),
    ]);
    for (index, (_, field_type)) in fields.iter().enumerate() {
        if !record_field_is_inlined(type_model, field_type) {
            continue;
        }
        emit_align_slot(scratch.size, regs, instructions);
        emit_field_block_size(
            type_model,
            field_type,
            field_slots[index],
            known_sizes[index],
            scratch.block_size,
            record_type,
            regs,
            instructions,
        )?;
        instructions.extend([
            abi::load_u64(r0, abi::stack_pointer(), scratch.size),
            abi::load_u64(r1, abi::stack_pointer(), scratch.block_size),
            abi::add_registers(r0, r0, r1),
            abi::store_u64(r0, abi::stack_pointer(), scratch.size),
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
        abi::move_immediate(r0, "Integer", &fixed.to_string()),
        abi::store_u64(r0, abi::stack_pointer(), scratch.cursor),
    ]);
    for (index, (_, field_type)) in fields.iter().enumerate() {
        if record_field_is_inlined(type_model, field_type) {
            emit_align_slot(scratch.cursor, regs, instructions);
            // Slot stores the block-relative offset of the inlined sub-block.
            instructions.extend([
                abi::load_u64(r1, abi::stack_pointer(), scratch.result),
                abi::load_u64(r0, abi::stack_pointer(), scratch.cursor),
                abi::store_u64(r0, r1, 8 * index),
            ]);
            emit_field_block_size(
                type_model,
                field_type,
                field_slots[index],
                known_sizes[index],
                scratch.block_size,
                record_type,
                regs,
                instructions,
            )?;
            // dest = recordBase + cursor; copy `block_size` bytes from the source.
            instructions.extend([
                abi::load_u64(r1, abi::stack_pointer(), scratch.result),
                abi::load_u64(r0, abi::stack_pointer(), scratch.cursor),
                abi::add_registers(r2, r1, r0), // r2 = dest
                abi::load_u64(r3, abi::stack_pointer(), field_slots[index]), // r3 = src
                abi::load_u64(r4, abi::stack_pointer(), scratch.block_size), // r4 = len
            ]);
            emit_byte_copy(
                r2,
                r3,
                r4,
                &format!("{symbol}_{tag}_f{index}"),
                regs,
                instructions,
            );
            // Advance the cursor past the copied block.
            instructions.extend([
                abi::load_u64(r4, abi::stack_pointer(), scratch.block_size),
                abi::load_u64(r0, abi::stack_pointer(), scratch.cursor),
                abi::add_registers(r0, r0, r4),
                abi::store_u64(r0, abi::stack_pointer(), scratch.cursor),
            ]);
        } else {
            instructions.extend([
                abi::load_u64(r0, abi::stack_pointer(), field_slots[index]),
                abi::load_u64(r1, abi::stack_pointer(), scratch.result),
                abi::store_u64(r0, r1, 8 * index),
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
/// Writes `regs.r(0)`/`regs.r(1)`.
pub(crate) fn emit_align_slot(
    slot: usize,
    regs: &MarshalRegs,
    instructions: &mut Vec<CodeInstruction>,
) {
    let mask = !7u64; // clear the low 3 bits after adding 7.
    let (r0, r1) = (regs.r(0), regs.r(1));
    instructions.extend([
        abi::load_u64(r0, abi::stack_pointer(), slot),
        abi::add_immediate(r0, r0, 7),
        abi::move_immediate(r1, "Integer", &mask.to_string()),
        abi::and_registers(r0, r0, r1),
        abi::store_u64(r0, abi::stack_pointer(), slot),
    ]);
}

/// The byte size of inlined field `field_type`'s source block into `out_slot`:
/// copied from `known_size` when the caller holds it, walked from the source
/// pointer in `ptr_slot` otherwise. Writes `regs.r(0)`..`regs.r(2)`.
#[allow(clippy::too_many_arguments)]
fn emit_field_block_size(
    type_model: &TypeModel,
    field_type: &ParameterType,
    ptr_slot: usize,
    known_size: Option<usize>,
    out_slot: usize,
    record_type: &ParameterType,
    regs: &MarshalRegs,
    instructions: &mut Vec<CodeInstruction>,
) -> Result<(), String> {
    match known_size {
        Some(size_slot) => {
            instructions.extend([
                abi::load_u64(regs.r(0), abi::stack_pointer(), size_slot),
                abi::store_u64(regs.r(0), abi::stack_pointer(), out_slot),
            ]);
            Ok(())
        }
        None => emit_inlined_block_size(
            type_model,
            field_type,
            ptr_slot,
            out_slot,
            record_type,
            regs,
            instructions,
        ),
    }
}

/// Emit the total byte size of the inlined sub-block of `field_type` whose source
/// pointer is in `ptr_slot`, into `out_slot`. Mirrors
/// `CodeBuilder::emit_inlined_block_size_from_ptr_slot`. A nested inlined **record**
/// field is rejected: sizing it needs a per-depth slot walk the fixed helper frame
/// cannot host, so its size must come from the caller
/// ([`emit_build_inlined_record_sized`]) or the record must build at the call site.
/// `record_type` names the enclosing record for that diagnostic. Writes
/// `regs.r(0)`..`regs.r(2)`.
fn emit_inlined_block_size(
    type_model: &TypeModel,
    field_type: &ParameterType,
    ptr_slot: usize,
    out_slot: usize,
    record_type: &ParameterType,
    regs: &MarshalRegs,
    instructions: &mut Vec<CodeInstruction>,
) -> Result<(), String> {
    let (r0, r1, r2) = (regs.r(0), regs.r(1), regs.r(2));
    if *field_type == ParameterType::String {
        // byteLength(+0) + 8 (length word) + 1 (trailing NUL).
        instructions.extend([
            abi::load_u64(r0, abi::stack_pointer(), ptr_slot),
            abi::load_u64(r1, r0, 0),
            abi::add_immediate(r1, r1, 9),
            abi::store_u64(r1, abi::stack_pointer(), out_slot),
        ]);
        Ok(())
    } else if typed_is_collection_type(field_type) {
        instructions.push(abi::load_u64(r0, abi::stack_pointer(), ptr_slot));
        emit_collection_flat_size(field_type, r0, r1, r2, instructions);
        instructions.push(abi::store_u64(r1, abi::stack_pointer(), out_slot));
        Ok(())
    } else if union_is_data(type_model, field_type)
        || matches!(field_type, ParameterType::ResultOf(_))
    {
        // A data union and a flat `Result` are self-describing: `size` word @+8.
        instructions.extend([
            abi::load_u64(r0, abi::stack_pointer(), ptr_slot),
            abi::load_u64(r1, r0, 8),
            abi::store_u64(r1, abi::stack_pointer(), out_slot),
        ]);
        Ok(())
    } else if type_model.record_fields.contains_key(field_type) {
        Err(format!(
            "helper-tier record marshaller cannot inline a nested record field \
             '{field_type}' of '{record_type}' without its size; pass it through \
             emit_build_inlined_record_sized or build at the call site"
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
/// writes `regs.r(0)`, `regs.r(1)` and `regs.r(5)`. `tag` disambiguates the loop
/// labels.
pub(crate) fn emit_byte_copy(
    dst_reg: &str,
    src_reg: &str,
    len_reg: &str,
    tag: &str,
    regs: &MarshalRegs,
    instructions: &mut Vec<CodeInstruction>,
) {
    let loop_l = format!("{tag}_rcp");
    let done_l = format!("{tag}_rcpd");
    let (index, byte, address) = (regs.r(0), regs.r(1), regs.r(5));
    instructions.extend([
        abi::move_immediate(index, "Integer", "0"),
        abi::label(&loop_l),
        abi::compare_registers(index, len_reg),
        abi::branch_ge(&done_l),
        abi::add_registers(address, src_reg, index),
        abi::load_u8(byte, address, 0),
        abi::add_registers(address, dst_reg, index),
        abi::store_u8(byte, address, 0),
        abi::add_immediate(index, index, 1),
        abi::branch(&loop_l),
        abi::label(&done_l),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::mir;

    const SIZE_SLOT: usize = 96;

    /// `Outer { a AS Integer, inner AS Inner, t AS String }` over a flat
    /// `Inner { s AS String, n AS Integer }`, so `inner` is an inlined nested record.
    fn model() -> TypeModel {
        let mut model = TypeModel::empty();
        model.record_fields.insert(
            ParameterType::declared("Inner"),
            vec![
                ("s".to_string(), ParameterType::String),
                ("n".to_string(), ParameterType::Integer),
            ],
        );
        model.record_fields.insert(
            ParameterType::declared("Outer"),
            vec![
                ("a".to_string(), ParameterType::Integer),
                ("inner".to_string(), ParameterType::declared("Inner")),
                ("t".to_string(), ParameterType::String),
            ],
        );
        model
    }

    fn build(
        known_sizes: &[Option<usize>],
        regs: &MarshalRegs,
    ) -> (Result<(), String>, Vec<CodeInstruction>) {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let model = model();
        let mut instructions = Vec::new();
        let mut relocations = Vec::new();
        let result = emit_build_inlined_record_sized(
            "rec",
            "t",
            &ParameterType::declared("Outer"),
            &model,
            &[8, 16, 24],
            known_sizes,
            &RecordBuildScratch {
                size: 32,
                result: 40,
                cursor: 48,
                block_size: 56,
            },
            regs,
            abi::mfb_return(1),
            "rec_fail",
            &mut instructions,
            &mut relocations,
        );
        (result, instructions)
    }

    /// Without a size, a nested inlined record is refused, as it always was.
    #[test]
    fn a_nested_record_field_without_its_size_is_refused() {
        let (result, _) = build(&[None, None, None], &MarshalRegs::fixed());
        let error = result.expect_err("a nested record field needs its size");
        assert!(error.contains("cannot inline a nested record field"), "{error}");
    }

    /// With the caller's size, the record builds, and that size slot is what both
    /// passes read for the nested field — the sizing pass and the copy pass must
    /// agree or the copy runs past the allocation.
    #[test]
    fn a_nested_record_field_builds_from_its_known_size_in_both_passes() {
        let (result, instructions) = build(&[None, Some(SIZE_SLOT), None], &MarshalRegs::fixed());
        result.expect("the nested field's size is known");
        let size_reads = instructions
            .iter()
            .filter(|i| {
                i.op == CodeOp::LdrU64
                    && i.get("offset").as_deref() == Some(SIZE_SLOT.to_string().as_str())
            })
            .count();
        assert_eq!(size_reads, 2);
    }

    /// A size for a field the layout keeps in its slot is a caller who has the
    /// layout wrong; refuse rather than build on it.
    #[test]
    fn a_known_size_on_a_slot_field_is_refused() {
        let (result, _) = build(&[Some(SIZE_SLOT), None, None], &MarshalRegs::fixed());
        let error = result.expect_err("an Integer field is not inlined");
        assert!(error.contains("is not inlined"), "{error}");
    }

    /// The fixed set is exactly what `crypto::generate`'s record builds have always
    /// been emitted with; changing it moves their byte-identity goldens.
    #[test]
    fn the_fixed_regs_are_the_legacy_names() {
        let regs = MarshalRegs::fixed();
        let names: Vec<&str> = (0..6).map(|i| regs.r(i)).collect();
        assert_eq!(names, ["%v9", "%v10", "%v11", "%v12", "%v13", "%v14"]);
    }

    /// Fresh regs never write a vreg the helper already holds — the collision a
    /// fixed set invites in a helper that has drawn a dozen vregs before the build.
    #[test]
    fn fresh_regs_never_write_a_vreg_the_caller_already_holds() {
        let mut vregs = Vregs::new();
        let held: Vec<String> = (0..16).map(|_| vregs.next()).collect();
        let regs = MarshalRegs::fresh(&mut vregs);
        let (result, instructions) = build(&[None, Some(SIZE_SLOT), None], &regs);
        result.expect("builds");
        for instruction in &instructions {
            if let Some(dst) = instruction.get("dst") {
                assert!(
                    !held.iter().any(|name| *name == dst),
                    "the marshaller wrote `{dst}`, a vreg the caller already held"
                );
            }
        }
    }
}
