//! The linux-x86_64 codegen delta (plan-00-H, complete).
//!
//! Everything Linux-invariant lives in [`crate::target::linux_common::code`]
//! (bug-321); what remains here is what the x86-64 ISA and its **raw-syscall
//! policy** force. The arena map/unmap, `write`, `getrandom`, and process exit
//! run via raw Linux x86-64 syscalls (no libc), so an integer-only executable is
//! fully static; the rest of the console runtime surface routes through libc via
//! the shared `emit_libc_call` seam.
//!
//! CodeInstructions are built with the neutral `abi::*` builders and the neutral
//! role tokens (`%sysarg*`/`%sysnr`, plan-34-D); `remap_x86_abi` realizes them to
//! their SysV homes (rdi, rsi, rdx, r10, ...). The x86-64 encoder
//! (`crate::arch::x86_64::encode`) realizes the neutral ops as concrete x86
//! bytes; `svc` encodes to the x86 `syscall` opcode.
//!
//! Every raw-syscall decision here must be matched by the corresponding flag in
//! [`super::plan`]'s `LinuxAbi`, or the plan declares a libc import no code ever
//! references (bug-71, bug-79.4).

use std::collections::HashMap;
use std::path::PathBuf;

use crate::arch::aarch64::abi;
use crate::codegen::engine::mir::MirPlan;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::engine::types::CodeRelocation;
use crate::codegen::engine::types::NativeCodePlan;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::linux_common::code::{
    self as common, AppSupport, LinuxArch, MAP_PRIVATE_ANON, PROT_READ_WRITE,
};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;

// --- Linux x86-64 syscall numbers -----------------------------------------
// x86-64 predates Linux's asm-generic syscall table, so its numbers differ from
// the aarch64/riscv64 ones (mmap 222 / munmap 215 there).
const SYS_WRITE: &str = "1";
const SYS_MMAP: &str = "9";
const SYS_MUNMAP: &str = "11";
const SYS_EXIT_GROUP: &str = "231";
const SYS_GETRANDOM: &str = "318";

pub(crate) fn lower_module(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
) -> Result<NativeCodePlan, String> {
    common::lower_module(module, native_plan, packages, flavor, X86_64)
}

pub(crate) fn lower_module_mir(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
) -> Result<MirPlan, String> {
    common::lower_module_mir(module, native_plan, packages, flavor, X86_64)
}

pub(crate) struct X86_64;

impl LinuxArch for X86_64 {
    fn arch(&self) -> &'static str {
        "x86_64"
    }

    fn target(&self) -> &'static str {
        "linux-x86_64"
    }

    fn musl_libc(&self) -> &'static str {
        "libc.musl-x86_64.so.1"
    }

    fn backend(&self) -> &'static dyn crate::codegen::engine::mir::Backend {
        &crate::arch::x86_64::backend::X86_64_BACKEND
    }

    fn app(&self) -> AppSupport {
        // GTK4 app mode (plan-05-linux-app.md), shared with linux-aarch64 via
        // `target::linux_gtk`. The x86 variants bracket every callback/helper for
        // the SysV callee-saved contract and use the per-ISA entry trampoline.
        AppSupport::Gtk {
            sysv_wrappers: true,
        }
    }

    fn stat_mode_offset(&self) -> usize {
        // Linux x86-64 `struct stat`: st_mode at offset 24 — NOT the 16 that
        // aarch64 and riscv64 use, and it sits amid a run of constants that are
        // identical everywhere (bug-321 finding #4).
        24
    }

    fn environ_got_dereferences(&self) -> usize {
        // On x86-64 the fused `adrp`/`add` pair lowers to a single GOTPCREL `mov`
        // that already loads `&environ` from the GOT slot; one further deref gives
        // the `char**`.
        1
    }

    fn emit_console_program_exit(
        &self,
        _libc: &str,
        _from: &str,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // exit_group(code) — nr 231, no libc. The shared callers place the exit
        // code in the neutral return register `x0`; because this syscall
        // immediately follows, select maps that `x0` to the syscall's first
        // argument (rdi) at the caller's own instruction. So the code is already
        // in rdi — only the syscall number is needed (`x8`→rax). A prior
        // `mov rdi,rax` here wrongly overwrote the code with the leaked variadic
        // `al`=8 (rax) left by the pre-shutdown call.
        instructions.push(abi::move_immediate(
            abi::syscall_register(),
            "Integer",
            SYS_EXIT_GROUP,
        ));
        instructions.push(abi::syscall());
        instructions.push(abi::branch_self());
        // Unreachable, but every function the validator sees needs a return op
        // (callers like the signal handler end with this).
        instructions.push(abi::return_());
        Ok(())
    }

    fn emit_write(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // write(fd, buf, len) — nr 1. The shared callers set fd/buf/len in the
        // AArch64 x0/x1/x2 slots → rdi/rsi/rdx; set the syscall number.
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", SYS_WRITE),
            abi::syscall(),
            // plan-85: the SysV syscall RESULT comes back in `rax` (`%retSys`), but
            // the aligned MFB convention reads a call result in the aligned bank
            // (`return_register()` → `rdi`). Stage the kernel return into the MFB
            // result register so the shared caller's `return_register()` read is
            // correct. (On AArch64/RISC-V both are `x0`, so those backends never
            // reach this x86-only method and need no staging.)
            abi::move_register(abi::return_register(), abi::sys_return()),
        ]);
        Ok(())
    }

    fn emit_random_bytes(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // getrandom(buf, len, flags=0) — nr 318. The caller leaves the buffer ptr
        // in the return register (→ rdi) and the length in x1 (→ rsi); set flags
        // and the syscall number.
        instructions.extend([
            abi::move_immediate(abi::sys_arg(2), "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", SYS_GETRANDOM),
            abi::syscall(),
            // plan-85: stage the kernel return (`rax`) into the aligned MFB result
            // register (`rdi`) — see `emit_write`.
            abi::move_register(abi::return_register(), abi::sys_return()),
        ]);
        Ok(())
    }

    fn emit_arena_map(
        &self,
        size_reg: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) -> Result<(), String> {
        // mmap(0, size, PROT_RW, MAP_PRIVATE|ANON, -1, 0) — nr 9.
        // x86-64 syscall ABI: nr=rax, args rdi,rsi,rdx,r10,r8,r9, ret=rax.
        instructions.extend([
            abi::move_immediate(abi::sys_arg(0), "Integer", "0"),
            abi::move_register(abi::sys_arg(1), size_reg),
            abi::move_immediate(abi::sys_arg(2), "Integer", PROT_READ_WRITE),
            abi::move_immediate(abi::sys_arg(3), "Integer", MAP_PRIVATE_ANON),
            // r8 = -1 (no fd) — immediates parse as u64, so use the bit pattern.
            abi::move_immediate(abi::sys_arg(4), "Integer", &u64::MAX.to_string()),
            abi::move_immediate(abi::sys_arg(5), "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", SYS_MMAP),
            abi::syscall(),
            // plan-85: stage the mmap return (`rax`, the arena base) into the aligned
            // MFB result register (`rdi`) — see `emit_write`.
            abi::move_register(abi::return_register(), abi::sys_return()),
        ]);
        Ok(())
    }

    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        // munmap(addr, len) — nr 11. The shared arena_destroy leaves addr/len in
        // the AArch64 x0/x1 slots, which the x86-64 selection maps to rdi/rsi, so
        // they are already in place; only the syscall number is set here.
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", SYS_MUNMAP),
            abi::syscall(),
        ]);
        Ok(())
    }

    // No `emit_thread_trampoline` override: x86-64 uses the default
    // `LinuxArch::emit_thread_trampoline`, which returns the shared
    // `lower_thread_trampoline` verbatim (like aarch64/riscv64). That shared body
    // now applies the SysV stack realign for ALL x86-64 (bug-408): glibc, musl,
    // and Windows all enter the thread start-routine via a `call` at `sp%16==8`
    // and need one +8 bias (an 88-byte frame). This backend previously inserted a
    // SECOND, unconditional `sub sp,8` here (commit 10b5d0fb4a) to compensate for
    // bug-385 having wrongly gated glibc OUT of the shared realign — which left
    // glibc correct only by accident (80 + 8 = 88) and pushed musl to a broken
    // 96-byte double realign (both box-proven on 2228/2227, 2026-08-01). With the
    // shared gate fixed to be per-arch, that override is removed.
}
