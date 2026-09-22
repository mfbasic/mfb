//! A COM vtable method call — the one shape shared by WASAPI audio and DirectWrite
//! system-font enumeration (plan-148-D).

use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::abi;

/// Call method `slot` of the COM object whose pointer is spilled at stack slot
/// `this_slot`: `this` into the first argument, `method = [[this] + slot * 8]`, call it,
/// and sign-extend its 32-bit result (an `HRESULT`, `UINT32` or `BOOL`) from the C-return
/// register into `result`.
///
/// Every other argument — register and outgoing stack tail — must already be staged.
/// `method` is a scratch register the caller owns; vregs are never allocated to
/// argument registers, so loading it after the arguments are staged disturbs none.
///
/// The result is read from `c_return(0)` because a call through a register is not
/// staged into the aligned MFB bank on Win64 (`rax` is the C result; the aligned bank
/// starts at `rcx`, which still holds `this`). See bug-452.
pub(crate) fn emit_com_call(
    instructions: &mut Vec<CodeInstruction>,
    this_slot: usize,
    slot: usize,
    method: impl Into<Operand>,
    result: impl Into<Operand>,
) {
    let method = method.into();
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), this_slot),
        abi::load_u64(method.clone(), abi::stack_pointer(), this_slot),
        abi::load_u64(method.clone(), method.clone(), 0),
        abi::load_u64(method.clone(), method.clone(), slot * 8),
        abi::branch_link_register(method),
        abi::sign_extend_word(result, abi::c_return(0)),
    ]);
}
