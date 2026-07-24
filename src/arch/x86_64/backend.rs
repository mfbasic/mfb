//! The x86-64 code-generation backend (`mir::Backend`) — plan-00-H. Selects
//! neutral MIR into x86-64 machine ops via `mir::select_x86` and supplies the
//! SysV [`X86_64RegisterModel`] for the shared allocator. The `linux-x86_64`
//! platform returns [`X86_64_BACKEND`] from `CodegenPlatform::backend`, so the
//! shared lowering dispatches selection + allocation through it with no edit to
//! the AArch64 backend or the shared selection sites.

use crate::arch::aarch64::regmodel::RegisterModel;
use crate::arch::x86_64::regmodel::{Win64RegisterModel, X86_64RegisterModel};
use crate::arch::x86_64::select::{select_x86, X86Abi};
use crate::target::shared::code::mir::{Backend, MirInstruction};
use crate::target::shared::code::CodeInstruction;

static X86_64_MODEL: X86_64RegisterModel = X86_64RegisterModel;

// The Win64 backend and its model land tested but unwired: the `win_x86_64`
// `CodegenPlatform` (plan-47-B A2, the stub wall) is their only production
// consumer, via `backend()`. Per 47-B Phase 3 they land "behind unit tests
// alone", so the allows below are removed when A2 wires `WIN64_BACKEND`.
#[allow(dead_code)]
static WIN64_MODEL: Win64RegisterModel = Win64RegisterModel;

/// The 32-byte Win64 shadow/home space, reserved below the first stack argument
/// (plan-47-B §4.3). Returned by both `shadow_space_bytes` and
/// `outgoing_args_base_offset` so the shared `finalize_frame` reserves it and
/// places outgoing arg 0 above it.
#[allow(dead_code)]
const WIN64_SHADOW_SPACE: usize = 32;

/// The x86-64 backend singleton (zero-sized).
pub(crate) struct X86_64Backend;

/// The process-wide x86-64 backend instance the platform hands to the shared
/// lowering.
pub(crate) static X86_64_BACKEND: X86_64Backend = X86_64Backend;

impl Backend for X86_64Backend {
    fn select(&self, neutral: &[MirInstruction]) -> Vec<CodeInstruction> {
        select_x86(neutral, X86Abi::SysV)
    }

    fn register_model(&self) -> &'static dyn RegisterModel {
        &X86_64_MODEL
    }

    fn frame_call_padding(&self) -> usize {
        // `call` pushes the 8-byte return address; the frame absorbs it so rsp is
        // 16-byte aligned at this function's own calls (SysV — libc `movaps`).
        8
    }
}

/// The Win64 x86-64 backend singleton (plan-47-B): the same ISA selection as
/// [`X86_64Backend`] but realized against the Win64 calling convention
/// ([`X86Abi::Win64`]) and register model, with the 32-byte shadow space wired
/// through the shared frame finalizer. The `win_x86_64` platform hands this to
/// the shared lowering.
#[allow(dead_code)] // wired by the win_x86_64 CodegenPlatform (47-B A2)
pub(crate) struct Win64Backend;

/// The process-wide Win64 backend instance.
#[allow(dead_code)] // wired by the win_x86_64 CodegenPlatform (47-B A2)
pub(crate) static WIN64_BACKEND: Win64Backend = Win64Backend;

impl Backend for Win64Backend {
    fn select(&self, neutral: &[MirInstruction]) -> Vec<CodeInstruction> {
        select_x86(neutral, X86Abi::Win64)
    }

    fn register_model(&self) -> &'static dyn RegisterModel {
        &WIN64_MODEL
    }

    fn frame_call_padding(&self) -> usize {
        // Identical to SysV: `call` pushes the 8-byte return address and Win64
        // also requires rsp 16-aligned at the call site (§4.3).
        8
    }

    fn shadow_space_bytes(&self) -> usize {
        WIN64_SHADOW_SPACE
    }

    fn outgoing_args_base_offset(&self) -> usize {
        WIN64_SHADOW_SPACE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::code::mir::lower_to_mir;

    #[test]
    fn backend_selects_and_reports_model_and_padding() {
        let backend = X86_64Backend;
        // select() lowers neutral MIR to x86 CodeInstructions.
        let mir = lower_to_mir(&[CodeInstruction::new("ret")]);
        let out = backend.select(&mir);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].op.mnemonic(), "ret");
        // The register model is the SysV x86-64 model (arena_base = r15).
        assert_eq!(backend.register_model().arena_base(), "r15");
        // The frame absorbs the 8-byte return address.
        assert_eq!(backend.frame_call_padding(), 8);
        // SysV has no shadow space / outgoing-args offset.
        assert_eq!(backend.shadow_space_bytes(), 0);
        assert_eq!(backend.outgoing_args_base_offset(), 0);
    }

    #[test]
    fn win64_backend_wires_the_shadow_space_and_win64_model() {
        let backend = Win64Backend;
        // Same ISA selection as SysV.
        let mir = lower_to_mir(&[CodeInstruction::new("ret")]);
        let out = backend.select(&mir);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].op.mnemonic(), "ret");
        // The 32-byte shadow space is wired through both frame seams (plan-47-B §4.3).
        assert_eq!(backend.shadow_space_bytes(), 32);
        assert_eq!(backend.outgoing_args_base_offset(), 32);
        // `call` still pushes the 8-byte return address; rsp stays 16-aligned.
        assert_eq!(backend.frame_call_padding(), 8);
        // The Win64 register model: 4-register external cap, 3 allocatable ints.
        let model = backend.register_model();
        assert_eq!(model.external_int_argument_registers(), 4);
        assert_eq!(model.arena_base(), "r15");
    }
}
