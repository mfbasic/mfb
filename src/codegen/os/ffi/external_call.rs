//! Split from `the retired flat native_helpers.rs` (category `os.ffi`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::types::*;
use crate::target::shared::abi;
use std::collections::HashMap;
/// Emit an external (IAT/libc) call whose integer arguments beyond the target's
/// external register file are spilled to the caller's outgoing stack tail
/// (bug-384).
///
/// The caller stages args `0..int_args` in the usual ABI `ARG` roles
/// (`return_register`/`ARG[1]`.. — a `move`/`load`/`add` into each `ARG[n]`).
/// This helper then spills every arg at an index at or beyond
/// `external_int_argument_registers()` to the reserved outgoing-args area via the
/// `OUTGOING_ARGS_BASE` sentinel, which `finalize_frame` sizes and resolves — no
/// manual `sub_sp` bracket, so the spills stay at DEPTH 0 and never collide with
/// the enclosing frame's `[sp+off]` locals. On Win64 the sentinel resolves above
/// the 32-byte shadow (arg 5 at `[rsp+0x20]`); on SysV/AAPCS/riscv it resolves at
/// the frame bottom.
///
/// The point of routing through the register model rather than hardcoding a
/// count is that it is correct on every target by construction: SysV passes 6
/// integer args in registers and AAPCS64/riscv64 pass 8, so for any call within
/// those limits the spill loop is empty and the emitted bytes are byte-identical
/// to a bare `emit_libc_call`. Only Win64 (4 register args) actually spills, and
/// only for a call that passes more than four — exactly the sites bug-384
/// describes. Args 0..4 stay in `rcx/rdx/r8/r9` on Win64 regardless.
pub(crate) fn emit_external_int_call(
    platform: &dyn CodegenPlatform,
    symbol: &str,
    from: &str,
    int_args: usize,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let register_args = platform
        .backend()
        .register_model()
        .external_int_argument_registers();
    for n in register_args..int_args {
        instructions.push(abi::outgoing_stack_arg_store(
            abi::c_arg(n),
            n - register_args,
        ));
    }
    platform.emit_libc_call(symbol, from, platform_imports, instructions, relocations)
}
