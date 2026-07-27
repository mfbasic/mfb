//! The AArch64 code-generation backend: the per-ISA tail that consumes neutral
//! MIR (`mir::Backend`). It selects MIR into AArch64 machine ops via
//! [`select_aarch64`] and supplies [`Aarch64RegisterModel`] for the shared
//! allocator. Both AArch64 platforms (macOS and Linux) return [`AARCH64_BACKEND`]
//! from their `CodegenPlatform::backend`, so the shared lowering dispatches
//! selection + allocation through it instead of naming AArch64 directly.

use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
use crate::arch::aarch64::select::select_aarch64;
use crate::target::shared::code::mir::{Backend, MirInstruction};
use crate::target::shared::code::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

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
        select_aarch64(neutral)
    }

    fn register_model(&self) -> &'static dyn RegisterModel {
        &AARCH64_MODEL
    }

    fn is_aarch64(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::regmodel::RegClass;

    #[test]
    fn backend_identifies_as_aarch64() {
        assert!(AARCH64_BACKEND.is_aarch64());
    }

    #[test]
    fn backend_register_model_is_aarch64_model() {
        // Call one RegisterModel method on the returned model to exercise the
        // `register_model` body and prove it hands back the AArch64 model.
        let model = AARCH64_BACKEND.register_model();
        // x0 is the AArch64 arena base register; the model must classify it.
        assert!(model.class_of(model.arena_base()).is_some());
        // The integer class must be non-empty for a real AArch64 model.
        assert!(!model.allocatable(RegClass::Int).is_empty());
    }

    #[test]
    fn backend_select_empty_is_empty() {
        // `select_aarch64` loops over the input, so an empty slice yields an
        // empty Vec (no panic) — exercises the `select` body.
        assert!(AARCH64_BACKEND.select(&[]).is_empty());
    }
}
