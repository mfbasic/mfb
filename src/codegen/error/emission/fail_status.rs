//! `_mfb_rt_fail_status` — a native runtime helper's error with an OS status in
//! its message (plan-158-A).
//!
//! A fixed runtime helper raises through [`raise_error_into`]: a code immediate
//! and the address of the error's one static message. That cannot say *why* an OS
//! call failed, so every failed WASAPI call reached the developer as the same
//! `ErrAudioDevice` sentence with the HRESULT thrown away.
//!
//! This helper builds `"<registered message> (<call>: 0x<8 hex digits>)"` in the
//! arena and raises it the way `FAIL` of an `Error` value does: as ONE owned
//! `Error` block parked in the per-thread current-error slot, tagged
//! `RESULT_ERR_BLOCK_TAG`, for the catcher to adopt. The message text is copied
//! into the block by the park, so the helper frees its temporary before
//! returning, and the loose message register is re-pointed at the block's own
//! inline copy. No new ownership rule: a parked block is exactly what a user
//! `FAIL` produces.
//!
//! The block carries no source (`0`): a runtime helper has no MFB-level origin.
//! The call site re-stamps it with its own location
//! (`CodeBuilder::emit_restamp_parked_helper_error`), just as it stamps a loose
//! helper error today.
//!
//! OOM is never worse than today's static error. If the message cannot be
//! allocated, the helper returns the loose error with the static registered
//! message the call site passed in. If the park itself cannot allocate the block,
//! it returns `ErrOutOfMemory` loose, the same answer every other park gives.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirFunction;
use crate::types::ParameterType;
use std::collections::HashMap;

/// `"0x" + 8 hex digits + ")"`: the bytes appended after the prefix.
const STATUS_SUFFIX_BYTES: usize = 11;
/// `mfb.string.v1` overhead: the `u64` byte length and the trailing NUL.
const STRING_OVERHEAD: usize = 9;

/// Synthesize `_mfb_rt_fail_status` (see the module doc and
/// [`FAIL_STATUS_SYMBOL`] for the register contract).
pub(crate) fn lower_fail_status_helper() -> CodeFunction {
    let symbol = FAIL_STATUS_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");

    let mut vregs = Vregs::new();
    let prefix = &vregs.next();
    let status = &vregs.next();
    let code = &vregs.next();
    let fallback = &vregs.next();
    let length = &vregs.next();
    let size = &vregs.next();
    let block = &vregs.next();
    let src = &vregs.next();
    let dst = &vregs.next();
    let count = &vregs.next();
    let byte = &vregs.next();
    let mask = &vregs.next();
    let tag = &vregs.next();
    let base = &vregs.next();

    let copy_loop = l("copy");
    let copied = l("copied");
    let format_oom = l("format_oom");
    let park_oom = l("park_oom");
    let done = l("done");

    let mut relocations = vec![
        internal_branch(symbol, ARENA_ALLOC_SYMBOL),
        internal_branch(symbol, PARK_ERROR_SYMBOL),
        internal_branch(symbol, ARENA_FREE_SYMBOL),
    ];
    let mut ins: Vec<CodeInstruction> = vec![
        abi::label("entry"),
        abi::move_register(prefix, abi::c_arg(0)),
        abi::move_register(status, abi::c_arg(1)),
        abi::move_register(code, abi::c_arg(2)),
        abi::move_register(fallback, abi::c_arg(3)),
        // The message is the prefix plus `0x########)`.
        abi::load_u64(length, prefix, 0),
        abi::add_immediate(length, length, STATUS_SUFFIX_BYTES),
        abi::add_immediate(size, length, STRING_OVERHEAD),
        abi::move_register(abi::c_arg(0), size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&format_oom),
        abi::move_register(block, RESULT_VALUE_REGISTER),
        abi::store_u64(length, block, 0),
        abi::add_immediate(src, prefix, 8),
        abi::add_immediate(dst, block, 8),
        abi::load_u64(count, prefix, 0),
        abi::label(&copy_loop),
        abi::compare_immediate(count, "0"),
        abi::branch_eq(&copied),
        abi::load_u8(byte, src, 0),
        abi::store_u8(byte, dst, 0),
        abi::add_immediate(src, src, 1),
        abi::add_immediate(dst, dst, 1),
        abi::subtract_immediate(count, count, 1),
        abi::branch(&copy_loop),
        abi::label(&copied),
        abi::move_immediate(byte, "Integer", &u64::from(b'0').to_string()),
        abi::store_u8(byte, dst, 0),
        abi::move_immediate(byte, "Integer", &u64::from(b'x').to_string()),
        abi::store_u8(byte, dst, 1),
        abi::move_immediate(mask, "Integer", "15"),
    ];
    // Eight nibbles of the low 32 bits, most significant first. The status is
    // whatever the backend's C call left, sign-extended or not; only bits 0..31 are
    // read, so an HRESULT prints as its documented 8-digit form.
    for digit in 0..8u8 {
        let decimal = l(&format!("digit{digit}"));
        ins.extend([
            abi::shift_right_immediate(byte, status, 28 - 4 * digit),
            abi::and_registers(byte, byte, mask),
            abi::compare_immediate(byte, "10"),
            abi::branch_lt(&decimal),
            // 'a' - '0' - 10: lands a nibble of 10..15 on 'a'..'f' after the `+ '0'`.
            abi::add_immediate(byte, byte, usize::from(b'a' - b'0' - 10)),
            abi::label(&decimal),
            abi::add_immediate(byte, byte, usize::from(b'0')),
            abi::store_u8(byte, dst, 2 + usize::from(digit)),
        ]);
    }
    ins.extend([
        abi::move_immediate(byte, "Integer", &u64::from(b')').to_string()),
        abi::store_u8(byte, dst, 10),
        abi::store_u8(abi::ZERO, dst, 11),
        // Park it: code, message, no source. The park copies the text into the
        // `Error` block, so the temporary is dead once it returns.
        abi::move_register(RESULT_VALUE_REGISTER, code),
        abi::move_register(RESULT_ERROR_MESSAGE_REGISTER, block),
        abi::move_immediate(RESULT_ERROR_SOURCE_REGISTER, "Integer", "0"),
        abi::branch_link(PARK_ERROR_SYMBOL),
        abi::move_register(tag, RESULT_TAG_REGISTER),
        abi::move_register(abi::c_arg(0), block),
        abi::move_register(abi::c_arg(1), size),
        abi::branch_link(ARENA_FREE_SYMBOL),
        abi::compare_immediate(tag, RESULT_ERR_BLOCK_TAG),
        abi::branch_ne(&park_oom),
        // The loose message register must stay valid (the uncaught-error banner
        // reads it), so point it at the parked block's inline copy:
        // `base + messageOffset`.
        abi::move_immediate(base, "Integer", &ARENA_CURRENT_ERROR_OFFSET.to_string()),
        abi::add_registers(base, ARENA_STATE_REGISTER, base),
        abi::load_u64(base, base, 0),
        abi::load_u64(src, base, 8),
        abi::add_registers(RESULT_ERROR_MESSAGE_REGISTER, base, src),
        abi::move_register(RESULT_VALUE_REGISTER, code),
        abi::move_immediate(RESULT_ERROR_SOURCE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_BLOCK_TAG),
        abi::branch(&done),
        // The park could not allocate the block. Its own registers were the loose
        // `ErrOutOfMemory`, but the free above clobbered them.
        abi::label(&park_oom),
    ]);
    raise_error_into(symbol, "ErrOutOfMemory", &mut ins, &mut relocations);
    ins.extend([
        abi::branch(&done),
        // No room for the message: the error the site would have raised before
        // plan-158, with its static message.
        abi::label(&format_oom),
        abi::move_register(RESULT_VALUE_REGISTER, code),
        abi::move_register(RESULT_ERROR_MESSAGE_REGISTER, fallback),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    finalize_vreg_helper("runtime.failStatus", symbol, "Nothing", ins, relocations)
}

/// Synthesize `_mfb_rt_restamp_parked_error`: the call-site re-stamp of a parked
/// helper error ([`CodeBuilder::emit_restamp_parked_error_body`]). One function per
/// module, like the park it calls: its body is the same at every site, and it is
/// the part (the old block's free, sized from the `Error` layout) that would
/// otherwise be inlined at each one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_restamp_parked_error_helper<'a>(
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, ParameterType>,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &'a HashMap<String, GlobalValue>,
    string_symbols: &'a HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let mut builder = CodeBuilder::for_synthetic_function(
        RESTAMP_PARKED_ERROR_SYMBOL,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    );
    builder.emit_restamp_parked_error_body()?;
    builder.emit(abi::return_());

    // The tail every synthesized error helper shares (`lower_park_error_helper`).
    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
    let is_x86 = crate::codegen::engine::mir::active_backend()
        .register_model()
        .arena_base()
        == crate::arch::x86_64::regmodel::ARENA_BASE_REGISTER;
    crate::optimizer::opt2::peephole::forward_stores_to_loads(&mut instructions, is_x86);
    crate::optimizer::opt2::peephole::remove_fp_shuttles(
        &mut instructions,
        crate::codegen::engine::mir::active_backend().register_model(),
    );
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );
    Ok(CodeFunction {
        name: "runtime.restampParkedError".to_string(),
        symbol: RESTAMP_PARKED_ERROR_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::mir;

    fn call_target(instruction: &CodeInstruction) -> Option<String> {
        (instruction.op == CodeOp::BranchLink)
            .then(|| instruction.get("target").unwrap_or_default().to_string())
    }

    /// plan-158-A: the helper parks exactly once and frees its temporary exactly
    /// once, after the park has copied it. A second park would overwrite the slot
    /// and leak the first block; a free before the park would park freed text.
    #[test]
    fn fail_status_helper_parks_once_then_frees_its_temporary() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let function = lower_fail_status_helper();
        let calls: Vec<String> = function
            .instructions
            .iter()
            .filter_map(call_target)
            .collect();
        assert_eq!(
            calls,
            vec![
                ARENA_ALLOC_SYMBOL.to_string(),
                PARK_ERROR_SYMBOL.to_string(),
                ARENA_FREE_SYMBOL.to_string(),
            ],
            "{calls:?}"
        );
        for callee in [ARENA_ALLOC_SYMBOL, PARK_ERROR_SYMBOL, ARENA_FREE_SYMBOL] {
            assert!(
                function
                    .relocations
                    .iter()
                    .any(|relocation| relocation.to == callee && relocation.binding == "internal"),
                "missing internal relocation to {callee}"
            );
        }
    }
}
