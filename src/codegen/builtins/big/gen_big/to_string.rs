//! plan-176-R: `big::Int` decimal and radix text. The layout, slot views, build/compare/trim and release helpers
//! live in `big/gen_big/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// Text (plan-127-B Phase 4).
// ---------------------------------------------------------------------------

/// The digits of a loaded operand in `radix` (2..=36, in `radix_slot`) as a new `String`
/// in `RESULT_VALUE_REGISTER`: an optional `-`, then the digits most significant first,
/// `0`-`9` then lowercase `a`-`z`, and `0` for zero. Branches to `alloc_fail` when a
/// block cannot be made.
///
/// Chunked short division (bug-775): a working copy of the magnitude, read as 32-bit
/// limbs, is divided by `chunk = radix^k` — the largest power of the radix below
/// `2^32` (`10^9`, `2^31`, `16^7`, `36^6`) — from the high limb down. Every step's
/// dividend is `rem * 2^32 + limb < chunk * 2^32 <= 2^64`, one 64-bit `udiv`. Each pass
/// therefore yields `k` digits, least significant first, from its remainder: exactly `k`
/// (zero-padded) while the quotient is non-zero, and only the significant ones from the
/// last, top chunk, which is non-zero because the quotient before it was. The copy's
/// zero high limbs are dropped after each pass. The working copy and the digit buffer
/// are this helper's own and are released at the sizes they were made with.
pub(crate) fn emit_int_to_string(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    operand: &IntSlots,
    radix_slot: usize,
    tag: &str,
    alloc_fail: &str,
) {
    let symbol = builder.current_symbol.clone();
    let work_slot = builder.allocate_stack_object(&format!("big_{tag}_work"), 8);
    let work_size = builder.allocate_stack_object(&format!("big_{tag}_work_size"), 8);
    let digits_slot = builder.allocate_stack_object(&format!("big_{tag}_digits"), 8);
    let digits_size = builder.allocate_stack_object(&format!("big_{tag}_digits_size"), 8);
    let written_slot = builder.allocate_stack_object(&format!("big_{tag}_written"), 8);
    let string_slot = builder.allocate_stack_object(&format!("big_{tag}_string"), 8);
    let view = emit_word_view(builder, vregs, operand, &format!("{tag}_text"));

    // Sizes: the working copy holds the magnitude's words (plus one, so it is never
    // empty); radix 2 writes at most 8 digits a byte, plus the sign.
    let size = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&size, abi::stack_pointer(), view.words),
        abi::add_immediate(&size, &size, 1),
        abi::shift_left_immediate(&size, &size, 3),
        abi::store_u64(&size, abi::stack_pointer(), work_size),
        abi::load_u64(&size, abi::stack_pointer(), operand.count),
        abi::shift_left_immediate(&size, &size, 3),
        abi::add_immediate(&size, &size, 2),
        abi::store_u64(&size, abi::stack_pointer(), digits_size),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), work_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(
        &symbol,
        &mut builder.instructions,
        &mut builder.relocations,
        alloc_fail,
    );
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), work_slot),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), digits_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(
        &symbol,
        &mut builder.instructions,
        &mut builder.relocations,
        alloc_fail,
    );
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), digits_slot),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), written_slot),
    ]);

    // Copy the magnitude into the working buffer.
    emit_copy_words(builder, vregs, &view, work_slot, &format!("{tag}_text"));

    // Zero writes a single '0'.
    let digits_done = format!("{symbol}_{tag}_digits_done");
    let nonzero = format!("{symbol}_{tag}_nonzero");
    let (count, zero_buffer, zero_value) = (vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), operand.count),
        abi::compare_immediate(&count, "0"),
        abi::branch_ne(&nonzero),
        abi::load_u64(&zero_buffer, abi::stack_pointer(), digits_slot),
        abi::move_immediate(&zero_value, "Integer", "48"),
        abi::store_u8(&zero_value, &zero_buffer, 0),
        abi::move_immediate(&zero_value, "Integer", "1"),
        abi::store_u64(&zero_value, abi::stack_pointer(), written_slot),
        abi::branch(&digits_done),
        abi::label(&nonzero),
    ]);

    // chunk = radix^k, the largest power below 2^32 (chunk * radix cannot overflow:
    // chunk < 2^32 and radix <= 36).
    let grow = format!("{symbol}_{tag}_chunk_grow");
    let grown = format!("{symbol}_{tag}_chunk_grown");
    let chunked = format!("{symbol}_{tag}_chunked");
    let (radix, chunk, k, next, limit) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    builder.instructions.extend([
        abi::load_u64(&radix, abi::stack_pointer(), radix_slot),
        abi::move_register(&chunk, &radix),
        abi::move_immediate(&k, "Integer", "1"),
        abi::move_immediate(&limit, "Integer", "4294967296"),
        abi::label(&grow),
        abi::multiply_registers(&next, &chunk, &radix),
        abi::compare_registers(&next, &limit),
        abi::branch_lo(&grown),
        abi::branch(&chunked),
        abi::label(&grown),
        abi::move_register(&chunk, &next),
        abi::add_immediate(&k, &k, 1),
        abi::branch(&grow),
        abi::label(&chunked),
    ]);

    // One pass per chunk: divide the working copy by `chunk`, high limb down. The
    // whole digit loop is call-free, so its state stays in registers.
    let digit_loop = format!("{symbol}_{tag}_digit");
    let divide_loop = format!("{symbol}_{tag}_divide");
    let divided = format!("{symbol}_{tag}_divided");
    let trim_loop = format!("{symbol}_{tag}_trim");
    let trimmed = format!("{symbol}_{tag}_trimmed");
    let emit_digit = format!("{symbol}_{tag}_emit");
    let letter = format!("{symbol}_{tag}_letter");
    let placed = format!("{symbol}_{tag}_placed");
    let top_chunk = format!("{symbol}_{tag}_top_chunk");
    let (work, remaining, cursor, remainder, dividend, quotient, left, out, digit) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    builder.instructions.extend([
        abi::load_u64(&work, abi::stack_pointer(), work_slot),
        abi::load_u64(&out, abi::stack_pointer(), digits_slot),
        // remaining = ceil(count / 4) limbs.
        abi::load_u64(&remaining, abi::stack_pointer(), operand.count),
        abi::add_immediate(&remaining, &remaining, 3),
        abi::shift_right_immediate(&remaining, &remaining, 2),
        abi::label(&digit_loop),
        abi::shift_left_immediate(&cursor, &remaining, 2),
        abi::add_registers(&cursor, &work, &cursor),
        abi::move_immediate(&remainder, "Integer", "0"),
        abi::label(&divide_loop),
        abi::compare_registers(&cursor, &work),
        abi::branch_eq(&divided),
        abi::subtract_immediate(&cursor, &cursor, 4),
        abi::load_u32(&dividend, &cursor, 0),
        abi::shift_left_immediate(&remainder, &remainder, 32),
        abi::or_registers(&dividend, &dividend, &remainder),
        abi::unsigned_divide_registers(&quotient, &dividend, &chunk),
        abi::store_u32(&quotient, &cursor, 0),
        // remainder = dividend - quotient * chunk
        abi::multiply_subtract_registers(&remainder, &quotient, &chunk, &dividend),
        abi::branch(&divide_loop),
        abi::label(&divided),
        // Drop the working copy's zero high limbs: while work[remaining - 1] == 0.
        abi::label(&trim_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&trimmed),
        abi::shift_left_immediate(&cursor, &remaining, 2),
        abi::add_registers(&cursor, &work, &cursor),
        abi::subtract_immediate(&cursor, &cursor, 4),
        abi::load_u32(&dividend, &cursor, 0),
        abi::compare_immediate(&dividend, "0"),
        abi::branch_ne(&trimmed),
        abi::subtract_immediate(&remaining, &remaining, 1),
        abi::branch(&trim_loop),
        abi::label(&trimmed),
        // The chunk's digits, least significant first: k of them while a quotient is
        // left, only the significant ones for the top chunk.
        abi::move_register(&left, &k),
        abi::label(&emit_digit),
        abi::unsigned_divide_registers(&quotient, &remainder, &radix),
        abi::multiply_subtract_registers(&digit, &quotient, &radix, &remainder),
        abi::move_register(&remainder, &quotient),
        abi::compare_immediate(&digit, "10"),
        abi::branch_ge(&letter),
        abi::add_immediate(&digit, &digit, 48),
        abi::branch(&placed),
        abi::label(&letter),
        abi::add_immediate(&digit, &digit, 87),
        abi::label(&placed),
        abi::store_u8(&digit, &out, 0),
        abi::add_immediate(&out, &out, 1),
        abi::subtract_immediate(&left, &left, 1),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&top_chunk),
        abi::compare_immediate(&left, "0"),
        abi::branch_ne(&emit_digit),
        abi::branch(&digit_loop),
        abi::label(&top_chunk),
        abi::compare_immediate(&remainder, "0"),
        abi::branch_ne(&emit_digit),
        // written = out - digits
        abi::load_u64(&cursor, abi::stack_pointer(), digits_slot),
        abi::subtract_registers(&out, &out, &cursor),
        abi::store_u64(&out, abi::stack_pointer(), written_slot),
        abi::label(&digits_done),
    ]);

    // The sign, written after the digits so it lands first once reversed.
    let unsigned = format!("{symbol}_{tag}_unsigned");
    let flag = vregs.next();
    let written = vregs.next();
    let buffer = vregs.next();
    let minus = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&flag, abi::stack_pointer(), operand.negative),
        abi::compare_immediate(&flag, "0"),
        abi::branch_eq(&unsigned),
        abi::load_u64(&written, abi::stack_pointer(), written_slot),
        abi::load_u64(&buffer, abi::stack_pointer(), digits_slot),
        abi::add_registers(&buffer, &buffer, &written),
        abi::move_immediate(&minus, "Integer", "45"),
        abi::store_u8(&minus, &buffer, 0),
        abi::add_immediate(&written, &written, 1),
        abi::store_u64(&written, abi::stack_pointer(), written_slot),
        abi::label(&unsigned),
        // The String: length word, the bytes reversed, a trailing NUL.
        abi::load_u64(abi::return_register(), abi::stack_pointer(), written_slot),
        abi::add_immediate(abi::return_register(), abi::return_register(), 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(
        &symbol,
        &mut builder.instructions,
        &mut builder.relocations,
        alloc_fail,
    );
    let reverse_loop = format!("{symbol}_{tag}_reverse");
    let reversed = format!("{symbol}_{tag}_reversed");
    let string = vregs.next();
    let length = vregs.next();
    let source = vregs.next();
    let target = vregs.next();
    let character = vregs.next();
    let step = vregs.next();
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), string_slot),
        abi::load_u64(&string, abi::stack_pointer(), string_slot),
        abi::load_u64(&length, abi::stack_pointer(), written_slot),
        abi::store_u64(&length, &string, 0),
        abi::load_u64(&source, abi::stack_pointer(), digits_slot),
        abi::move_immediate(&step, "Integer", "0"),
        abi::label(&reverse_loop),
        abi::compare_registers(&step, &length),
        abi::branch_eq(&reversed),
        // string[8 + step] = digits[length - 1 - step]
        abi::subtract_registers(&target, &length, &step),
        abi::subtract_immediate(&target, &target, 1),
        abi::add_registers(&target, &source, &target),
        abi::load_u8(&character, &target, 0),
        abi::add_registers(&target, &string, &step),
        abi::store_u8(&character, &target, 8),
        abi::add_immediate(&step, &step, 1),
        abi::branch(&reverse_loop),
        abi::label(&reversed),
        abi::add_registers(&target, &string, &length),
        abi::store_u8(abi::ZERO, &target, 8),
    ]);

    // Release the working copy and the digit buffer at the sizes they were made with.
    for (pointer, size) in [(work_slot, work_size), (digits_slot, digits_size)] {
        builder.instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), pointer),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), size),
        ]);
        emit_arena_free(&symbol, &mut builder.instructions, &mut builder.relocations);
    }
    builder.instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        string_slot,
    ));
}
