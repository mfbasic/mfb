//! plan-176-O: marshalling C structs in and out of a `LINK` call. The link support blob,
//! `ThunkContext` and `lower_link_initializer` live in `codegen/link/thunk/link_thunk/mod.rs`.

use super::*;

/// Emit the sized store for a struct field of `ctype` at `[sp + off]` from `src`
/// (plan-50-E §4.4).
fn store_field(ctype: CAbiType, src: impl Into<abi::Reg>, off: usize) -> CodeInstruction {
    match ctype {
        CAbiType::Int8 | CAbiType::UInt8 | CAbiType::Bool | CAbiType::Byte => {
            abi::store_u8(src, abi::stack_pointer(), off)
        }
        CAbiType::Int16 | CAbiType::UInt16 => abi::store_u16(src, abi::stack_pointer(), off),
        CAbiType::Int32 | CAbiType::UInt32 | CAbiType::Float => {
            abi::store_u32(src, abi::stack_pointer(), off)
        }
        // The 8-byte fields, plus the three that are never struct fields at all
        // (`ctype_size_align` answers `None` for `CVoid`/`CBuffer`, so
        // `check_cstruct` rejects them before lowering). Enumerated rather than
        // left to `_` (plan-113) so a new C type must decide its store width.
        CAbiType::Int64
        | CAbiType::UInt64
        | CAbiType::Double
        | CAbiType::Ptr
        | CAbiType::Str
        | CAbiType::Void
        | CAbiType::Buffer => abi::store_u64(src, abi::stack_pointer(), off),
    }
}

/// Emit the sized load for a struct field of `ctype` at `[sp + off]` into `dst`.
///
/// Every load zero-extends, so a SIGNED narrow field is sign-extended by the
/// caller — otherwise a `CInt32 sections = -1` surfaces as 4294967295, which is
/// exactly bug-238.
fn load_field(ctype: CAbiType, dst: impl Into<abi::Reg>, off: usize) -> CodeInstruction {
    match ctype {
        CAbiType::Int8 | CAbiType::UInt8 | CAbiType::Bool | CAbiType::Byte => {
            abi::load_u8(dst, abi::stack_pointer(), off)
        }
        CAbiType::Int16 | CAbiType::UInt16 => abi::load_u16(dst, abi::stack_pointer(), off),
        CAbiType::Int32 | CAbiType::UInt32 | CAbiType::Float => {
            abi::load_u32(dst, abi::stack_pointer(), off)
        }
        // See `store_field`: the 8-byte fields, plus the three that can never be
        // struct fields.
        CAbiType::Int64
        | CAbiType::UInt64
        | CAbiType::Double
        | CAbiType::Ptr
        | CAbiType::Str
        | CAbiType::Void
        | CAbiType::Buffer => abi::load_u64(dst, abi::stack_pointer(), off),
    }
}

#[allow(clippy::too_many_arguments)]
/// Write the `BIND IN` fields into a struct slot's buffer before the call
/// (plan-50-E §4.4).
///
/// Only bound fields are written; the buffer is already fully zeroed, so every
/// other field is 0. Reads a wrapper PARAMETER (or an immediate) — there is no
/// record on the input side, hence none of the register-lifetime hazard that
/// `marshal_struct_out` has.
pub(super) fn marshal_struct_in(
    function: &IrLinkFunction,
    decl: &crate::ir::IrCStruct,
    layout: &crate::ir::CLayout,
    buf_off: usize,
    slot_name: &str,
    symbol: &str,
    param_base: usize,
    param_index: &HashMap<&str, usize>,
    range_fail: &str,
    float_overflow_fail: &str,
    alloc_fail: &str,
    scratch: &ThunkScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let ThunkScratch { t1, t2, t3, .. } = scratch;
    let Some(bind) = function.bind_in.iter().find(|b| b.slot == slot_name) else {
        return Ok(());
    };
    for field in &bind.fields {
        let Some(pos) = decl.fields.iter().position(|f| f.name == field.name) else {
            return Err(format!(
                "LINK function '{}.{}' BIND IN sets unknown field '{}' of CSTRUCT '{}'",
                function.alias, function.name, field.name, decl.name
            ));
        };
        // `layout` is `decl`'s (`compute_c_layout`, which refuses a field with no
        // C ABI type), so every field has its type there.
        let ctype = layout.ctypes[pos];
        let off = buf_off + layout.offsets[pos];

        // plan-50-F: a `CString` field takes an MFBASIC String and gives C a
        // NUL-terminated copy that lives for the call. Handled before the integer
        // path: it is a pointer, not a value to range-check.
        if ctype == CAbiType::Str {
            let Some(param) = &field.param else {
                return Err(format!(
                    "LINK function '{}.{}' BIND IN field '{}' is a CString and must bind a String parameter, not a literal",
                    function.alias, function.name, field.name
                ));
            };
            let Some(&pidx) = param_index.get(param.as_str()) else {
                return Err(format!(
                    "LINK function '{}.{}' BIND IN binds field '{}' to unknown parameter '{param}'",
                    function.alias, function.name, field.name
                ));
            };
            // Allocates (clobbering x0-x17), but nothing is live across it: the
            // source is a frame slot and the destination is sp-relative.
            emit_copy_string_to_cstring(
                symbol,
                param_base + pidx * 8,
                off,
                alloc_fail,
                scratch,
                instructions,
                relocations,
            );
            continue;
        }

        if let Some(literal) = field.literal {
            instructions.push(abi::move_immediate(
                t1,
                "Integer",
                &c_literal_bits(Some(ctype), literal).to_string(),
            ));
        } else if let Some(param) = &field.param {
            let Some(&pidx) = param_index.get(param.as_str()) else {
                return Err(format!(
                    "LINK function '{}.{}' BIND IN binds field '{}' to unknown parameter '{param}'",
                    function.alias, function.name, field.name
                ));
            };
            instructions.push(abi::load_u64(
                t1,
                abi::stack_pointer(),
                param_base + pidx * 8,
            ));
            if ctype == CAbiType::Float {
                emit_narrow_to_cfloat(t1, t2, t3, float_overflow_fail, instructions);
            }
        } else {
            return Err(format!(
                "LINK function '{}.{}' BIND IN field '{}' binds neither a parameter nor a literal",
                function.alias, function.name, field.name
            ));
        }

        // A 64-bit MFBASIC Integer must fit the C field's width. Truncating
        // silently is the bug-238 class; the existing CInt32 argument path
        // range-checks for exactly this reason, and a struct field is no different.
        if let Some(bits) = narrow_signed_bits(ctype) {
            instructions.extend([
                abi::shift_left_immediate(t2, t1, (64 - bits) as u8),
                abi::arithmetic_shift_right_immediate(t2, t2, (64 - bits) as u8),
                abi::compare_registers(t1, t2),
                abi::branch_ne(range_fail),
            ]);
        }
        instructions.push(store_field(ctype, t1, off));
    }
    Ok(())
}

/// The bit width of a signed narrow C integer, or `None` when the ctype needs no
/// range check (64-bit, unsigned, float, or pointer).
pub(super) fn narrow_signed_bits(ctype: CAbiType) -> Option<u32> {
    match ctype {
        CAbiType::Int8 => Some(8),
        CAbiType::Int16 => Some(16),
        CAbiType::Int32 => Some(32),
        // Exhaustive rather than a `_` arm (plan-113): a new C integer type must
        // decide here whether it needs a range check, not silently inherit "no".
        CAbiType::Int64
        | CAbiType::UInt8
        | CAbiType::UInt16
        | CAbiType::UInt32
        | CAbiType::UInt64
        | CAbiType::Bool
        | CAbiType::Byte
        | CAbiType::Float
        | CAbiType::Double
        | CAbiType::Ptr
        | CAbiType::Str
        | CAbiType::Buffer
        | CAbiType::Void => None,
    }
}

#[allow(clippy::too_many_arguments)]
/// Build the wrapper's result record from a struct slot's post-call buffer
/// (plan-50-E §4.5).
///
/// **Register lifetime is the whole design here.** `_mfb_arena_alloc` destroys
/// every caller-saved register (`x0`-`x17`) with no survivor set, so the record
/// pointer cannot live in one across any later allocation. Two structural rules
/// make that safe:
///
///  1. allocate the record FIRST and spill its pointer to a stack slot;
///  2. read each field from the `sp`-relative struct buffer, which survives every
///     call by construction, and reload the record pointer per field.
///
/// Reading fields into registers and *then* allocating would lose them all — the
/// `copy-record-register-aliasing` bug in miniature.
pub(super) fn marshal_struct_out(
    function: &IrLinkFunction,
    decl: &crate::ir::IrCStruct,
    layout: &crate::ir::CLayout,
    buf_off: usize,
    record_fields: &HashMap<
        crate::types::ParameterType,
        Vec<(String, crate::types::ParameterType)>,
    >,
    symbol: &str,
    cstr_area: usize,
    cursor_off: usize,
    total_off: usize,
    alloc_fail: &str,
    encoding_fail: &str,
    nan_fail: &str,
    inf_fail: &str,
    scratch: &ThunkScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let ThunkScratch {
        t0,
        t1,
        t2,
        t3,
        t4,
        t5,
        t6,
        ..
    } = scratch;
    let record = record_fields.get(&decl.maps_to).ok_or_else(|| {
        format!(
            "LINK function '{}.{}' returns CSTRUCT '{}', whose record '{}' has no field layout",
            function.alias, function.name, decl.name, decl.maps_to
        )
    })?;

    // A record slot is one word at `8*i`, but a `String` field's word is
    // NOT a pointer — it is the offset, relative to the record's own block, of an
    // inlined `{len, bytes, NUL}` sub-block in a trailing data region
    // (`emit_build_inlined_record`). The caller walks that region contiguously to
    // size and copy the record, so the region must exist and be exact.
    //
    // That forces the shape of this routine: every length must be known BEFORE
    // the single allocation, so pass 0 measures, pass 1 sizes and allocates, and
    // pass 2 writes. There is no per-field allocation.
    const REC_OFF: usize = RESULT_SAVE_OFF;
    const ALIGN8_MASK: &str = "18446744073709551608"; // !7u64

    // Resolve each record field to its CSTRUCT field once.
    let mut plan: Vec<(usize, CAbiType, usize)> = Vec::new(); // (rec_idx, ctype, buf offset)
    for (rec_idx, (rname, _)) in record.iter().enumerate() {
        let Some(cpos) = decl.fields.iter().position(|f| f.name == *rname) else {
            return Err(format!(
                "LINK function '{}.{}': record '{}' field '{rname}' has no CSTRUCT field",
                function.alias, function.name, decl.maps_to
            ));
        };
        // `layout` is `decl`'s (`compute_c_layout`, which refuses a field with no
        // C ABI type), so every field has its type there.
        plan.push((rec_idx, layout.ctypes[cpos], buf_off + layout.offsets[cpos]));
    }

    // Pass 0: measure. For each CString field stash [char*, len] and validate the
    // bytes as UTF-8 (§12.4). A NULL char* becomes the empty String, len 0.
    let mut cstr_index: HashMap<usize, usize> = HashMap::new();
    for (rec_idx, ctype, off) in &plan {
        if *ctype != CAbiType::Str {
            continue;
        }
        let k = cstr_index.len();
        cstr_index.insert(*rec_idx, k);
        let ptr_slot = cstr_area + k * 16;
        let len_slot = ptr_slot + 8;
        let null_label = format!("{symbol}_sf{rec_idx}_null");
        let len_loop = format!("{symbol}_sf{rec_idx}_len");
        let len_done = format!("{symbol}_sf{rec_idx}_len_done");
        let measured = format!("{symbol}_sf{rec_idx}_measured");
        instructions.extend([
            abi::load_u64(t0, abi::stack_pointer(), *off),
            abi::store_u64(t0, abi::stack_pointer(), ptr_slot),
            abi::move_immediate(t1, "Integer", "0"),
            abi::store_u64(t1, abi::stack_pointer(), len_slot),
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
            abi::store_u64(t1, abi::stack_pointer(), len_slot),
            // Validate before anything is copied into the record.
            abi::load_u64(abi::return_register(), abi::stack_pointer(), ptr_slot),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), len_slot),
        ]);
        emit_call_validate_utf8(symbol, encoding_fail, instructions, relocations);
        instructions.extend([
            abi::branch(&measured),
            abi::label(&null_label),
            abi::label(&measured),
        ]);
    }

    // Pass 1: size = 8*n, then each inlined block (8-aligned, `len + 9` bytes).
    let fixed = 8 * record.len();
    instructions.extend([
        abi::move_immediate(t0, "Integer", &fixed.to_string()),
        abi::store_u64(t0, abi::stack_pointer(), total_off),
    ]);
    for (rec_idx, ctype, _) in &plan {
        if *ctype != CAbiType::Str {
            continue;
        }
        let len_slot = cstr_area + cstr_index[rec_idx] * 16 + 8;
        instructions.extend([
            // total = align8(total)
            abi::load_u64(t0, abi::stack_pointer(), total_off),
            abi::add_immediate(t0, t0, 7),
            abi::move_immediate(t1, "Integer", ALIGN8_MASK),
            abi::and_registers(t0, t0, t1),
            // total += len + 9
            abi::load_u64(t2, abi::stack_pointer(), len_slot),
            abi::add_immediate(t2, t2, 9),
            abi::add_registers(t0, t0, t2),
            abi::store_u64(t0, abi::stack_pointer(), total_off),
        ]);
    }
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), total_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        REC_OFF,
    ));

    // Pass 2: write each field. The record pointer is reloaded per field — it did
    // not survive the allocation (`_mfb_arena_alloc` destroys x0-x17).
    instructions.extend([
        abi::move_immediate(t0, "Integer", &fixed.to_string()),
        abi::store_u64(t0, abi::stack_pointer(), cursor_off),
    ]);
    for (rec_idx, ctype, off) in &plan {
        if *ctype == CAbiType::Str {
            let k = cstr_index[rec_idx];
            let ptr_slot = cstr_area + k * 16;
            let len_slot = ptr_slot + 8;
            let copy_loop = format!("{symbol}_sf{rec_idx}_copy");
            let copy_done = format!("{symbol}_sf{rec_idx}_copy_done");
            instructions.extend([
                // cursor = align8(cursor)
                abi::load_u64(t0, abi::stack_pointer(), cursor_off),
                abi::add_immediate(t0, t0, 7),
                abi::move_immediate(t1, "Integer", ALIGN8_MASK),
                abi::and_registers(t0, t0, t1),
                abi::store_u64(t0, abi::stack_pointer(), cursor_off),
                // record[8*i] = cursor (the block-relative offset)
                abi::load_u64(t1, abi::stack_pointer(), REC_OFF),
                abi::store_u64(t0, t1, 8 * rec_idx),
                // dst = record + cursor; [dst] = len
                abi::add_registers(t2, t1, t0),
                abi::load_u64(t1, abi::stack_pointer(), len_slot),
                abi::store_u64(t1, t2, 0),
                // copy `len` bytes to dst+8, then NUL-terminate.
                abi::load_u64(t3, abi::stack_pointer(), ptr_slot),
                abi::add_immediate(t4, t2, 8),
                abi::move_immediate(t5, "Integer", "0"),
                abi::label(&copy_loop),
                abi::compare_registers(t5, t1),
                abi::branch_eq(&copy_done),
                abi::load_u8(t6, t3, 0),
                abi::store_u8(t6, t4, 0),
                abi::add_immediate(t3, t3, 1),
                abi::add_immediate(t4, t4, 1),
                abi::add_immediate(t5, t5, 1),
                abi::branch(&copy_loop),
                abi::label(&copy_done),
                abi::store_u8(abi::ZERO, t4, 0),
                // cursor += len + 9
                abi::load_u64(t0, abi::stack_pointer(), cursor_off),
                abi::load_u64(t1, abi::stack_pointer(), len_slot),
                abi::add_immediate(t1, t1, 9),
                abi::add_registers(t0, t0, t1),
                abi::store_u64(t0, abi::stack_pointer(), cursor_off),
            ]);
            continue;
        }
        instructions.push(load_field(*ctype, t0, *off));
        match *ctype {
            // Every load zero-extends, so a signed narrow field must be
            // sign-extended or -1 surfaces as its unsigned reading (bug-238).
            CAbiType::Int8 | CAbiType::Int16 | CAbiType::Int32 => {
                let bits = narrow_signed_bits(*ctype).unwrap();
                instructions.extend([
                    abi::shift_left_immediate(t0, t0, (64 - bits) as u8),
                    abi::arithmetic_shift_right_immediate(t0, t0, (64 - bits) as u8),
                ]);
            }
            CAbiType::Bool => {
                let set = format!("{symbol}_sf{rec_idx}_true");
                let end = format!("{symbol}_sf{rec_idx}_end");
                instructions.extend([
                    abi::compare_immediate(t0, "0"),
                    abi::branch_ne(&set),
                    abi::move_immediate(t0, "Integer", "0"),
                    abi::branch(&end),
                    abi::label(&set),
                    abi::move_immediate(t0, "Integer", "1"),
                    abi::label(&end),
                ]);
            }
            CAbiType::Double | CAbiType::Float => {
                // An MFBASIC Float is always finite (§3), so reject NaN/Inf at the
                // boundary exactly as the CDouble return path does. bug-788: a
                // `CFloat` field was loaded as 4 bytes; widen it first.
                if *ctype == CAbiType::Float {
                    emit_widen_from_cfloat(t0, t1, instructions);
                }
                let finite = format!("{symbol}_sf{rec_idx}_finite");
                instructions.extend([
                    abi::move_immediate(t1, "Integer", F64_POSITIVE_INF_BITS),
                    abi::and_registers(t2, t0, t1),
                    abi::compare_registers(t2, t1),
                    abi::branch_ne(&finite),
                    abi::move_immediate(t3, "Integer", F64_MANTISSA_MASK),
                    abi::and_registers(t4, t0, t3),
                    abi::compare_immediate(t4, "0"),
                    abi::branch_eq(inf_fail),
                    abi::branch(nan_fail),
                    abi::label(&finite),
                ]);
            }
            // No post-load normalization. The unsigned narrows and `CByte` are
            // already correct zero-extended; `CInt64`/`CUInt64`/`CPtr` fill the
            // register; `CString` was handled by the `continue` above; and
            // `CVoid`/`CBuffer` can never be struct fields (`ctype_size_align`
            // answers `None`, so `check_cstruct` rejects them). Enumerated
            // rather than `_ => {}` (plan-113): this is the exact match where a
            // silently-defaulted narrow signed field WAS bug-238.
            CAbiType::UInt8
            | CAbiType::UInt16
            | CAbiType::UInt32
            | CAbiType::UInt64
            | CAbiType::Int64
            | CAbiType::Byte
            | CAbiType::Ptr
            | CAbiType::Str
            | CAbiType::Void
            | CAbiType::Buffer => {}
        }
        instructions.extend([
            abi::load_u64(t1, abi::stack_pointer(), REC_OFF),
            abi::store_u64(t0, t1, 8 * rec_idx),
        ]);
    }
    instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        REC_OFF,
    ));
    Ok(())
}
