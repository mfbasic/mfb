//! Helper-tier `List OF <flat record>` marshaller — the list-level sibling of
//! [`super::record::emit_build_inlined_record`].
//!
//! A runtime helper that returns a list of records it builds one at a time
//! (`net::lookup`, `audio::devices`) cannot size the list's data region up front:
//! every element is a spec-canonical record image whose byte size depends on the
//! `String`s inlined into it. So the helper builds each element as its own block
//! through the record marshaller — which leaves that block's byte size in its
//! `size` scratch slot — records `(pointer, size)` for it in a scratch **pair
//! array**, and hands the array to [`emit_build_record_list`].
//!
//! The result is the block the call-site list builder
//! (`CodeBuilder::lower_collection_values`) produces for the same elements: a
//! kind-`LIST` header with value type `OBJECT`, `count == capacity`, one lookup
//! entry per element, and a tight data region in which each element's start is
//! rounded up to 8 (`list_element_padding_alignment`, bug-147.4) and its entry
//! records its exact byte size. A whole-block copy of the list is therefore a
//! correct deep copy, exactly as for a source-built list.
//!
//! Each element block and the pair array are the helper's own scratch — nothing
//! outside the helper ever holds their pointers — and are freed here, with the
//! sizes they were allocated with. An empty list allocates no pair array, so none
//! is freed.
//!
//! Every value lives in a frame slot across the loops: `_mfb_arena_free` clobbers
//! every caller-saved register. The scratch vregs written are a [`MarshalRegs`].

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

use super::record::{emit_align_slot, emit_byte_copy, MarshalRegs};

/// Bytes per element in the pair array: the element block's pointer at `+0` and
/// its byte size at `+8`.
pub(crate) const RECORD_LIST_PAIR_SIZE: usize = 16;

/// Three caller-reserved 8-byte frame slots the list marshaller uses as scratch:
/// the running data length / data cursor, the element index, and the allocated
/// list block. Distinct offsets in the helper's frame; their contents do not
/// outlive the call.
pub(crate) struct RecordListScratch {
    pub(crate) cursor: usize,
    pub(crate) index: usize,
    pub(crate) list: usize,
}

/// Build a `List OF <record>` from the `count` element blocks named by the pair
/// array, free every element block and the pair array, and leave the new list
/// pointer in `result_reg`.
///
/// `pairs_slot` holds the pair array's pointer ([`RECORD_LIST_PAIR_SIZE`] bytes per
/// element, allocated `count * RECORD_LIST_PAIR_SIZE` bytes); `count_slot` holds
/// the element count. A count of zero builds an empty list and frees no array — the
/// caller allocates none for an empty list.
///
/// Branches to `alloc_fail` if the list allocation fails; the element blocks and
/// the pair array are then still allocated, as a helper's other failure exits
/// leave their scratch.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_build_record_list(
    symbol: &str,
    tag: &str,
    pairs_slot: usize,
    count_slot: usize,
    scratch: &RecordListScratch,
    regs: &MarshalRegs,
    result_reg: impl Into<Operand>,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let size_loop = format!("{symbol}_{tag}_rl_size");
    let size_done = format!("{symbol}_{tag}_rl_size_done");
    let fill_loop = format!("{symbol}_{tag}_rl_fill");
    let fill_done = format!("{symbol}_{tag}_rl_fill_done");
    let no_pairs = format!("{symbol}_{tag}_rl_no_pairs");
    let (r0, r1, r2, r3, r4) = (regs.r(0), regs.r(1), regs.r(2), regs.r(3), regs.r(4));

    // Pass 1: data length = sum of the element sizes, each start rounded up to 8.
    instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), scratch.cursor),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), scratch.index),
        abi::label(&size_loop),
        abi::load_u64(r0, abi::stack_pointer(), scratch.index),
        abi::load_u64(r1, abi::stack_pointer(), count_slot),
        abi::compare_registers(r0, r1),
        abi::branch_ge(&size_done),
    ]);
    emit_align_slot(scratch.cursor, regs, instructions);
    emit_pair_field(r2, pairs_slot, scratch.index, 8, regs, instructions);
    instructions.extend([
        abi::load_u64(r0, abi::stack_pointer(), scratch.cursor),
        abi::add_registers(r0, r0, r2),
        abi::store_u64(r0, abi::stack_pointer(), scratch.cursor),
        abi::load_u64(r0, abi::stack_pointer(), scratch.index),
        abi::add_immediate(r0, r0, 1),
        abi::store_u64(r0, abi::stack_pointer(), scratch.index),
        abi::branch(&size_loop),
        abi::label(&size_done),
    ]);

    // Allocate header + count * entry + data length, 8-aligned.
    instructions.extend([
        abi::load_u64(r0, abi::stack_pointer(), count_slot),
        abi::move_immediate(r1, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(r0, r0, r1),
        abi::add_immediate(r0, r0, COLLECTION_HEADER_SIZE),
        abi::load_u64(r1, abi::stack_pointer(), scratch.cursor),
        abi::add_registers(abi::return_register(), r0, r1),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), scratch.list),
        abi::move_register(r1, abi::mfb_return(1)),
        // The header `CodeBuilder::emit_write_collection_header` writes.
        abi::move_immediate(r0, "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8(r0, r1, COLLECTION_OFFSET_KIND),
        abi::move_immediate(r0, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(r0, r1, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(r0, "Byte", &COLLECTION_TYPE_OBJECT.to_string()),
        abi::store_u8(r0, r1, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(r0, "Byte", "1"),
        abi::store_u8(r0, r1, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u8(abi::ZERO, r1, COLLECTION_OFFSET_BUCKETS_READY),
        abi::load_u64(r0, abi::stack_pointer(), count_slot),
        abi::store_u64(r0, r1, COLLECTION_OFFSET_COUNT),
        abi::store_u64(r0, r1, COLLECTION_OFFSET_CAPACITY),
        abi::load_u64(r0, abi::stack_pointer(), scratch.cursor),
        abi::store_u64(r0, r1, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(r0, r1, COLLECTION_OFFSET_DATA_CAPACITY),
    ]);

    // Pass 2: one entry per element, its block copied to its padded start, then
    // the element block freed.
    instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), scratch.cursor),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), scratch.index),
        abi::label(&fill_loop),
        abi::load_u64(r0, abi::stack_pointer(), scratch.index),
        abi::load_u64(r1, abi::stack_pointer(), count_slot),
        abi::compare_registers(r0, r1),
        abi::branch_ge(&fill_done),
    ]);
    emit_align_slot(scratch.cursor, regs, instructions);
    // r2 = entry = list + HEADER + index * ENTRY.
    instructions.extend([
        abi::load_u64(r1, abi::stack_pointer(), scratch.list),
        abi::load_u64(r0, abi::stack_pointer(), scratch.index),
        abi::move_immediate(r2, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(r2, r0, r2),
        abi::add_immediate(r2, r2, COLLECTION_HEADER_SIZE),
        abi::add_registers(r2, r1, r2),
        abi::move_immediate(r3, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(r3, r2, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, r2, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, r2, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::load_u64(r3, abi::stack_pointer(), scratch.cursor),
        abi::store_u64(r3, r2, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
    ]);
    emit_pair_field(r4, pairs_slot, scratch.index, 8, regs, instructions);
    instructions.push(abi::store_u64(r4, r2, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH));
    // r2 = dest = list + HEADER + count * ENTRY + cursor.
    instructions.extend([
        abi::load_u64(r1, abi::stack_pointer(), scratch.list),
        abi::load_u64(r0, abi::stack_pointer(), count_slot),
        abi::move_immediate(r2, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(r2, r0, r2),
        abi::add_immediate(r2, r2, COLLECTION_HEADER_SIZE),
        abi::add_registers(r2, r1, r2),
        abi::load_u64(r3, abi::stack_pointer(), scratch.cursor),
        abi::add_registers(r2, r2, r3),
    ]);
    // r3 = src (the element block), r4 = its size.
    emit_pair_field(r3, pairs_slot, scratch.index, 0, regs, instructions);
    emit_pair_field(r4, pairs_slot, scratch.index, 8, regs, instructions);
    emit_byte_copy(
        r2,
        r3,
        r4,
        &format!("{symbol}_{tag}_rl_copy"),
        regs,
        instructions,
    );
    // cursor += size; free(element, size).
    instructions.extend([
        abi::load_u64(r0, abi::stack_pointer(), scratch.cursor),
        abi::add_registers(r0, r0, r4),
        abi::store_u64(r0, abi::stack_pointer(), scratch.cursor),
    ]);
    emit_pair_field(abi::c_arg(1), pairs_slot, scratch.index, 8, regs, instructions);
    emit_pair_field(abi::c_arg(0), pairs_slot, scratch.index, 0, regs, instructions);
    emit_arena_free(symbol, instructions, relocations);
    instructions.extend([
        abi::load_u64(r0, abi::stack_pointer(), scratch.index),
        abi::add_immediate(r0, r0, 1),
        abi::store_u64(r0, abi::stack_pointer(), scratch.index),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
    ]);

    // Free the pair array (count * PAIR bytes) — none exists for an empty list —
    // and hand back the list.
    instructions.extend([
        abi::load_u64(r0, abi::stack_pointer(), count_slot),
        abi::compare_immediate(r0, "0"),
        abi::branch_eq(&no_pairs),
        abi::move_immediate(r1, "Integer", &RECORD_LIST_PAIR_SIZE.to_string()),
        abi::multiply_registers(abi::c_arg(1), r0, r1),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), pairs_slot),
    ]);
    emit_arena_free(symbol, instructions, relocations);
    instructions.extend([
        abi::label(&no_pairs),
        abi::load_u64(result_reg, abi::stack_pointer(), scratch.list),
    ]);
}

/// Load the pair array's word at `pairs[index] + field` (`0` = pointer, `8` = size)
/// into `dst`. Writes `regs.r(0)`/`regs.r(1)`; `dst` may be an ABI register.
fn emit_pair_field(
    dst: impl Into<Operand>,
    pairs_slot: usize,
    index_slot: usize,
    field: usize,
    regs: &MarshalRegs,
    instructions: &mut Vec<CodeInstruction>,
) {
    let (r0, r1) = (regs.r(0), regs.r(1));
    instructions.extend([
        abi::load_u64(r0, abi::stack_pointer(), index_slot),
        abi::move_immediate(r1, "Integer", &RECORD_LIST_PAIR_SIZE.to_string()),
        abi::multiply_registers(r0, r0, r1),
        abi::load_u64(r1, abi::stack_pointer(), pairs_slot),
        abi::add_registers(r0, r0, r1),
        abi::load_u64(dst, r0, field),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::mir;
    use crate::codegen::engine::util::Vregs;

    fn emit(regs: &MarshalRegs) -> Vec<CodeInstruction> {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let mut instructions = Vec::new();
        let mut relocations = Vec::new();
        emit_build_record_list(
            "rl",
            "t",
            8,
            16,
            &RecordListScratch {
                cursor: 24,
                index: 32,
                list: 40,
            },
            regs,
            abi::mfb_return(1),
            "rl_fail",
            &mut instructions,
            &mut relocations,
        );
        instructions
    }

    fn calls(instructions: &[CodeInstruction], symbol: &str) -> usize {
        instructions
            .iter()
            .filter(|i| i.op == CodeOp::BranchLink && i.get("target").as_deref() == Some(symbol))
            .count()
    }

    /// One allocation (the list) and exactly two free sites: each element block
    /// inside the fill loop, and the pair array after it. A missing free is the
    /// bug-599 leak again; a third would free something the helper does not own.
    #[test]
    fn the_list_is_the_only_allocation_and_both_scratch_kinds_are_freed() {
        let instructions = emit(&MarshalRegs::fixed());
        assert_eq!(calls(&instructions, ARENA_ALLOC_SYMBOL), 1);
        assert_eq!(calls(&instructions, ARENA_FREE_SYMBOL), 2);
    }

    /// The header matches `emit_write_collection_header` for a list of records:
    /// kind `LIST`, value type `OBJECT`, and buckets-ready cleared.
    #[test]
    fn the_header_is_a_list_of_objects() {
        let instructions = emit(&MarshalRegs::fixed());
        let byte_store_after = |offset: usize, value: usize| {
            instructions.windows(2).any(|pair| {
                pair[0].op == CodeOp::MovImm
                    && pair[0].get("value").as_deref() == Some(value.to_string().as_str())
                    && pair[1].get("offset").as_deref() == Some(offset.to_string().as_str())
            })
        };
        assert!(byte_store_after(COLLECTION_OFFSET_KIND, COLLECTION_KIND_LIST));
        assert!(byte_store_after(
            COLLECTION_OFFSET_VALUE_TYPE,
            COLLECTION_TYPE_OBJECT
        ));
        assert!(instructions.iter().any(|i| {
            i.get("src").as_deref() == Some(abi::ZERO)
                && i.get("offset").as_deref()
                    == Some(COLLECTION_OFFSET_BUCKETS_READY.to_string().as_str())
        }));
    }

    /// Both passes round each element's start up to 8 (bug-147.4): the sizing pass
    /// and the fill pass must agree, or an entry's offset runs past the block.
    #[test]
    fn both_passes_pad_each_element_start_to_eight() {
        let instructions = emit(&MarshalRegs::fixed());
        let mask = (!7u64).to_string();
        let aligns = instructions
            .iter()
            .filter(|i| i.op == CodeOp::MovImm && i.get("value").as_deref() == Some(mask.as_str()))
            .count();
        assert_eq!(aligns, 2);
    }

    /// With fresh regs the list builder writes no vreg a helper already holds.
    #[test]
    fn fresh_regs_never_write_a_vreg_the_caller_already_holds() {
        let mut vregs = Vregs::new();
        let held: Vec<String> = (0..16).map(|_| vregs.next()).collect();
        let instructions = emit(&MarshalRegs::fresh(&mut vregs));
        for instruction in &instructions {
            if let Some(dst) = instruction.get("dst") {
                assert!(
                    !held.iter().any(|name| *name == dst),
                    "the list builder wrote `{dst}`, a vreg the caller already held"
                );
            }
        }
    }
}
