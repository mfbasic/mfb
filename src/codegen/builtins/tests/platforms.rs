//! The real code-generation platform of each backend, for tests that drive one emitter.
//!
//! `TestPlatform` stubs the OS seam with `unimplemented!()`, so an emitter that reaches
//! the OS has to be driven against a real backend: its `emit_external_call` is what
//! consults the import list, which is the thing the import-refusal tests are about.

use crate::codegen::engine::types::CodegenPlatform;
use crate::os::linux::flavor::LinuxFlavor::Glibc;
use crate::target::linux_common::code::Platform as Linux;
use crate::testutil::CodeTarget;

/// The platform `target`'s `lower_module` would hand its helpers.
pub(super) fn platform_for(target: CodeTarget) -> Box<dyn CodegenPlatform> {
    match target {
        CodeTarget::MacosAarch64 => Box::new(crate::target::macos_aarch64::code::Platform),
        CodeTarget::WindowsX86_64 => Box::new(crate::target::win_x86_64::code::Platform),
        CodeTarget::LinuxAarch64 => Box::new(Linux::for_test(
            crate::target::linux_aarch64::code::Aarch64,
            Glibc,
        )),
        CodeTarget::LinuxX86_64 => Box::new(Linux::for_test(
            crate::target::linux_x86_64::code::X86_64,
            Glibc,
        )),
        CodeTarget::LinuxRiscv64 => Box::new(Linux::for_test(
            crate::target::linux_riscv64::code::Riscv64,
            Glibc,
        )),
    }
}
