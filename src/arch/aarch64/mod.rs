/// `abi` was hoisted to `crate::target::shared::abi` (plan-34-B Phase 2); re-export
/// it here so AArch64-internal callers (`select`, `regmodel`, `encode`) keep using
/// `super::abi` / `crate::arch::aarch64::abi` unchanged.
pub(crate) use crate::target::shared::abi;
pub(crate) mod backend;
pub(crate) mod encode;
pub(crate) mod regmodel;
pub(crate) mod reloc;
pub(crate) mod select;
