//! The native access foundation every `big` member lowers through (plan-127-A §4.3).
//!
//! A `big::Int` argument is a pointer to a two-slot record (plan-127-A §4.2, pinned in
//! Phase 1): slot 0 (`+0`) holds the block-relative offset of the inlined
//! `List OF Byte` magnitude, slot 1 (`+8`) holds `negative` inline. The magnitude is a
//! kind-2 fixed-width list, so its `count` is at `+8` and its bytes start at `+40` for
//! every capacity. A `big::Int` therefore occupies `16 + 40 + dataCapacity` bytes — the
//! size the record marshaller computes and scope exit releases.
//!
//! Three emitters, and every member goes through them:
//!
//! - [`emit_load_int`] reads an argument into `(data, count, negative)` frame slots and
//!   **trims** it: trailing zero bytes are not counted and a zero magnitude reads as
//!   non-negative. The decoder is total by design — a hand-built record that breaks the
//!   canonical form still reads as the number it spells.
//! - [`emit_alloc_magnitude`] makes the result record up front at its largest possible
//!   size, with a zeroed magnitude region the member writes into directly. One block
//!   per result; no scratch buffer and no second copy (Open Decision 1).
//! - [`emit_build_int`] **normalizes** that result — drops trailing zero bytes, forces a
//!   zero value non-negative — and publishes it in the result register. It is the one
//!   place canonical form is established, so it is a property of the package rather
//!   than of each lowering.
//!
//! Everything is kept in frame slots between steps. The only call these emitters make
//! is the arena allocation, and every value a later step needs is reloaded from its
//! slot, so no register is assumed to survive it.

use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

/// `big::Int` slot 0: the block-relative offset of the inlined magnitude.
pub(crate) const INT_OFFSET_MAGNITUDE: usize = 0;
/// `big::Int` slot 1: the `negative` flag, inline.
pub(crate) const INT_OFFSET_NEGATIVE: usize = 8;
/// Where the record marshaller places the first inlined field: `8 * fieldCount`.
pub(crate) const INT_MAGNITUDE_BLOCK: usize = 16;
/// The first magnitude byte, relative to the record: the block plus the list header.
pub(crate) const INT_DATA_OFFSET: usize = INT_MAGNITUDE_BLOCK + COLLECTION_HEADER_SIZE;

/// The frame slots a loaded `big::Int` lives in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IntSlots {
    /// Address of the first (least significant) magnitude byte.
    pub(crate) data: usize,
    /// Significant byte count — trailing zero bytes already excluded.
    pub(crate) count: usize,
    /// `1` when negative, `0` otherwise (always `0` when `count` is `0`).
    pub(crate) negative: usize,
}

/// The frame slots a result under construction lives in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResultSlots {
    /// The result record.
    pub(crate) record: usize,
    /// Address of its first magnitude byte.
    pub(crate) data: usize,
}

/// Spill every incoming argument register to its own frame slot, before any other
/// instruction. An argument register is live only until the body's first scratch use
/// or call; after this every argument is read from its slot.
pub(crate) fn emit_spill_args(builder: &mut CodeBuilder, args: &[ValueResult]) -> Vec<usize> {
    args.iter()
        .enumerate()
        .map(|(index, arg)| {
            let slot = builder.allocate_stack_object(&format!("big_arg{index}"), 8);
            builder.emit(abi::store_u64(&arg.location, abi::stack_pointer(), slot));
            slot
        })
        .collect()
}

/// Resolve the `big::Int` whose record address is in `arg_slot` to its data address,
/// significant byte count and sign, each in a fresh frame slot. `tag` keeps the trim
/// loop's labels distinct when a member loads several operands.
pub(crate) fn emit_load_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    arg_slot: usize,
    tag: &str,
) -> IntSlots {
    let symbol = builder.current_symbol.clone();
    let slots = IntSlots {
        data: builder.allocate_stack_object(&format!("big_{tag}_data"), 8),
        count: builder.allocate_stack_object(&format!("big_{tag}_count"), 8),
        negative: builder.allocate_stack_object(&format!("big_{tag}_negative"), 8),
    };
    let record = vregs.next();
    let block = vregs.next();
    let data = vregs.next();
    let count = vregs.next();
    let negative = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&record, abi::stack_pointer(), arg_slot),
        abi::load_u64(&block, &record, INT_OFFSET_MAGNITUDE),
        abi::add_registers(&block, &record, &block),
        abi::load_u64(&count, &block, COLLECTION_OFFSET_COUNT),
        abi::add_immediate(&data, &block, COLLECTION_HEADER_SIZE),
        abi::load_u64(&negative, &record, INT_OFFSET_NEGATIVE),
        abi::store_u64(&data, abi::stack_pointer(), slots.data),
    ]);
    emit_trim(builder, vregs, &data, &count, &negative, &format!("{symbol}_{tag}_load"));
    builder.instructions.extend([
        abi::store_u64(&count, abi::stack_pointer(), slots.count),
        abi::store_u64(&negative, abi::stack_pointer(), slots.negative),
    ]);
    slots
}

/// Make a result `big::Int` able to hold `capacity_slot` magnitude bytes: the record
/// block, its slot-0 offset, a kind-2 list header with `count` zero, and a zeroed
/// magnitude region. Branches to `alloc_fail` when the block cannot be made.
pub(crate) fn emit_alloc_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    capacity_slot: usize,
    tag: &str,
    alloc_fail: &str,
) -> ResultSlots {
    let symbol = builder.current_symbol.clone();
    let slots = ResultSlots {
        record: builder.allocate_stack_object(&format!("big_{tag}_record"), 8),
        data: builder.allocate_stack_object(&format!("big_{tag}_rdata"), 8),
    };
    let size = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&size, abi::stack_pointer(), capacity_slot),
        abi::add_immediate(&size, &size, INT_DATA_OFFSET),
        abi::move_register(abi::return_register(), &size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(
        &symbol,
        &mut builder.instructions,
        &mut builder.relocations,
        alloc_fail,
    );
    let record = vregs.next();
    let block = vregs.next();
    let scratch = vregs.next();
    let capacity = vregs.next();
    let cursor = vregs.next();
    let end = vregs.next();
    let zero_loop = format!("{symbol}_{tag}_zero");
    let zero_done = format!("{symbol}_{tag}_zero_done");
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), slots.record),
        abi::load_u64(&record, abi::stack_pointer(), slots.record),
        abi::load_u64(&capacity, abi::stack_pointer(), capacity_slot),
        // Slot 0: the magnitude block sits right after the two slots.
        abi::move_immediate(&scratch, "Integer", &INT_MAGNITUDE_BLOCK.to_string()),
        abi::store_u64(&scratch, &record, INT_OFFSET_MAGNITUDE),
        abi::store_u64(abi::ZERO, &record, INT_OFFSET_NEGATIVE),
        abi::add_immediate(&block, &record, INT_MAGNITUDE_BLOCK),
        // The kind-2 `List OF Byte` header, as `emit_build_byte_list` writes it. The
        // capacity words are the block's real size, so a copy or a release sizes the
        // block exactly; `count`/`dataLength` start at zero and are set by
        // `emit_build_int`.
        abi::move_immediate(&scratch, "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(&scratch, &block, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&scratch, &block, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(&scratch, &block, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &block, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u8(abi::ZERO, &block, COLLECTION_OFFSET_BUCKETS_READY),
        abi::store_u64(abi::ZERO, &block, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&capacity, &block, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(abi::ZERO, &block, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&capacity, &block, COLLECTION_OFFSET_DATA_CAPACITY),
        // The magnitude region, zeroed: a member writes only the bytes it computes,
        // and multiplication accumulates into bytes it has not written yet.
        abi::add_immediate(&cursor, &record, INT_DATA_OFFSET),
        abi::store_u64(&cursor, abi::stack_pointer(), slots.data),
        abi::add_registers(&end, &cursor, &capacity),
        abi::label(&zero_loop),
        abi::compare_registers(&cursor, &end),
        abi::branch_eq(&zero_done),
        abi::store_u8(abi::ZERO, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&zero_loop),
        abi::label(&zero_done),
    ]);
    slots
}

/// Normalize the result in `result` to canonical form and leave its record address in
/// `RESULT_VALUE_REGISTER`. `count_slot` holds how many magnitude bytes the member
/// wrote (at most the capacity); `negative_slot` holds `1` for a negative result.
pub(crate) fn emit_build_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    result: &ResultSlots,
    count_slot: usize,
    negative_slot: usize,
    tag: &str,
) {
    let symbol = builder.current_symbol.clone();
    let record = vregs.next();
    let block = vregs.next();
    let data = vregs.next();
    let count = vregs.next();
    let negative = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&data, abi::stack_pointer(), result.data),
        abi::load_u64(&count, abi::stack_pointer(), count_slot),
        abi::load_u64(&negative, abi::stack_pointer(), negative_slot),
    ]);
    emit_trim(builder, vregs, &data, &count, &negative, &format!("{symbol}_{tag}_build"));
    builder.instructions.extend([
        abi::load_u64(&record, abi::stack_pointer(), result.record),
        abi::add_immediate(&block, &record, INT_MAGNITUDE_BLOCK),
        abi::store_u64(&count, &block, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &block, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&negative, &record, INT_OFFSET_NEGATIVE),
        abi::move_register(RESULT_VALUE_REGISTER, &record),
    ]);
}

/// Order the magnitudes of two loaded operands, ignoring sign: `-1`, `0` or `1` into
/// `out`. A longer significant count is larger; equal counts compare byte by byte from
/// the most significant end. Both operands are already trimmed by [`emit_load_int`], so
/// the count comparison is exact.
pub(crate) fn emit_compare_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    b: &IntSlots,
    out: &str,
    tag: &str,
) {
    let symbol = builder.current_symbol.clone();
    let less = format!("{symbol}_{tag}_less");
    let greater = format!("{symbol}_{tag}_greater");
    let equal = format!("{symbol}_{tag}_equal");
    let walk = format!("{symbol}_{tag}_walk");
    let finished = format!("{symbol}_{tag}_finished");
    let count_a = vregs.next();
    let count_b = vregs.next();
    let data_a = vregs.next();
    let data_b = vregs.next();
    let byte_a = vregs.next();
    let byte_b = vregs.next();
    let cursor = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&count_a, abi::stack_pointer(), a.count),
        abi::load_u64(&count_b, abi::stack_pointer(), b.count),
        abi::compare_registers(&count_a, &count_b),
        abi::branch_lo(&less),
        abi::branch_hi(&greater),
        abi::load_u64(&data_a, abi::stack_pointer(), a.data),
        abi::load_u64(&data_b, abi::stack_pointer(), b.data),
        abi::label(&walk),
        abi::compare_immediate(&count_a, "0"),
        abi::branch_eq(&equal),
        abi::subtract_immediate(&count_a, &count_a, 1),
        abi::add_registers(&cursor, &data_a, &count_a),
        abi::load_u8(&byte_a, &cursor, 0),
        abi::add_registers(&cursor, &data_b, &count_a),
        abi::load_u8(&byte_b, &cursor, 0),
        abi::compare_registers(&byte_a, &byte_b),
        abi::branch_lo(&less),
        abi::branch_hi(&greater),
        abi::branch(&walk),
        abi::label(&less),
        // `move_immediate` refuses a negative literal: build -1 as 0 - 1.
        abi::move_immediate(out, "Integer", "0"),
        abi::subtract_immediate(out, out, 1),
        abi::branch(&finished),
        abi::label(&greater),
        abi::move_immediate(out, "Integer", "1"),
        abi::branch(&finished),
        abi::label(&equal),
        abi::move_immediate(out, "Integer", "0"),
        abi::label(&finished),
    ]);
}

/// Order two loaded operands as signed numbers: `-1`, `0` or `1` into `out`. Differing
/// signs decide it outright (a trimmed zero is never negative, so zero sorts between
/// the negatives and the positives); equal signs defer to the magnitudes, reversed when
/// both are negative.
pub(crate) fn emit_compare_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    b: &IntSlots,
    out: &str,
    tag: &str,
) {
    let symbol = builder.current_symbol.clone();
    let same_sign = format!("{symbol}_{tag}_same_sign");
    let a_negative = format!("{symbol}_{tag}_a_negative");
    let finished = format!("{symbol}_{tag}_signed_done");
    let sign_a = vregs.next();
    let sign_b = vregs.next();
    let zero = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&sign_a, abi::stack_pointer(), a.negative),
        abi::load_u64(&sign_b, abi::stack_pointer(), b.negative),
        abi::compare_registers(&sign_a, &sign_b),
        abi::branch_eq(&same_sign),
        abi::compare_immediate(&sign_a, "0"),
        abi::branch_ne(&a_negative),
        abi::move_immediate(out, "Integer", "1"),
        abi::branch(&finished),
        abi::label(&a_negative),
        abi::move_immediate(out, "Integer", "0"),
        abi::subtract_immediate(out, out, 1),
        abi::branch(&finished),
        abi::label(&same_sign),
    ]);
    emit_compare_magnitude(builder, vregs, a, b, out, tag);
    let sign = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&sign, abi::stack_pointer(), a.negative),
        abi::compare_immediate(&sign, "0"),
        abi::branch_eq(&finished),
        abi::move_immediate(&zero, "Integer", "0"),
        abi::subtract_registers(out, &zero, out),
        abi::label(&finished),
    ]);
}

/// A new `big::Int` with `operand`'s magnitude and the sign in `negative_slot`
/// (`1` = negative), normalized, in `RESULT_VALUE_REGISTER`. Serves `abs` and `negate`.
/// Branches to `alloc_fail` when the result cannot be made.
pub(crate) fn emit_copy_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    operand: &IntSlots,
    negative_slot: usize,
    tag: &str,
    alloc_fail: &str,
) {
    let symbol = builder.current_symbol.clone();
    let result = emit_alloc_magnitude(builder, vregs, operand.count, tag, alloc_fail);
    let copy_loop = format!("{symbol}_{tag}_copy");
    let copy_done = format!("{symbol}_{tag}_copy_done");
    let src = vregs.next();
    let dst = vregs.next();
    let count = vregs.next();
    let index = vregs.next();
    let at = vregs.next();
    let byte = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&src, abi::stack_pointer(), operand.data),
        abi::load_u64(&dst, abi::stack_pointer(), result.data),
        abi::load_u64(&count, abi::stack_pointer(), operand.count),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &count),
        abi::branch_eq(&copy_done),
        abi::add_registers(&at, &src, &index),
        abi::load_u8(&byte, &at, 0),
        abi::add_registers(&at, &dst, &index),
        abi::store_u8(&byte, &at, 0),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
    ]);
    emit_build_int(builder, vregs, &result, operand.count, negative_slot, tag);
}

/// The canonical-form rule, emitted once for both directions: walk `count` down past
/// zero high bytes, then clear `negative` when nothing is left. `data` is preserved;
/// `count` and `negative` are updated in place.
fn emit_trim(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    data: &str,
    count: &str,
    negative: &str,
    label_base: &str,
) {
    let cursor = vregs.next();
    let byte = vregs.next();
    let trim_loop = format!("{label_base}_trim");
    let trim_done = format!("{label_base}_trim_done");
    let sign_ok = format!("{label_base}_sign_ok");
    builder.instructions.extend([
        abi::label(&trim_loop),
        abi::compare_immediate(count, "0"),
        abi::branch_eq(&trim_done),
        abi::add_registers(&cursor, data, count),
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_ne(&trim_done),
        abi::subtract_immediate(count, count, 1),
        abi::branch(&trim_loop),
        abi::label(&trim_done),
        abi::compare_immediate(count, "0"),
        abi::branch_ne(&sign_ok),
        abi::move_immediate(negative, "Integer", "0"),
        abi::label(&sign_ok),
    ]);
}
