//! The x86-64 code-generation backend (`mir::Backend`) — plan-00-H. Selects
//! neutral MIR into x86-64 machine ops via `mir::select_x86` and supplies the
//! SysV [`X86_64RegisterModel`] for the shared allocator. The `linux-x86_64`
//! platform returns [`X86_64_BACKEND`] from `CodegenPlatform::backend`, so the
//! shared lowering dispatches selection + allocation through it with no edit to
//! the AArch64 backend or the shared selection sites.

use crate::arch::aarch64::regmodel::RegisterModel;
use crate::arch::x86_64::regmodel::X86_64RegisterModel;
use crate::arch::x86_64::select::select_x86;
use crate::target::shared::code::mir::{Backend, MirInstruction};
use crate::target::shared::code::CodeInstruction;

static X86_64_MODEL: X86_64RegisterModel = X86_64RegisterModel;

/// The x86-64 backend singleton (zero-sized).
pub(crate) struct X86_64Backend;

/// The process-wide x86-64 backend instance the platform hands to the shared
/// lowering.
pub(crate) static X86_64_BACKEND: X86_64Backend = X86_64Backend;

impl Backend for X86_64Backend {
    fn select(&self, neutral: &[MirInstruction]) -> Vec<CodeInstruction> {
        select_x86(neutral)
    }

    fn register_model(&self) -> &'static dyn RegisterModel {
        &X86_64_MODEL
    }
}
