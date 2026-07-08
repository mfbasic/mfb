//! The RISC-V 64 code-generation backend (`mir::Backend`) — plan-99. Selects
//! neutral MIR into RV64GC machine ops via [`select_riscv64`] and supplies the
//! lp64d [`Riscv64RegisterModel`] for the shared allocator. The `linux-riscv64`
//! platform returns [`RISCV64_BACKEND`] from `CodegenPlatform::backend`, so the
//! shared lowering dispatches selection + allocation through it with no edit to
//! the other backends or the shared selection sites.

use crate::arch::aarch64::regmodel::RegisterModel;
use crate::arch::riscv64::regmodel::Riscv64RegisterModel;
use crate::arch::riscv64::select::select_riscv64;
use crate::target::shared::code::mir::{Backend, MirInstruction};
use crate::target::shared::code::CodeInstruction;

static RISCV64_MODEL: Riscv64RegisterModel = Riscv64RegisterModel;

/// The rv64 backend singleton (zero-sized).
pub(crate) struct Riscv64Backend;

/// The process-wide rv64 backend instance the platform hands to the shared
/// lowering.
pub(crate) static RISCV64_BACKEND: Riscv64Backend = Riscv64Backend;

impl Backend for Riscv64Backend {
    fn select(&self, neutral: &[MirInstruction]) -> Vec<CodeInstruction> {
        select_riscv64(neutral)
    }

    fn register_model(&self) -> &'static dyn RegisterModel {
        &RISCV64_MODEL
    }

    fn frame_call_padding(&self) -> usize {
        // The return address is held in `ra` (a register), not pushed by the
        // call, so a 16-aligned frame keeps `sp` 16-aligned at call sites — no
        // padding needed (like AArch64, unlike x86-64).
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::code::mir::lower_to_mir;

    #[test]
    fn backend_selects_and_reports_model_and_padding() {
        let backend = Riscv64Backend;
        let mir = lower_to_mir(&[CodeInstruction::new("ret")]);
        let out = backend.select(&mir);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].op.mnemonic(), "ret");
        assert_eq!(backend.register_model().arena_base(), "s11");
        assert_eq!(backend.frame_call_padding(), 0);
    }
}
