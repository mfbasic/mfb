/// The neutral instruction-emission ABI (register roles, instruction builders).
/// Hoisted out of `arch::aarch64` (plan-34-B Phase 2) so shared lowering does not
/// depend on a specific ISA module; `arch::aarch64` re-exports it for its own
/// internal callers.
pub(crate) mod abi;
pub(crate) mod code;
pub(crate) mod lower;
/// The ISA-neutral `RegisterModel` trait + `RegClass`, hoisted out of
/// `arch::aarch64::regmodel` (plan-34-B Phase 2); each backend implements it.
pub(crate) mod regmodel;
pub(crate) mod nir;
pub(crate) mod plan;
pub(crate) mod runtime;
pub(crate) mod validate;
