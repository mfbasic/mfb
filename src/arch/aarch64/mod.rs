/// `abi` was hoisted to `crate::target::shared::abi` (plan-34-B Phase 2); re-export
/// it here so AArch64-internal callers (`select`, `regmodel`, `encode`) keep using
/// `super::abi` / `crate::arch::aarch64::abi` unchanged.
pub(crate) use crate::target::shared::abi;
pub(crate) mod backend;
// The decoder serves `mfb dbg` reading AArch64 code: on every host but a
// riscv64 Linux one, which reads riscv64 code (tests run it everywhere).
#[cfg(any(not(all(target_os = "linux", target_arch = "riscv64")), test))]
pub(crate) mod decode;
pub(crate) mod encode;
pub(crate) mod regmodel;
pub(crate) mod reloc;
pub(crate) mod select;
