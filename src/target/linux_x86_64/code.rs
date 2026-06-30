//! Linux x86-64 codegen platform (plan-00-H Phase 1).
//!
//! Brings up the integer-only machine floor: the program entry + arena map/unmap
//! + random bytes + exit, all via **raw Linux x86-64 syscalls** (no libc), so the
//! emitted executable is fully static. The runtime-helper surface (io / fs / net /
//! term / ...) is not wired in this phase — those `CodegenPlatform` methods return
//! a `Phase 1: <name> not yet implemented` error. They are unreachable for a
//! program that runs only integer language code.
//!
//! CodeInstructions are built with the neutral `abi::*` builders (the same ones
//! the AArch64 backend uses), but with **x86-64 register names** ("rax", "rdi",
//! ...). The x86-64 encoder (`crate::arch::x86_64::encode`) realizes the neutral
//! ops (`mov_imm`, `sub_sp`, `str_u64`, `bl`, `svc`, `branch_self`, ...) as the
//! concrete x86 instruction bytes. `svc` encodes to the x86 `syscall` opcode.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::arch::aarch64::abi;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::shared::code::{
    self, CodeFrame, CodeFunction, CodeInstruction, CodeRelocation, MirPlan, NativeCodePlan,
    ProgramEntrySpec, RelocIntent,
};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;

pub(crate) fn lower_module(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
) -> Result<NativeCodePlan, String> {
    code::lower_module_for_platform(module, native_plan, packages, &Platform { flavor })
}

pub(crate) fn lower_module_mir(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
) -> Result<MirPlan, String> {
    code::lower_module_mir_for_platform(module, native_plan, packages, &Platform { flavor })
}

struct Platform {
    #[allow(dead_code)]
    flavor: LinuxFlavor,
}

// --- Linux x86-64 syscall numbers -----------------------------------------
const SYS_WRITE: &str = "1";
const SYS_MMAP: &str = "9";
const SYS_MUNMAP: &str = "11";
const SYS_EXIT_GROUP: &str = "231";
const SYS_GETRANDOM: &str = "318";

// `mmap` argument constants.
const PROT_READ_WRITE: &str = "3"; // PROT_READ | PROT_WRITE
const MAP_PRIVATE_ANON: &str = "34"; // MAP_PRIVATE | MAP_ANONYMOUS (0x02 | 0x20)

/// Build `eor dst, lhs, rhs` (no abi helper takes three explicit reg operands in
/// the shape the entry needs for the zero-idiom; the AArch64 `exclusive_or`
/// helper exists but we keep the entry explicit and ISA-local).
fn xor_self(reg: &str) -> CodeInstruction {
    CodeInstruction::new("eor")
        .field("dst", reg)
        .field("lhs", reg)
        .field("rhs", reg)
}

impl code::CodegenPlatform for Platform {
    fn target(&self) -> &'static str {
        "linux-x86_64"
    }

    fn arch(&self) -> &'static str {
        "x86_64"
    }

    fn backend(&self) -> &'static dyn code::mir::Backend {
        &crate::arch::x86_64::backend::X86_64_BACKEND
    }

    // termios layout — Linux values (mirrors linux_aarch64).
    fn termios_size(&self) -> usize {
        60
    }
    fn termios_lflag_offset(&self) -> usize {
        12
    }
    fn termios_lflag_width(&self) -> usize {
        4
    }
    fn termios_cc_offset(&self) -> usize {
        17
    }
    fn termios_echo_flag(&self) -> u64 {
        8
    }
    fn termios_icanon_flag(&self) -> u64 {
        2
    }
    fn termios_vmin_index(&self) -> usize {
        6
    }
    fn termios_vtime_index(&self) -> usize {
        5
    }

    fn emit_program_entry(
        &self,
        spec: &ProgramEntrySpec<'_>,
        _platform_imports: &HashMap<String, String>,
    ) -> Result<CodeFunction, String> {
        // Minimal raw-syscall entry (plan-00-H Phase 1). Establishes the arena
        // base/state invariants the rest of the runtime presumes, calls the
        // language entry, and exits via exit_group. The full Result-tag error
        // reporting / PROGRAM_EXIT path is deferred to a later phase.
        let entry = spec.entry_symbol;
        let mut instructions: Vec<CodeInstruction> = Vec::new();
        let mut relocations: Vec<CodeRelocation> = Vec::new();

        // 1. entry label.
        instructions.push(abi::label("entry"));
        // 2. r14 = 0 (the zero register).
        instructions.push(xor_self("r14"));
        // 3. Reserve the entry-stack arena.
        instructions.push(abi::subtract_stack(spec.entry_stack_size));
        // 4. arena_base (r15) = rsp.
        instructions.push(abi::add_immediate("r15", "rsp", 0));
        // 5. Zero the arena-state header + the free-list head, mirroring the
        //    unconditional stores at the top of `lower_program_entry`.
        for off in [0usize, 8, 16, 24] {
            instructions.push(abi::store_u64("r14", "r15", off));
        }
        instructions.push(abi::store_u64(
            "r14",
            "r15",
            code::ARENA_FREE_LIST_HEAD_OFFSET,
        ));

        // 6. Call the language entry.
        instructions.push(abi::branch_link(spec.language_entry_symbol));
        relocations.push(CodeRelocation {
            from: entry.to_string(),
            to: spec.language_entry_symbol.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });

        // 7-8. Exit. `_mfb_fn_main` returns a Result: tag in rax (x0), value in
        //      rdx (x1 = RESULT_VALUE_REGISTER → the SysV 2nd return register).
        //      For an Integer entry, exit with the value masked to 8 bits;
        //      otherwise exit 0. (Phase 1: the Result tag / error path is not yet
        //      handled — OK programs only.)
        if spec.language_entry_returns == "Integer" {
            // rdx &= 0xff. The encoder has no immediate-form AND, so materialize
            // 255 in rcx and use the register-form `and`.
            instructions.push(abi::move_immediate("rcx", "Integer", "255"));
            instructions.push(
                CodeInstruction::new("and")
                    .field("dst", "rdx")
                    .field("lhs", "rdx")
                    .field("rhs", "rcx"),
            );
            instructions.push(abi::move_register("rdi", "rdx"));
        } else {
            instructions.push(abi::move_immediate("rdi", "Integer", "0"));
        }
        instructions.push(abi::move_immediate("rax", "Integer", SYS_EXIT_GROUP));
        instructions.push(abi::syscall());
        instructions.push(abi::branch_self());
        // Unreachable, but the native-code validator requires a return op.
        instructions.push(abi::return_());

        Ok(CodeFunction {
            name: "program.entry".to_string(),
            symbol: entry.to_string(),
            params: Vec::new(),
            returns: "Nothing".to_string(),
            // The entry manages its own frame (the `sub_sp` above), so the
            // generic frame is empty.
            frame: CodeFrame {
                stack_size: 0,
                callee_saved: Vec::new(),
            },
            stack_slots: Vec::new(),
            instructions,
            relocations,
        })
    }

    fn emit_program_exit(
        &self,
        _from: &str,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // exit_group(rdi = current return-register value). The shared callers
        // leave the exit code in the return register; on x86-64 SysV that is rax.
        instructions.push(abi::move_register("rdi", "rax"));
        instructions.push(abi::move_immediate("rax", "Integer", SYS_EXIT_GROUP));
        instructions.push(abi::syscall());
        instructions.push(abi::branch_self());
        // Unreachable, but every function the validator sees needs a return op
        // (callers like the signal handler end with this).
        instructions.push(abi::return_());
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
            abi::move_immediate("rdi", "Integer", "0"),
            abi::move_register("rsi", size_reg),
            abi::move_immediate("rdx", "Integer", PROT_READ_WRITE),
            abi::move_immediate("r10", "Integer", MAP_PRIVATE_ANON),
            // r8 = -1 (no fd) — immediates parse as u64, so use the bit pattern.
            abi::move_immediate("r8", "Integer", &u64::MAX.to_string()),
            abi::move_immediate("r9", "Integer", "0"),
            abi::move_immediate("rax", "Integer", SYS_MMAP),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        // munmap(addr, len) — nr 11. The shared arena_destroy leaves addr/len in
        // the AArch64 x0/x1 slots, which the x86-64 selection maps to rdi/rsi, so
        // they are already in place; only the syscall number is set here.
        instructions.extend([
            abi::move_immediate("rax", "Integer", SYS_MUNMAP),
            abi::syscall(),
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
            abi::move_immediate("rdx", "Integer", "0"),
            abi::move_immediate("rax", "Integer", SYS_GETRANDOM),
            abi::syscall(),
        ]);
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
            abi::move_immediate("rax", "Integer", SYS_WRITE),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_thread_trampoline(
        &self,
        _platform_imports: &HashMap<String, String>,
    ) -> Result<CodeFunction, String> {
        Err("x86_64 Phase 1: emit_thread_trampoline not yet implemented".into())
    }

    // --- Runtime-helper OS methods (deferred to a later phase) --------------

    fn emit_poll_input(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_poll_input not yet implemented".into())
    }

    fn emit_is_terminal(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_is_terminal not yet implemented".into())
    }

    fn emit_terminal_size(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_terminal_size not yet implemented".into())
    }

    fn emit_path_exists(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_path_exists not yet implemented".into())
    }

    fn emit_path_stat(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_path_stat not yet implemented".into())
    }

    fn stat_mode_offset(&self) -> usize {
        // Linux x86-64 `struct stat`: st_mode lives at offset 24.
        24
    }

    fn emit_current_directory(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_current_directory not yet implemented".into())
    }

    fn emit_fs_path_operation(
        &self,
        _from: &str,
        _operation: code::FsPathOperation,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_fs_path_operation not yet implemented".into())
    }

    fn emit_errno(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_errno not yet implemented".into())
    }

    fn emit_libc_call(
        &self,
        _base: &str,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_libc_call not yet implemented".into())
    }

    fn emit_open_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_open_file not yet implemented".into())
    }

    fn emit_read_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_read_file not yet implemented".into())
    }

    fn emit_close_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_close_file not yet implemented".into())
    }

    fn emit_sync_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_sync_file not yet implemented".into())
    }

    fn emit_seek_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_seek_file not yet implemented".into())
    }

    fn emit_rename_path(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_rename_path not yet implemented".into())
    }

    fn emit_mkstemps(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_mkstemps not yet implemented".into())
    }

    fn emit_temp_directory(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_temp_directory not yet implemented".into())
    }

    fn emit_opendir(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_opendir not yet implemented".into())
    }

    fn emit_readdir(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_readdir not yet implemented".into())
    }

    fn emit_closedir(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_closedir not yet implemented".into())
    }

    fn dirent_name_offset(&self) -> usize {
        // Linux `struct dirent`: d_name at offset 19.
        19
    }

    fn dirent_name_length_offset(&self) -> usize {
        0
    }

    fn emit_realpath(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_realpath not yet implemented".into())
    }

    fn emit_variadic_call(
        &self,
        _base: &str,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("x86_64 Phase 1: emit_variadic_call not yet implemented".into())
    }

    // --- net constants (Linux values; mirror linux_aarch64) ----------------

    fn addrinfo_addr_offset(&self) -> usize {
        24
    }
    fn sol_socket(&self) -> &'static str {
        "1"
    }
    fn so_reuseaddr(&self) -> &'static str {
        "2"
    }
    fn so_rcvtimeo(&self) -> &'static str {
        "20"
    }
    fn so_sndtimeo(&self) -> &'static str {
        "21"
    }
    fn eagain(&self) -> &'static str {
        "11"
    }
    fn emsgsize(&self) -> &'static str {
        "90"
    }
    fn o_nonblock(&self) -> &'static str {
        "2048"
    }
    fn einprogress(&self) -> &'static str {
        "115"
    }
    fn so_error(&self) -> &'static str {
        "4"
    }
}
