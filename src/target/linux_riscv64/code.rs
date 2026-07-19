//! The linux-riscv64 codegen delta.
//!
//! Everything Linux-invariant lives in [`crate::target::linux_common::code`]
//! (bug-321); what remains here is only what the RISC-V ISA forces — plus the
//! app-mode hard-stop, which is deliberate and load-bearing (see [`Riscv64::app`]).

use std::path::PathBuf;

use crate::os::linux::flavor::LinuxFlavor;
use crate::target::linux_common::code::{
    self as common, emit_asm_generic_arena_map, emit_asm_generic_arena_unmap, AppSupport, LinuxArch,
};
use crate::target::shared::code::{self, CodeInstruction, MirPlan, NativeCodePlan};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;

/// The message every app-mode hard-stop panics with. bug-117.1: app mode was
/// never ported to rv64, and plan-51-A §3.3 records why it is now permanently
/// out rather than pending — AppImage/type2-runtime publishes no riscv64 runtime
/// to seal an AppDir with.
pub(crate) const APP_MODE_UNPORTED: &str =
    "rv64 app mode not ported (plan-05 is aarch64/x86-64 only)";

pub(crate) fn lower_module(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
) -> Result<NativeCodePlan, String> {
    common::lower_module(module, native_plan, packages, flavor, Riscv64)
}

pub(crate) fn lower_module_mir(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
) -> Result<MirPlan, String> {
    common::lower_module_mir(module, native_plan, packages, flavor, Riscv64)
}

pub(crate) struct Riscv64;

impl LinuxArch for Riscv64 {
    fn arch(&self) -> &'static str {
        "riscv64"
    }

    fn target(&self) -> &'static str {
        "linux-riscv64"
    }

    fn musl_libc(&self) -> &'static str {
        "libc.musl-riscv64.so.1"
    }

    fn backend(&self) -> &'static dyn code::mir::Backend {
        &crate::arch::riscv64::backend::RISCV64_BACKEND
    }

    /// **Do not change this to `Gtk` without porting the toolkit.**
    ///
    /// `target::linux_gtk` emits aarch64-convention register names; handing it
    /// to rv64 would produce armed-but-dead wrong-ISA code. Declaring
    /// [`AppSupport::Unsupported`] makes all nine app-mode hooks in
    /// `linux_common::code` hard-stop instead — the innermost of the three
    /// defense layers bug-223 requires. The other two are
    /// [`super::Backend::supports_app_mode`] (false) and the
    /// `NativeBuildMode::Console`-only guard in `super::lower_validated_module`,
    /// which turns a non-CLI caller's app build into a clean `Err` rather than
    /// this panic.
    fn app(&self) -> AppSupport {
        AppSupport::Unsupported(APP_MODE_UNPORTED)
    }

    fn stat_mode_offset(&self) -> usize {
        // Linux riscv64 `struct stat`: st_mode at offset 16. NOT a shared Linux
        // constant — x86-64 puts it at 24 (bug-321 finding #4).
        16
    }

    fn environ_got_dereferences(&self) -> usize {
        // On rv64 the `adrp`/`add` pair lowers to `auipc`/`ld` (GOT), which
        // already loads `&environ` out of the slot; one further deref gives the
        // live `char**`.
        1
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
