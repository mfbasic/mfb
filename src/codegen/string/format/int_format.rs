//! The out-of-line integer decimal formatter (plan-118-C).
//!
//! `toString(Integer)` / `toString(Byte)` used to inline the whole conversion at
//! every call site: a 40-byte backward digit buffer, the divide/remainder loop,
//! the sign, the arena allocation, and a forward copy into the new String block.
//! Measured on `FUNC ts(n AS Integer) AS String RETURN toString(n)`, that is
//! ~100 machine instructions per site — and `call:toString` was 1,030,128
//! instructions over 5,826 sites, the fourth-largest expansion category in the
//! module.
//!
//! The float formatter beside this one (`float_format.rs`) already worked this
//! way; this is its integer twin, with the same ABI so the two call sites are
//! the same shape.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;

/// Internal symbol: `x0` = the value, `x1` = non-zero for a SIGNED render
/// (`Integer`) or zero for unsigned (`Byte`). Returns the standard allocation
/// Result: `x0` = tag, `x1` = String pointer (`{len, bytes…, NUL}`), or the
/// out-of-memory error Result.
pub(crate) const INT_TO_STRING_SYMBOL: &str = "_mfb_rt_int_to_string";

/// The backward digit buffer. 20 digits is the widest `u64`, plus a sign; 40
/// matches the inline version's slot so the two cannot disagree about capacity.
const LOCAL_SIZE: usize = 40;

/// Lower the integer formatter helper (vreg-allocated; emitted iff referenced).
pub(crate) fn lower_int_to_string_helper() -> CodeFunction {
    let symbol = INT_TO_STRING_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");

    let value = "%v20";
    let negative = "%v21";
    let length = "%v22";
    let cursor = "%v23";
    let divisor = "%v24";
    let quotient = "%v25";
    let digit = "%v26";
    let dst = "%v27";
    let block = "%v28";
    let signed = "%v29";

    let zero = l("zero");
    let nonnegative = l("nonnegative");
    let loop_start = l("loop");
    let digits_done = l("digits_done");
    let sign_done = l("sign_done");
    let unsigned = l("unsigned");
    let alloc_ok = l("alloc_ok");
    let copy_loop = l("copy_loop");
    let copy_done = l("copy_done");
    let alloc_error = l("alloc_error");
    let done = l("done");

    let mut relocations: Vec<CodeRelocation> = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    let mut ins: Vec<CodeInstruction> = vec![
        abi::label("entry"),
        abi::move_register(value, abi::c_arg(0)),
        abi::move_register(signed, abi::c_arg(1)),
        abi::move_immediate(negative, "Integer", "0"),
        abi::move_immediate(length, "Integer", "0"),
        abi::compare_immediate(value, "0"),
        abi::branch_eq(&zero),
        // A `Byte` renders unsigned, so only the signed form takes the negate.
        abi::compare_immediate(signed, "0"),
        abi::branch_eq(&unsigned),
        abi::compare_immediate(value, "0"),
        abi::branch_ge(&nonnegative),
        abi::subtract_registers(value, abi::ZERO, value),
        abi::move_immediate(negative, "Integer", "1"),
        abi::label(&nonnegative),
        abi::label(&unsigned),
        // Digits are written backward from the last byte of the scratch buffer.
        abi::add_immediate(cursor, abi::stack_pointer(), LOCAL_SIZE - 1),
        abi::move_immediate(divisor, "Integer", "10"),
        abi::label(&loop_start),
        abi::compare_immediate(value, "0"),
        abi::branch_eq(&digits_done),
        abi::unsigned_divide_registers(quotient, value, divisor),
        abi::multiply_subtract_registers(digit, quotient, divisor, value),
        abi::add_immediate(digit, digit, b'0' as usize),
        abi::store_u8(digit, cursor, 0),
        abi::subtract_immediate(cursor, cursor, 1),
        abi::add_immediate(length, length, 1),
        abi::move_register(value, quotient),
        abi::branch(&loop_start),
        abi::label(&zero),
        abi::add_immediate(cursor, abi::stack_pointer(), LOCAL_SIZE - 1),
        abi::move_immediate(digit, "Integer", &(b'0' as u64).to_string()),
        abi::store_u8(digit, cursor, 0),
        abi::subtract_immediate(cursor, cursor, 1),
        abi::move_immediate(length, "Integer", "1"),
        abi::label(&digits_done),
        abi::compare_immediate(negative, "0"),
        abi::branch_eq(&sign_done),
        abi::move_immediate(digit, "Integer", &(b'-' as u64).to_string()),
        abi::store_u8(digit, cursor, 0),
        abi::subtract_immediate(cursor, cursor, 1),
        abi::add_immediate(length, length, 1),
        abi::label(&sign_done),
        abi::add_immediate(cursor, cursor, 1),
        // `mfb.string.v1` is `u64 byteLength; bytes; NUL`, so 8 + len + 1.
        abi::add_immediate(abi::c_arg(0), length, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(block, RESULT_VALUE_REGISTER),
        abi::store_u64(length, block, 0),
        abi::add_immediate(dst, block, 8),
        abi::label(&copy_loop),
        abi::compare_immediate(length, "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8(digit, cursor, 0),
        abi::store_u8(digit, dst, 0),
        abi::add_immediate(cursor, cursor, 1),
        abi::add_immediate(dst, dst, 1),
        abi::subtract_immediate(length, length, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::move_immediate(digit, "Integer", "0"),
        abi::store_u8(digit, dst, 0),
        abi::move_register(RESULT_VALUE_REGISTER, block),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ];
    raise_error_into(symbol, "ErrOutOfMemory", &mut ins, &mut relocations);
    ins.extend([abi::label(&done), abi::return_()]);

    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], LOCAL_SIZE);
    CodeFunction {
        name: "runtime.intToString".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "String".to_string(),
        frame,
        stack_slots,
        instructions: ins,
        relocations,
    }
}
