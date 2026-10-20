//! plan-176-O: marshalling values across a `LINK` call — returns, strings to and from C
//! strings, and C floats. The link support blob, `ThunkContext` and `lower_link_initializer`
//! live in `codegen/link/thunk/link_thunk/mod.rs`.

use super::*;

/// Marshal the native return (`AS return <ctype>`) into the wrapper result in
/// `RESULT_VALUE_REGISTER` (plan-linker.md §12.3/§12.4).
pub(super) fn emit_return_passthrough(
    function: &IrLinkFunction,
    m: ReturnMarshal,
    symbol: &str,
    scratch: &ThunkScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let ThunkScratch {
        t0, t1, t2, t3, t4, ..
    } = scratch;
    let cret_off = m.cret_off;
    let status_off = m.status_off;
    // plan-113: exhaustive on `CAbiType`, so a new C type cannot be added
    // without deciding its return marshalling here. `None` is a spelling outside
    // the 16, which `ir::verify` already rejected -- kept as the same hard error.
    let Some(ctype) = function.abi_return_ctype.c_abi() else {
        return Err(format!(
            "LINK function '{}.{}' has unknown ABI return ctype '{}'",
            function.alias,
            function.name,
            function.abi_return_ctype.name()
        ));
    };
    match ctype {
        CAbiType::Ptr if matches!(function.return_type, crate::types::ParameterType::String) => {
            emit_copy_cstring_to_string(
                symbol,
                cret_off,
                m.alloc_fail,
                m.encoding_fail,
                scratch,
                instructions,
                relocations,
            );
        }
        // bug-788: a `float` return was widened into the same stash before here.
        CAbiType::Double | CAbiType::Float => {
            // §12.3: a C `double` may be NaN/Inf, but an MFBASIC `Float` is always
            // finite (04_types.md — an MFBASIC Float is always finite), so reject non-finite results at the boundary.
            // A non-finite double has all exponent bits set (`0x7FF0…`); the
            // mantissa then distinguishes Inf (zero) from NaN (non-zero).
            let finite = format!("{symbol}_float_finite");
            instructions.extend([
                abi::load_u64(t0, abi::stack_pointer(), m.cretd_off),
                abi::move_immediate(t1, "Integer", F64_POSITIVE_INF_BITS),
                abi::and_registers(t2, t0, t1),
                abi::compare_registers(t2, t1),
                abi::branch_ne(&finite),
                abi::move_immediate(t3, "Integer", F64_MANTISSA_MASK),
                abi::and_registers(t4, t0, t3),
                abi::compare_immediate(t4, "0"),
                abi::branch_eq(m.inf_fail),
                abi::branch(m.nan_fail),
                abi::label(&finite),
                abi::move_register(RESULT_VALUE_REGISTER, t0),
            ]);
        }
        CAbiType::Ptr | CAbiType::Int64 => {
            instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                cret_off,
            ));
        }
        CAbiType::Int32 => {
            instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                status_off,
            ));
        }
        CAbiType::Bool => {
            let set = format!("{symbol}_bool_true");
            let end = format!("{symbol}_bool_end");
            instructions.extend([
                abi::load_u64(t0, abi::stack_pointer(), cret_off),
                abi::compare_immediate(t0, "0"),
                abi::branch_ne(&set),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                abi::branch(&end),
                abi::label(&set),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                abi::label(&end),
            ]);
        }
        CAbiType::Byte => {
            instructions.extend([
                abi::load_u64(t0, abi::stack_pointer(), cret_off),
                abi::move_immediate(t1, "Integer", "255"),
                abi::and_registers(RESULT_VALUE_REGISTER, t0, t1),
            ]);
        }
        // The narrow integers carry no return-side normalization: the C ABI
        // already delivers them in the low bits of the return register, and the
        // wrapper's MFBASIC type is `Integer`. Listed explicitly (rather than
        // left to a default arm) so an unknown ctype is a hard error — plan-50-A.
        CAbiType::Int8
        | CAbiType::Int16
        | CAbiType::UInt8
        | CAbiType::UInt16
        | CAbiType::UInt32
        | CAbiType::UInt64 => {
            instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                cret_off,
            ));
        }
        // The three with no return-side meaning at all. `CString` is the
        // argument direction (a `char *` return is `CPtr` + a `String` wrapper);
        // `CVoid` returns nothing, so `returns_value` is false and this is never
        // called for it; `CBuffer` is rejected as the ABI return proper by
        // `check_buffer_slots` rule 5. All three are unreachable through
        // `ir::verify`, and stay the same hard error rather than a silent raw
        // 64-bit move — plan-50-A. plan-113 made them explicit arms because
        // `CAbiType` is exhaustively matched; the message is byte-identical to
        // the `other =>` default it replaced.
        CAbiType::Str | CAbiType::Void | CAbiType::Buffer => {
            return Err(format!(
                "LINK function '{}.{}' has unknown ABI return ctype '{}'",
                function.alias,
                function.name,
                ctype.name()
            ));
        }
    }
    Ok(())
}

/// Copy the NUL-free MFBASIC `String` at `[sp + str_off]` into a freshly arena
/// allocated NUL-terminated C buffer, storing the pointer at `[sp + out_off]`.
pub(super) fn emit_copy_string_to_cstring(
    symbol: &str,
    str_off: usize,
    out_off: usize,
    alloc_fail: &str,
    scratch: &ThunkScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let ThunkScratch {
        t0,
        t1,
        t2,
        t3,
        t4,
        t5,
        ..
    } = scratch;
    let loop_label = format!("{symbol}_cs{out_off}_copy");
    let done_label = format!("{symbol}_cs{out_off}_done");
    instructions.extend([
        abi::load_u64(t0, abi::stack_pointer(), str_off),
        abi::load_u64(t1, t0, 0),
        abi::add_immediate(abi::return_register(), t1, 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), out_off),
        abi::load_u64(t0, abi::stack_pointer(), str_off),
        abi::load_u64(t1, t0, 0),
        abi::add_immediate(t2, t0, 8),
        abi::move_register(t3, abi::mfb_return(1)),
        abi::move_immediate(t4, "Integer", "0"),
        abi::label(&loop_label),
        abi::compare_registers(t4, t1),
        abi::branch_eq(&done_label),
        abi::load_u8(t5, t2, 0),
        abi::store_u8(t5, t3, 0),
        abi::add_immediate(t2, t2, 1),
        abi::add_immediate(t3, t3, 1),
        abi::add_immediate(t4, t4, 1),
        abi::branch(&loop_label),
        abi::label(&done_label),
        abi::store_u8(abi::ZERO, t3, 0),
    ]);
}

/// Copy a NUL-terminated C string at `[sp + cret_off]` into an owned MFBASIC
/// `String`, leaving the result pointer in `RESULT_VALUE_REGISTER`
/// (plan-linker.md §12.4 copy-and-leave). A NULL pointer yields an empty
/// `String`.
/// The string-return scratch slot in the thunk frame.
pub(super) const RESULT_SAVE_OFF: usize = 24;

/// The status slot, reused as length scratch by the whole-return string copy.
/// Must track `STATUS_OFF`.
const STATUS_SCRATCH_OFF: usize = 8;

/// Copy the thunk's `const char *` return into an owned `String`, stashed in
/// [`RESULT_SAVE_OFF`].
///
/// This is the whole-return path, which runs once and last: the status has been
/// gated and nothing reads it again, so the STATUS slot is free length scratch.
/// A record's `String` FIELD does not come through here — a field is an inlined
/// sub-block of the record, not a separate String (see `marshal_struct_out`).
fn emit_copy_cstring_to_string(
    symbol: &str,
    cret_off: usize,
    alloc_fail: &str,
    encoding_fail: &str,
    scratch: &ThunkScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let ThunkScratch {
        t0,
        t1,
        t2,
        t3,
        t4,
        t5,
        ..
    } = scratch;
    let null_label = format!("{symbol}_ret_null");
    let len_loop = format!("{symbol}_ret_len");
    let len_done = format!("{symbol}_ret_len_done");
    let copy_loop = format!("{symbol}_ret_copy");
    let copy_done = format!("{symbol}_ret_copy_done");
    let ret_done = format!("{symbol}_ret_done");
    let ret_off = RESULT_SAVE_OFF;
    let len_off = STATUS_SCRATCH_OFF;
    instructions.extend([
        abi::load_u64(t0, abi::stack_pointer(), cret_off),
        abi::compare_immediate(t0, "0"),
        abi::branch_eq(&null_label),
        // strlen
        abi::move_register(t3, t0),
        abi::move_immediate(t1, "Integer", "0"),
        abi::label(&len_loop),
        abi::load_u8(t2, t3, 0),
        abi::compare_immediate(t2, "0"),
        abi::branch_eq(&len_done),
        abi::add_immediate(t3, t3, 1),
        abi::add_immediate(t1, t1, 1),
        abi::branch(&len_loop),
        abi::label(&len_done),
        abi::store_u64(t1, abi::stack_pointer(), len_off),
        abi::add_immediate(abi::return_register(), t1, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::load_u64(t1, abi::stack_pointer(), len_off),
        abi::store_u64(t1, abi::mfb_return(1), 0),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), ret_off),
        abi::load_u64(t2, abi::stack_pointer(), cret_off),
        abi::add_immediate(t3, abi::mfb_return(1), 8),
        abi::move_immediate(t4, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(t4, t1),
        abi::branch_eq(&copy_done),
        abi::load_u8(t5, t2, 0),
        abi::store_u8(t5, t3, 0),
        abi::add_immediate(t2, t2, 1),
        abi::add_immediate(t3, t3, 1),
        abi::add_immediate(t4, t4, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, t3, 0),
        // §12.4: returned bytes are validated as UTF-8 at the boundary.
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ret_off),
        abi::add_immediate(abi::return_register(), abi::return_register(), 8),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), len_off),
    ]);
    emit_call_validate_utf8(symbol, encoding_fail, instructions, relocations);
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), ret_off),
        abi::branch(&ret_done),
        // NULL -> empty string [u64 0][nul].
        abi::label(&null_label),
        abi::move_immediate(abi::return_register(), "Integer", "9"),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::ZERO, abi::mfb_return(1), 0),
        abi::store_u8(abi::ZERO, abi::mfb_return(1), 8),
        // Store on this path too: the save slot must be authoritative on BOTH
        // paths so a caller can read it from the frame rather than trusting a
        // physical register to survive (plan-50-F).
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), ret_off),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::label(&ret_done),
    ]);
}

/// The integer scratch vregs of one `LINK` thunk, minted once per thunk and
/// handed to every helper that emits into the thunk's instruction stream.
///
/// One allocator run sees the whole stream, so a register name IS the value:
/// the helpers below reuse the thunk's scratch by name, and they get those names
/// from here rather than from hand-numbered `"%vN"` literals — a literal would
/// alias whatever the `Vregs` counter minted N-th (bug-721). Minted in field
/// order, which is the order of the literals `%v9`..`%v16` they replaced, so the
/// allocator's `(start, vreg)` tie-break sees the same relative order.
/// bug-788: narrow the MFBASIC `Float` whose binary64 bits are in `value` to a C
/// `float`. It is rounded to binary32 (nearest, ties to even). A finite value too
/// large for binary32 fails with `ErrFloatOverflow` rather than reaching C as an
/// infinity. The binary32 bits are left NaN-boxed in `value` (upper 32 bits set):
/// that is the form a single must take in a RISC-V FP register, and it is
/// harmless elsewhere, where only the low 32 bits are read. A 4-byte store of
/// `value` writes exactly the C `float`.
pub(super) fn emit_narrow_to_cfloat(
    value: &VirtualRegister,
    tmp1: &VirtualRegister,
    tmp2: &VirtualRegister,
    overflow_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::float_move_d_from_x(abi::FP_SCRATCH[0], value),
        abi::float_convert_s_from_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0]),
        abi::float_move_x_from_d(value, abi::FP_SCRATCH[0]),
        abi::move_immediate(tmp1, "Integer", F32_POSITIVE_INF_BITS),
        abi::and_registers(tmp2, value, tmp1),
        abi::compare_registers(tmp2, tmp1),
        abi::branch_eq(overflow_fail),
    ]);
    emit_nan_box_cfloat(value, tmp1, instructions);
}

/// Keep the low 32 bits of `value` (a C `float`) and set the upper 32, the
/// NaN-boxed form `fcvt_d_from_s` requires on RISC-V.
fn emit_nan_box_cfloat(
    value: &VirtualRegister,
    tmp: &VirtualRegister,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::shift_left_immediate(value, value, 32),
        abi::shift_right_immediate(value, value, 32),
        abi::move_immediate(tmp, "Integer", "18446744069414584320"), // 0xFFFF_FFFF_0000_0000
        abi::or_registers(value, value, tmp),
    ]);
}

/// bug-788: widen the C `float` in the low 32 bits of `value` to the binary64
/// bits of an MFBASIC `Float` (exact). The caller still applies the finiteness
/// gate, since a C `float` may be NaN or an infinity.
pub(super) fn emit_widen_from_cfloat(
    value: &VirtualRegister,
    tmp: &VirtualRegister,
    instructions: &mut Vec<CodeInstruction>,
) {
    emit_nan_box_cfloat(value, tmp, instructions);
    instructions.extend([
        abi::float_move_d_from_x(abi::FP_SCRATCH[0], value),
        abi::float_convert_d_from_s(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0]),
        abi::float_move_x_from_d(value, abi::FP_SCRATCH[0]),
    ]);
}

/// Whether `function` narrows a wrapper `Float` parameter to a C `float` — a
/// `CFloat` argument slot or a `CFloat` `BIND IN` field fed by a parameter — and
/// so needs the `ErrFloatOverflow` exit (bug-788).
pub(super) fn function_narrows_cfloat(
    function: &IrLinkFunction,
    link_cstructs: &[crate::ir::IrCStruct],
) -> bool {
    let fed_by_param = |name: &str| function.params.iter().any(|(param, _)| param == name);
    let slot_narrows = function.abi_slots.iter().any(|slot| {
        !slot.direction.writes_back()
            && slot.ctype.is_c_abi(CAbiType::Float)
            && fed_by_param(&slot.name)
    });
    let field_narrows = function.bind_in.iter().any(|bind| {
        let Some(slot) = function.abi_slots.iter().find(|s| s.name == bind.slot) else {
            return false;
        };
        let Some(decl) = link_cstructs
            .iter()
            .find(|c| c.alias == function.alias && slot.ctype.is_named(&c.name))
        else {
            return false;
        };
        bind.fields.iter().any(|field| {
            field.param.is_some()
                && decl
                    .fields
                    .iter()
                    .find(|f| f.name == field.name)
                    .is_some_and(|f| f.ctype.is_c_abi(CAbiType::Float))
        })
    });
    slot_narrows || field_narrows
}

/// The 64-bit pattern a `CONST` pin or `BIND IN` integer literal stores for a C
/// slot or field of `ctype`. An integer literal names a numeric value, so a
/// floating C type receives that value converted, not the integer's raw bits
/// (bug-788).
pub(super) fn c_literal_bits(ctype: Option<CAbiType>, value: i64) -> u64 {
    match ctype {
        Some(CAbiType::Double) => (value as f64).to_bits(),
        Some(CAbiType::Float) => 0xFFFF_FFFF_0000_0000 | u64::from((value as f32).to_bits()),
        _ => value as u64,
    }
}
