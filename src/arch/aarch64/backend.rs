//! The AArch64 code-generation backend: the per-ISA tail that consumes neutral
//! MIR (`mir::Backend`). It selects MIR into AArch64 machine ops via
//! [`select_aarch64`] and supplies [`Aarch64RegisterModel`] for the shared
//! allocator. Both AArch64 platforms (macOS and Linux) return [`AARCH64_BACKEND`]
//! from their `CodegenPlatform::backend`, so the shared lowering dispatches
//! selection + allocation through it instead of naming AArch64 directly.

use crate::arch::aarch64::regmodel::{Aarch64RegisterModel, RegisterModel};
use crate::target::shared::code::mir::{self, Backend, MirInstruction};
use crate::target::shared::code::CodeInstruction;

/// The AArch64 register model singleton handed to the shared allocator.
static AARCH64_MODEL: Aarch64RegisterModel = Aarch64RegisterModel;

/// The AArch64 backend singleton. Zero-sized; installed as the active backend
/// for the duration of an AArch64 build.
pub(crate) struct Aarch64Backend;

/// The process-wide AArch64 backend instance the platforms hand to the shared
/// lowering.
pub(crate) static AARCH64_BACKEND: Aarch64Backend = Aarch64Backend;

impl Backend for Aarch64Backend {
    fn select(&self, neutral: &[MirInstruction]) -> Vec<CodeInstruction> {
        mir::select_aarch64(neutral)
    }

    fn register_model(&self) -> &'static dyn RegisterModel {
        &AARCH64_MODEL
    }
}
