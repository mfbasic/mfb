//! Target-generic native lowering for the `bits` package (plan-95 migration).
//!
//! These were `CodeBuilder` methods in the former
//! `src/target/shared/code/builder_bits.rs`. Each `bits` member's
//! `Implementation::Native` `common` slot points at a thin per-member
//! [`crate::codegen::registry::NativeLower`] wrapper (in its `func_*.rs`) that
//! calls the shared group emitter here. Emit-only through `abi::`. The `abi::`
//! register operands are passed by value (`VirtualRegister` is `Copy`) rather than
//! by reference, which is the only shape change from the relocated originals — the
//! emitted instructions are identical.

use crate::target::shared::abi;
use crate::target::shared::code::mir;
use crate::target::shared::code::{CodeBuilder, Operand, ValueResult, VirtualRegister};
use crate::target::shared::nir::NirValue;

// 64-bit population-count masks (SWAR Hamming weight), as decimal so they round
// trip through `move_immediate`'s arbitrary-constant path.
const POPCOUNT_MASK_5555: &str = "6148914691236517205"; // 0x5555555555555555
const POPCOUNT_MASK_3333: &str = "3689348814741910323"; // 0x3333333333333333
const POPCOUNT_MASK_0F0F: &str = "1085102592571150095"; // 0x0F0F0F0F0F0F0F0F
const POPCOUNT_MASK_0101: &str = "72340172838076673"; //  0x0101010101010101

/// Lower the two `Integer` operands of a binary `bits` op into fresh registers,
/// spilling the first across the second lowering so a temporary reset cannot
/// clobber it.
pub(crate) fn lower_bits_two_integers(
    builder: &mut CodeBuilder,
    function: &str,
    args: &[NirValue],
) -> Result<(VirtualRegister, VirtualRegister, String, String), String> {
    let left = builder.lower_value(&args[0])?;
    if left.type_ != "Integer" {
        return Err(format!("bits.{function} does not accept {}", left.type_));
    }
    let left_slot = builder.allocate_stack_object("bits_left", 8);
    builder.emit(abi::store_u64(
        &left.location,
        abi::stack_pointer(),
        left_slot,
    ));
    let right = builder.lower_value(&args[1])?;
    if right.type_ != "Integer" {
        return Err(format!("bits.{function} does not accept {}", right.type_));
    }
    let right_slot = builder.allocate_stack_object("bits_right", 8);
    builder.emit(abi::store_u64(
        &right.location,
        abi::stack_pointer(),
        right_slot,
    ));
    builder.reset_temporary_registers();
    let left_reg = builder.allocate_register()?;
    let right_reg = builder.allocate_register()?;
    builder.emit(abi::load_u64(left_reg, abi::stack_pointer(), left_slot));
    builder.emit(abi::load_u64(right_reg, abi::stack_pointer(), right_slot));
    Ok((left_reg, right_reg, left.text, right.text))
}

pub(crate) fn lower_bits_binary(
    builder: &mut CodeBuilder,
    function: &str,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let (left_reg, right_reg, left_text, right_text) =
        lower_bits_two_integers(builder, function, args)?;
    let dst = builder.allocate_register()?;
    match function {
        "band" => builder.emit(abi::and_registers(dst, left_reg, right_reg)),
        "bor" => builder.emit(abi::or_registers(dst, left_reg, right_reg)),
        "bxor" => builder.emit(abi::exclusive_or_registers(dst, left_reg, right_reg)),
        other => {
            return Err(format!(
                "native bits lowering does not support bits.{other}"
            ))
        }
    }
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.{function}({left_text}, {right_text})"),
    })
}

/// Lower a single `bits.*` argument and require it to be `Integer`, returning the
/// lowered value or the shared `does not accept` diagnostic (bug-332 G5).
pub(crate) fn lower_bits_one_integer(
    builder: &mut CodeBuilder,
    function: &str,
    arg: &NirValue,
) -> Result<ValueResult, String> {
    let value = builder.lower_value(arg)?;
    if value.type_ != "Integer" {
        return Err(format!("bits.{function} does not accept {}", value.type_));
    }
    Ok(value)
}

pub(crate) fn lower_bits_not(
    builder: &mut CodeBuilder,
    arg: &NirValue,
) -> Result<ValueResult, String> {
    let value = lower_bits_one_integer(builder, "bnot", arg)?;
    let dst = builder.allocate_register()?;
    builder.emit(abi::bitwise_not(dst, &value.location));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.bnot({})", value.text),
    })
}

/// `sl`/`sr`/`sra` — variable shift after validating `count` is in `0 .. 63`. An
/// out-of-range count fails `ErrInvalidArgument`.
pub(crate) fn lower_bits_shift(
    builder: &mut CodeBuilder,
    function: &str,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let (value_reg, count_reg, value_text, count_text) =
        lower_bits_two_integers(builder, function, args)?;
    let valid = builder.label("bits_shift_valid");
    let out_of_range = builder.label("bits_shift_out_of_range");
    builder.emit(abi::compare_immediate(count_reg, "0"));
    builder.emit(abi::branch_lt(&out_of_range));
    builder.emit(abi::compare_immediate(count_reg, "63"));
    builder.emit(abi::branch_le(&valid));
    builder.emit(abi::label(&out_of_range));
    builder.raise_error_bare("ErrInvalidArgument")?;
    builder.emit(abi::label(&valid));
    let dst = builder.allocate_register()?;
    match function {
        "sl" => builder.emit(abi::shift_left_variable(dst, value_reg, count_reg)),
        "sr" => builder.emit(abi::shift_right_variable(dst, value_reg, count_reg)),
        "sra" => builder.emit(abi::arithmetic_shift_right_variable(dst, value_reg, count_reg)),
        other => {
            return Err(format!(
                "native bits lowering does not support bits.{other}"
            ))
        }
    }
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.{function}({value_text}, {count_text})"),
    })
}

/// `rl32`/`rr32`/`rl64`/`rr64` — total rotates. AArch64 has only rotate-right
/// (`RORV`), so a left rotate by `count` becomes a right rotate by `-count` (the
/// hardware reduces the amount modulo the width).
pub(crate) fn lower_bits_rotate(
    builder: &mut CodeBuilder,
    function: &str,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let (value_reg, count_reg, value_text, count_text) =
        lower_bits_two_integers(builder, function, args)?;
    let dst = builder.allocate_register()?;
    match function {
        "rr64" => builder.emit(abi::rotate_right_registers(dst, value_reg, count_reg)),
        "rl64" => {
            let neg = builder.allocate_register()?;
            builder.emit(abi::subtract_registers(neg, abi::ZERO, count_reg));
            builder.emit(abi::rotate_right_registers(dst, value_reg, neg));
        }
        "rr32" => builder.emit(abi::rotate_right_word_registers(dst, value_reg, count_reg)),
        "rl32" => {
            let neg = builder.allocate_register()?;
            builder.emit(abi::subtract_registers(neg, abi::ZERO, count_reg));
            builder.emit(abi::rotate_right_word_registers(dst, value_reg, neg));
        }
        other => {
            return Err(format!(
                "native bits lowering does not support bits.{other}"
            ))
        }
    }
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.{function}({value_text}, {count_text})"),
    })
}

/// `clz`/`ctz`. `ctz` reverses the bits (`RBIT`) and then counts leading zeros;
/// both return `64` for a zero input.
pub(crate) fn lower_bits_count_zeros(
    builder: &mut CodeBuilder,
    function: &str,
    arg: &NirValue,
) -> Result<ValueResult, String> {
    let value = lower_bits_one_integer(builder, function, arg)?;
    let dst = builder.allocate_register()?;
    match function {
        "clz" => builder.emit(abi::count_leading_zeros(dst, &value.location)),
        "ctz" => {
            let reversed = builder.allocate_register()?;
            builder.emit(abi::reverse_bits(reversed, &value.location));
            builder.emit(abi::count_leading_zeros(dst, reversed));
        }
        other => {
            return Err(format!(
                "native bits lowering does not support bits.{other}"
            ))
        }
    }
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.{function}({})", value.text),
    })
}

/// `popCount` — 64-bit Hamming weight via the standard SWAR sequence (no SIMD, so
/// it lowers entirely with the integer ALU ops the codegen already owns).
pub(crate) fn lower_bits_popcount(
    builder: &mut CodeBuilder,
    arg: &NirValue,
) -> Result<ValueResult, String> {
    let value = lower_bits_one_integer(builder, "popCount", arg)?;
    let text = format!("bits.popCount({})", value.text);

    // plan-39 K2: on AArch64 the 64-bit Hamming weight is a short NEON sequence —
    // move the value into a `d` register, `CNT` per byte, `ADDV` the 8 byte-counts
    // into lane 0, and move the (0..=64) sum back — instead of the 12-instruction
    // SWAR. Other ISAs keep the portable SWAR below.
    if mir::active_backend().is_aarch64() {
        let dst = builder.allocate_register()?;
        builder.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[0], &value.location));
        builder.emit(abi::vector_cnt8b(abi::VEC_SCRATCH[0], abi::VEC_SCRATCH[0]));
        builder.emit(abi::vector_addv8b(abi::VEC_SCRATCH[0], abi::VEC_SCRATCH[0]));
        builder.emit(abi::vector_extract_to_x(dst, abi::VEC_SCRATCH[0], 0));
        return Ok(ValueResult {
            type_: "Integer".to_string(),
            location: Operand::from(dst.render()),
            text,
        });
    }

    let acc = builder.allocate_register()?;
    let temp = builder.allocate_register()?;
    let mask = builder.allocate_register()?;
    builder.emit(abi::move_register(acc, &value.location));

    // acc = acc - ((acc >> 1) & 0x5555...)
    builder.emit(abi::shift_right_immediate(temp, acc, 1));
    builder.emit(abi::move_immediate(mask, "Integer", POPCOUNT_MASK_5555));
    builder.emit(abi::and_registers(temp, temp, mask));
    builder.emit(abi::subtract_registers(acc, acc, temp));

    // acc = (acc & 0x3333...) + ((acc >> 2) & 0x3333...)
    builder.emit(abi::move_immediate(mask, "Integer", POPCOUNT_MASK_3333));
    let low = builder.allocate_register()?;
    builder.emit(abi::and_registers(low, acc, mask));
    builder.emit(abi::shift_right_immediate(temp, acc, 2));
    builder.emit(abi::and_registers(temp, temp, mask));
    builder.emit(abi::add_registers(acc, low, temp));

    // acc = (acc + (acc >> 4)) & 0x0F0F...
    builder.emit(abi::shift_right_immediate(temp, acc, 4));
    builder.emit(abi::add_registers(acc, acc, temp));
    builder.emit(abi::move_immediate(mask, "Integer", POPCOUNT_MASK_0F0F));
    builder.emit(abi::and_registers(acc, acc, mask));

    // acc = (acc * 0x0101...) >> 56
    builder.emit(abi::move_immediate(mask, "Integer", POPCOUNT_MASK_0101));
    builder.emit(abi::multiply_registers(acc, acc, mask));
    builder.emit(abi::shift_right_immediate(acc, acc, 56));

    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(acc.render()),
        text,
    })
}

/// `bswap16`/`bswap32`/`bswap64` — byte reversal. The 16/32-bit forms clear the
/// bits above their width: `REV` on the `W` register zero-extends, and the 16-bit
/// form additionally shifts the reversed low half into place.
pub(crate) fn lower_bits_bswap(
    builder: &mut CodeBuilder,
    function: &str,
    arg: &NirValue,
) -> Result<ValueResult, String> {
    let value = lower_bits_one_integer(builder, function, arg)?;
    let dst = builder.allocate_register()?;
    match function {
        "bswap16" => {
            // REV of the low word puts the two low bytes at bits [31:16]; a logical
            // >>16 drops the other two bytes and clears bits 16..63.
            builder.emit(abi::reverse_bytes_word(dst, &value.location));
            builder.emit(abi::shift_right_immediate(dst, dst, 16));
        }
        "bswap32" => builder.emit(abi::reverse_bytes_word(dst, &value.location)),
        "bswap64" => builder.emit(abi::reverse_bytes(dst, &value.location)),
        other => {
            return Err(format!(
                "native bits lowering does not support bits.{other}"
            ))
        }
    }
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.{function}({})", value.text),
    })
}
