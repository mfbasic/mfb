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
//!   size, rounded up to whole 64-bit words, with a zeroed magnitude region the member
//!   writes into directly. One block per result; no scratch buffer and no second copy
//!   (Open Decision 1).
//! - [`emit_build_int`] **normalizes** that result — drops trailing zero bytes, forces a
//!   zero value non-negative — and publishes it in the result register. It is the one
//!   place canonical form is established, so it is a property of the package rather
//!   than of each lowering.
//!
//! The arithmetic kernels read the byte magnitude as 64-bit words (bug-775; see "Word
//! access" below): add, subtract and multiply step a word at a time, and division and
//! text conversion a 32-bit limb at a time, so a quotient digit always comes from one
//! 64-bit `udiv`. The stored format stays the public little-endian `List OF Byte`.
//!
//! Everything is kept in frame slots between steps. The only call these emitters make
//! is the arena allocation, and every value a later step needs is reloaded from its
//! slot, so no register is assumed to survive it.

use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;

mod add_sub;
mod div_mod;
mod mul;
mod shift;
mod to_string;
pub(crate) use add_sub::emit_add_int;
use div_mod::emit_move_slot;
pub(crate) use div_mod::{emit_build_div_result, emit_div_mod_int, emit_rem_int};
// `div_mod.rs` keeps its `super::DIV_RESULT_TYPE_ID` (a `big` item) from before the split.
use super::DIV_RESULT_TYPE_ID;
pub(crate) use mul::{emit_fold_list, emit_mul_int};
pub(crate) use shift::{
    emit_reject_negative, emit_shift_left_magnitude, emit_shift_right_magnitude,
};
pub(crate) use to_string::emit_int_to_string;

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
    emit_trim(
        builder,
        vregs,
        &data,
        &count,
        &negative,
        &format!("{symbol}_{tag}_load"),
    );
    builder.instructions.extend([
        abi::store_u64(&count, abi::stack_pointer(), slots.count),
        abi::store_u64(&negative, abi::stack_pointer(), slots.negative),
    ]);
    slots
}

/// Make a result `big::Int` able to hold `capacity_slot` magnitude bytes: the record
/// block, its slot-0 offset, a kind-2 list header with `count` zero, and a zeroed
/// magnitude region. Branches to `alloc_fail` when the block cannot be made.
///
/// The region is rounded up to whole 64-bit words (bug-775), so the word kernels can
/// store whole words into any result: the header's capacity words record the rounded
/// size, which is the size a copy or a release reads, and the extra bytes start zero,
/// so a member that writes fewer than `capacity_slot` bytes still trims to the value
/// it computed.
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
    let rounded = builder.allocate_stack_object(&format!("big_{tag}_rounded"), 8);
    let size = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&size, abi::stack_pointer(), capacity_slot),
        abi::add_immediate(&size, &size, 7),
        abi::shift_right_immediate(&size, &size, 3),
        abi::shift_left_immediate(&size, &size, 3),
        abi::store_u64(&size, abi::stack_pointer(), rounded),
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
        abi::load_u64(&capacity, abi::stack_pointer(), rounded),
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
        // The magnitude region, zeroed a word at a time (its size is a whole number of
        // words): a member writes only the words it computes, and multiplication
        // accumulates into words it has not written yet.
        abi::add_immediate(&cursor, &record, INT_DATA_OFFSET),
        abi::store_u64(&cursor, abi::stack_pointer(), slots.data),
        abi::add_registers(&end, &cursor, &capacity),
        abi::label(&zero_loop),
        abi::compare_registers(&cursor, &end),
        abi::branch_eq(&zero_done),
        abi::store_u64(abi::ZERO, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::branch(&zero_loop),
        abi::label(&zero_done),
    ]);
    slots
}

// ---------------------------------------------------------------------------
// Word access (bug-775).
//
// The public magnitude is a little-endian `List OF Byte`, and every target is
// little-endian, so its bytes ARE a little-endian array of 64-bit words — except that
// an operand's byte count need not be a multiple of eight, and a hand-built operand's
// list can end exactly at its last byte. A word kernel therefore reads whole words
// from memory only below the operand's last word, and takes the last word from a frame
// slot that `emit_word_view` assembles byte by byte. No read goes past the operand's
// significant bytes, so no read can leave its block.
//
// A result is made by `emit_alloc_magnitude`, whose region is a whole number of zeroed
// words, so the kernels store whole words into it.
// ---------------------------------------------------------------------------

/// A loaded operand seen as little-endian 64-bit words.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WordSlots {
    /// Address of word 0 (the operand's data slot).
    pub(crate) data: usize,
    /// `ceil(count / 8)`: how many words the significant bytes span.
    pub(crate) words: usize,
    /// Word `words - 1`, assembled from its significant bytes (`0` for a zero operand).
    pub(crate) top: usize,
}

/// The word view of a loaded operand: its word count and its assembled top word, each
/// in a fresh frame slot. Reads only the operand's significant bytes.
pub(crate) fn emit_word_view(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    operand: &IntSlots,
    tag: &str,
) -> WordSlots {
    let symbol = builder.current_symbol.clone();
    let view = WordSlots {
        data: operand.data,
        words: builder.allocate_stack_object(&format!("big_{tag}_words"), 8),
        top: builder.allocate_stack_object(&format!("big_{tag}_top"), 8),
    };
    let assemble = format!("{symbol}_{tag}_top_word");
    let assembled = format!("{symbol}_{tag}_top_word_done");
    let (count, words, start, top, cursor, byte) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), operand.count),
        abi::add_immediate(&words, &count, 7),
        abi::shift_right_immediate(&words, &words, 3),
        abi::store_u64(&words, abi::stack_pointer(), view.words),
        abi::move_immediate(&top, "Integer", "0"),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq(&assembled),
        // start = &data[8 * (words - 1)], cursor = &data[count]; gather the bytes
        // between them, most significant first.
        abi::subtract_immediate(&start, &words, 1),
        abi::shift_left_immediate(&start, &start, 3),
        abi::load_u64(&cursor, abi::stack_pointer(), operand.data),
        abi::add_registers(&start, &cursor, &start),
        abi::add_registers(&cursor, &cursor, &count),
        abi::label(&assemble),
        abi::compare_registers(&cursor, &start),
        abi::branch_eq(&assembled),
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::load_u8(&byte, &cursor, 0),
        abi::shift_left_immediate(&top, &top, 8),
        abi::or_registers(&top, &top, &byte),
        abi::branch(&assemble),
        abi::label(&assembled),
        abi::store_u64(&top, abi::stack_pointer(), view.top),
    ]);
    view
}

/// `dst = word index` of `view`, for `index < words`: memory below the last word, the
/// assembled top word at it. `data` holds the word-0 address and `last` holds
/// `words - 1`. `label_base` must be unique to the call site.
#[allow(clippy::too_many_arguments)]
fn emit_word_at(
    builder: &mut CodeBuilder,
    view: &WordSlots,
    data: impl Into<Reg>,
    last: impl Into<Reg>,
    index: impl Into<Reg>,
    dst: impl Into<Reg>,
    address: impl Into<Reg>,
    label_base: &str,
) {
    let (data, last, index, dst, address): (Reg, Reg, Reg, Reg, Reg) = (
        data.into(),
        last.into(),
        index.into(),
        dst.into(),
        address.into(),
    );
    let use_top = format!("{label_base}_top");
    let done = format!("{label_base}_done");
    builder.instructions.extend([
        abi::compare_registers(index, last),
        abi::branch_eq(&use_top),
        abi::shift_left_immediate(address, index, 3),
        abi::add_registers(address, data, address),
        abi::load_u64(dst, address, 0),
        abi::branch(&done),
        abi::label(&use_top),
        abi::load_u64(dst, abi::stack_pointer(), view.top),
        abi::label(&done),
    ]);
}

/// `dst = word index` of `view`, or zero at and past `words` (held in `words`).
#[allow(clippy::too_many_arguments)]
fn emit_word_or_zero(
    builder: &mut CodeBuilder,
    view: &WordSlots,
    data: impl Into<Reg>,
    words: impl Into<Reg>,
    last: impl Into<Reg>,
    index: impl Into<Reg>,
    dst: impl Into<Reg>,
    address: impl Into<Reg>,
    label_base: &str,
) {
    let (index, dst): (Reg, Reg) = (index.into(), dst.into());
    let present = format!("{label_base}_present");
    let done = format!("{label_base}_absent_done");
    builder.instructions.extend([
        abi::compare_registers(index, words),
        abi::branch_lo(&present),
        abi::move_immediate(dst, "Integer", "0"),
        abi::branch(&done),
        abi::label(&present),
    ]);
    emit_word_at(builder, view, data, last, index, dst, address, label_base);
    builder.instructions.push(abi::label(&done));
}

/// Copy `view`'s words into the buffer whose address is in `dst_slot`: `words - 1`
/// whole words, then the assembled top word. The buffer must hold `8 * words` bytes.
pub(crate) fn emit_copy_words(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    view: &WordSlots,
    dst_slot: usize,
    tag: &str,
) {
    let symbol = builder.current_symbol.clone();
    let copy = format!("{symbol}_{tag}_wcopy");
    let copied = format!("{symbol}_{tag}_wcopy_done");
    let empty = format!("{symbol}_{tag}_wcopy_empty");
    let (src, dst, end, word) = (vregs.next(), vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&end, abi::stack_pointer(), view.words),
        abi::compare_immediate(&end, "0"),
        abi::branch_eq(&empty),
        abi::load_u64(&src, abi::stack_pointer(), view.data),
        abi::load_u64(&dst, abi::stack_pointer(), dst_slot),
        // end = &src[words - 1]: the top word comes from its slot.
        abi::subtract_immediate(&end, &end, 1),
        abi::shift_left_immediate(&end, &end, 3),
        abi::add_registers(&end, &src, &end),
        abi::label(&copy),
        abi::compare_registers(&src, &end),
        abi::branch_eq(&copied),
        abi::load_u64(&word, &src, 0),
        abi::store_u64(&word, &dst, 0),
        abi::add_immediate(&src, &src, 8),
        abi::add_immediate(&dst, &dst, 8),
        abi::branch(&copy),
        abi::label(&copied),
        abi::load_u64(&word, abi::stack_pointer(), view.top),
        abi::store_u64(&word, &dst, 0),
        abi::label(&empty),
    ]);
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
    emit_trim(
        builder,
        vregs,
        &data,
        &count,
        &negative,
        &format!("{symbol}_{tag}_build"),
    );
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
    out: impl Into<Reg>,
    tag: &str,
) {
    let out: Reg = out.into();
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
    out: impl Into<Reg>,
    tag: &str,
) {
    let out: Reg = out.into();
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
    let view = emit_word_view(builder, vregs, operand, &format!("{tag}_src"));
    let result = emit_alloc_magnitude(builder, vregs, operand.count, tag, alloc_fail);
    emit_copy_words(builder, vregs, &view, result.data, tag);
    emit_build_int(builder, vregs, &result, operand.count, negative_slot, tag);
}

/// The canonical-form rule, emitted once for both directions: walk `count` down past
/// zero high bytes, then clear `negative` when nothing is left. `data` is preserved;
/// `count` and `negative` are updated in place.
fn emit_trim(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    data: impl Into<Reg>,
    count: impl Into<Reg>,
    negative: impl Into<Reg>,
    label_base: &str,
) {
    let data: Reg = data.into();
    let (count, negative): (Reg, Reg) = (count.into(), negative.into());
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

// ---------------------------------------------------------------------------
// Signed division and `DivResult` (plan-127-C Phase 2).
// ---------------------------------------------------------------------------

use crate::codegen::memory::marshal::{
    emit_build_inlined_record_sized, MarshalRegs, RecordBuildScratch,
};
use crate::types::ParameterType;

/// Release the `big::Int` record whose address is in `record_slot` — a helper's own
/// intermediate that the caller never sees — at the size it was made with
/// (`INT_DATA_OFFSET + dataCapacity`).
pub(crate) fn emit_release_int(builder: &mut CodeBuilder, vregs: &mut Vregs, record_slot: usize) {
    let symbol = builder.current_symbol.clone();
    let (record, size) = (vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&record, abi::stack_pointer(), record_slot),
        abi::add_immediate(&size, &record, INT_MAGNITUDE_BLOCK),
        abi::load_u64(&size, &size, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&size, &size, INT_DATA_OFFSET),
        abi::move_register(abi::c_arg(0), &record),
        abi::move_register(abi::c_arg(1), &size),
    ]);
    emit_arena_free(&symbol, &mut builder.instructions, &mut builder.relocations);
}

// ---------------------------------------------------------------------------
// Values from `Integer`s (plan-127-C Phase 3).
// ---------------------------------------------------------------------------

/// A normalized `big::Int` equal to the `Integer` in `value_slot`, in
/// `RESULT_VALUE_REGISTER`: the absolute value as eight little-endian bytes (two's
/// complement negation read unsigned, exact even for `Integer`'s minimum), trimmed by
/// `emit_build_int`. Branches to `alloc_fail` when the block cannot be made. This is
/// `big::fromInteger`'s body, shared with the members that build intermediate values from
/// an `Integer` counter.
pub(crate) fn emit_int_from_integer(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    value_slot: usize,
    tag: &str,
    alloc_fail: &str,
) {
    let symbol = builder.current_symbol.clone();
    let capacity = builder.allocate_stack_object(&format!("big_{tag}_capacity"), 8);
    let negative = builder.allocate_stack_object(&format!("big_{tag}_negative"), 8);
    let positive = format!("{symbol}_{tag}_positive");
    let write_loop = format!("{symbol}_{tag}_write");
    let write_done = format!("{symbol}_{tag}_write_done");
    let eight = vregs.next();
    builder.instructions.extend([
        abi::move_immediate(&eight, "Integer", "8"),
        abi::store_u64(&eight, abi::stack_pointer(), capacity),
    ]);
    let result = emit_alloc_magnitude(builder, vregs, capacity, tag, alloc_fail);
    let (value, flag, zero, cursor, index) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    builder.instructions.extend([
        abi::load_u64(&value, abi::stack_pointer(), value_slot),
        abi::move_immediate(&flag, "Integer", "0"),
        abi::compare_immediate(&value, "0"),
        abi::branch_ge(&positive),
        abi::move_immediate(&flag, "Integer", "1"),
        abi::move_immediate(&zero, "Integer", "0"),
        abi::subtract_registers(&value, &zero, &value),
        abi::label(&positive),
        abi::store_u64(&flag, abi::stack_pointer(), negative),
        abi::load_u64(&cursor, abi::stack_pointer(), result.data),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&write_loop),
        abi::compare_immediate(&index, "8"),
        abi::branch_eq(&write_done),
        abi::store_u8(&value, &cursor, 0),
        abi::shift_right_immediate(&value, &value, 8),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&write_loop),
        abi::label(&write_done),
    ]);
    emit_build_int(builder, vregs, &result, capacity, negative, tag);
}
