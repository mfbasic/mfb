//! The linux-aarch64 codegen delta.
//!
//! Everything Linux-invariant lives in [`crate::target::linux_common::code`]
//! (bug-321); what remains here is only what the AArch64 ISA forces.

use std::path::PathBuf;

use crate::os::linux::flavor::LinuxFlavor;
use crate::target::linux_common::code::{
    self as common, emit_asm_generic_arena_map, emit_asm_generic_arena_unmap, AppSupport, LinuxArch,
};
use crate::target::shared::code::{self, CodeInstruction, MirPlan, NativeCodePlan};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;

pub(crate) fn lower_module(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
) -> Result<NativeCodePlan, String> {
    common::lower_module(module, native_plan, packages, flavor, Aarch64)
}

pub(crate) fn lower_module_mir(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
) -> Result<MirPlan, String> {
    common::lower_module_mir(module, native_plan, packages, flavor, Aarch64)
}

pub(crate) struct Aarch64;

impl LinuxArch for Aarch64 {
    fn arch(&self) -> &'static str {
        "aarch64"
    }

    fn target(&self) -> &'static str {
        "linux-aarch64"
    }

    fn musl_libc(&self) -> &'static str {
        "libc.musl-aarch64.so.1"
    }

    fn backend(&self) -> &'static dyn code::mir::Backend {
        &crate::arch::aarch64::backend::AARCH64_BACKEND
    }

    fn app(&self) -> AppSupport {
        // GTK4 app mode (plan-05-linux-app.md), shared via `target::linux_gtk`.
        // AArch64 is the ISA the toolkit was written against, so its helpers need
        // no calling-convention bracket.
        AppSupport::Gtk {
            sysv_wrappers: false,
        }
    }

    fn stat_mode_offset(&self) -> usize {
        // Linux aarch64 `struct stat`: st_mode at offset 16. NOT a shared Linux
        // constant — x86-64 puts it at 24 (bug-321 finding #4).
        16
    }

    fn environ_got_dereferences(&self) -> usize {
        // `adrp`/`add` (external = GOT) yield the *address of* the GOT slot, so
        // one deref gives `&environ` and a second gives the live `char**`.
        2
    }

    fn emit_arena_map(
        &self,
        size_reg: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) -> Result<(), String> {
        emit_asm_generic_arena_map(size_reg, instructions)
    }

    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        emit_asm_generic_arena_unmap(instructions)
    }
}
