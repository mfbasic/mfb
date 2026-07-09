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
use crate::target::linux_gtk as gtk;
use crate::target::shared::code::{
    self, AppEntrySpec, CodeDataObject, CodeFrame, CodeFunction, CodeInstruction, CodeRelocation,
    MirPlan, NativeCodePlan, ProgramEntrySpec, RelocIntent,
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

    // Raw ELF entry: argc/argv are on the initial stack, not in registers.
    fn entry_args_in_registers(&self) -> bool {
        false
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
        platform_imports: &HashMap<String, String>,
    ) -> Result<CodeFunction, String> {
        // Same shared entry as AArch64 (plan-00-H): now that x86 links libc and
        // routes through the MIR seam, the full Result-tag error-reporting /
        // signal / RNG-seed / global-init entry works unchanged — `select_x86`
        // maps the neutral registers to their SysV homes (`x31`→r14 zero reg,
        // `arena_base`→r15, the scratch pool → the caller/callee-saved GPRs).
        let mut function = code::lower_program_entry(
            spec.entry_symbol,
            spec.language_entry_symbol,
            spec.language_entry_returns,
            spec.language_entry_accepts_args,
            spec.global_initializer_symbol,
            spec.link_init_symbol,
            spec.entry_stack_size,
            spec.global_slot_count,
            platform_imports,
            self,
            spec.emit_cleanup_failure_audit,
            spec.seed_rng,
            spec.register_signal_handlers,
            spec.capture_args,
        )?;
        // The shared entry uses the neutral zero register `x31` for its arena/
        // global zero-init stores, relying on it *being* zero — true for AArch64's
        // hardware `xzr`, but on x86 `x31` realizes to `r14`, an ordinary GPR that
        // holds garbage from `_start`. Zero it once, right after the entry label,
        // before any `store x31` runs. `eor x31,x31,x31` selects to `xor r14,r14`.
        let zero = abi::exclusive_or_registers("x31", "x31", "x31");
        let at = usize::from(
            function
                .instructions
                .first()
                .is_some_and(|inst| inst.op == crate::arch::aarch64::ops::CodeOp::Label),
        );
        function.instructions.insert(at, zero);
        Ok(function)
    }

    fn emit_program_exit(
        &self,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // App mode (plan-05-linux-app.md §6.7): the worker program reports
        // completion through the GTK finish helper instead of hard-exiting, so
        // the main thread (GTK loop) decides the shutdown policy — mirrors
        // linux-aarch64.
        if from == code::MACAPP_PROGRAM_SYMBOL {
            instructions.extend([
                abi::branch_link(gtk::FINISH_SYMBOL),
                abi::branch_self(),
                abi::return_(),
            ]);
            relocations.push(CodeRelocation {
                from: from.to_string(),
                to: gtk::FINISH_SYMBOL.to_string(),
                kind: RelocIntent::Call,
                binding: "internal".to_string(),
                library: None,
            });
            return Ok(());
        }
        // exit_group(code). The shared callers place the exit code in the neutral
        // return register `x0`; because this syscall immediately follows, select
        // maps that `x0` to the syscall's first argument (rdi) at the caller's own
        // instruction. So the code is already in rdi — only the syscall number is
        // needed (`x8`→rax). A prior `mov rdi,rax` here wrongly overwrote the code
        // with the leaked variadic `al`=8 (rax) left by the pre-shutdown call.
        instructions.push(abi::move_immediate("x8", "Integer", SYS_EXIT_GROUP));
        instructions.push(abi::syscall());
        instructions.push(abi::branch_self());
        // Unreachable, but every function the validator sees needs a return op
        // (callers like the signal handler end with this).
        instructions.push(abi::return_());
        Ok(())
    }

    // --- Linux GTK4 app mode (shared with linux-aarch64 via target::linux_gtk;
    // the x86 variants bracket every callback/helper for the SysV callee-saved
    // contract + the r14 zero register, and use the per-ISA entry trampoline) ---

    fn emit_app_program_entry(
        &self,
        spec: &AppEntrySpec,
        platform_imports: &HashMap<String, String>,
    ) -> Option<Result<Vec<CodeFunction>, String>> {
        Some(gtk::emit_app_program_entry_x86(spec, platform_imports))
    }

    fn app_mode_data_objects(&self) -> Vec<CodeDataObject> {
        gtk::app_mode_data_objects()
    }

    fn emit_app_io_write_helper(
        &self,
        symbol: &str,
        stderr: bool,
        newline: bool,
        _term_state_offset: Option<usize>,
        _platform_imports: &HashMap<String, String>,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(gtk::wrap_x86_helper(gtk::emit_app_io_write_helper(
            symbol, stderr, newline,
        ))))
    }

    fn emit_app_io_flush_helper(
        &self,
        symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(gtk::wrap_x86_helper(gtk::emit_app_io_flush_helper(
            symbol,
        ))))
    }

    fn emit_app_io_input_helper(
        &self,
        symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(gtk::wrap_x86_helper(gtk::emit_app_io_input_helper(
            symbol,
        ))))
    }

    fn emit_app_raw_input_mode(
        &self,
        symbol: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        gtk::emit_set_raw_input_mode(instructions, relocations, symbol);
        Some(Ok(()))
    }

    fn emit_app_io_is_terminal_helper(
        &self,
        symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(gtk::wrap_x86_helper(
            gtk::emit_app_io_is_terminal_helper(symbol),
        )))
    }

    fn emit_app_term_helper(
        &self,
        call: &str,
        symbol: &str,
        term_state_offset: usize,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        gtk::emit_app_term_helper(call, symbol, term_state_offset)
            .map(gtk::wrap_x86_helper)
            .map(Ok)
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
        platform_imports: &HashMap<String, String>,
    ) -> Result<CodeFunction, String> {
        // Same shared trampoline as AArch64: pthread hands the control block in
        // the first argument register; the body is alias-free machine-floor
        // code (x13/x14/x20 scratch) that selects cleanly through the x86 remap.
        let mut function = code::lower_thread_trampoline(platform_imports, self)?;
        // Zero the x86 zero register for THIS thread. `x31` realizes as `r14`,
        // which the program entry zeroes once for the main thread — but a
        // pthread worker starts with whatever musl left in r14, so every
        // "zero" in the worker (string NUL terminators, queue/tag zero-init,
        // zero compares) would be garbage. Mirrors `emit_program_entry`.
        let zero = abi::exclusive_or_registers("x31", "x31", "x31");
        let at = usize::from(
            function
                .instructions
                .first()
                .is_some_and(|inst| inst.op == crate::arch::aarch64::ops::CodeOp::Label),
        );
        function.instructions.insert(at, zero);
        // Re-bias the stack for SysV alignment. pthread enters the trampoline
        // like any C callee (rsp ≡ 8 mod 16); the shared trampoline's 80-byte
        // frame keeps that parity, so every function it calls would be entered
        // at ≡ 0 — the whole worker call tree then runs 8 off the C convention
        // and musl's SSE locals (movaps/movdqa on [rsp+K] in fstatat,
        // pthread_create, …) fault. An extra 8-byte bias (popped before the
        // trampoline's return) restores ≡ 0 at its call instructions, exactly
        // what SysV requires. The trampoline's own [sp, K] slots are relative
        // to the final sp, so they are unaffected. AArch64 needs no bias.
        function.instructions.insert(at + 1, abi::subtract_stack(8));
        let mut i = at + 2;
        while i < function.instructions.len() {
            if function.instructions[i].op == crate::arch::aarch64::ops::CodeOp::Ret {
                function.instructions.insert(i, abi::add_stack(8));
                i += 2;
            } else {
                i += 1;
            }
        }
        Ok(function)
    }

    // --- Runtime-helper OS methods (deferred to a later phase) --------------

    fn emit_poll_input(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("poll", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_is_terminal(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("isatty", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_terminal_size(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("ioctl", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_path_exists(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([abi::move_immediate("x1", "Integer", "0")]);
        self.emit_libc_call("access", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_path_stat(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("stat", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn stat_mode_offset(&self) -> usize {
        // Linux x86-64 `struct stat`: st_mode lives at offset 24.
        24
    }

    fn emit_current_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("getcwd", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_environ_pointer(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // `environ` is an imported libc data global. On x86-64 the fused
        // `adrp`/`add` pair lowers to a single GOTPCREL `mov` that loads
        // `&environ` from the GOT slot; one further deref gives the `char**`.
        let library = platform_imports
            .get("environ")
            .ok_or_else(|| "os.environ runtime helper requires environ import".to_string())?
            .clone();
        let dst = abi::return_register();
        instructions.push(abi::load_page_address(dst, "environ"));
        instructions.push(abi::add_page_offset(dst, dst, "environ"));
        for kind in [RelocIntent::GotLoadHi, RelocIntent::GotLoadLo] {
            relocations.push(CodeRelocation {
                from: from.to_string(),
                to: "environ".to_string(),
                kind,
                binding: "external".to_string(),
                library: Some(library.clone()),
            });
        }
        instructions.push(abi::load_u64(dst, dst, 0));
        Ok(())
    }

    fn emit_fs_path_operation(
        &self,
        from: &str,
        operation: code::FsPathOperation,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let symbol = match operation {
            code::FsPathOperation::Chdir => "chdir",
            code::FsPathOperation::Unlink => "unlink",
            code::FsPathOperation::Mkdir => "mkdir",
            code::FsPathOperation::Rmdir => "rmdir",
        };
        if matches!(operation, code::FsPathOperation::Mkdir) {
            instructions.push(abi::move_immediate("x1", "Integer", "493"));
        }
        self.emit_libc_call(symbol, from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_errno(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call(
            "__errno_location",
            from,
            platform_imports,
            instructions,
            relocations,
        )?;
        instructions.push(abi::load_u32("x9", abi::return_register(), 0));
        Ok(())
    }

    fn emit_libc_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Call an imported libc function through the PLT: `bl base` selects to a
        // `call rel32` whose reloc the linker binds to `base`'s PLT stub (which
        // jumps through the GOT slot the loader filled). Same shape as AArch64.
        let library = platform_imports
            .get(base)
            .ok_or_else(|| format!("runtime helper requires {base} import"))?
            .clone();
        instructions.push(abi::branch_link(base));
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: base.to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(library),
        });
        Ok(())
    }

    fn emit_open_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_variadic_call("open", from, platform_imports, instructions, relocations)
    }

    fn emit_read_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("read", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_close_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("close", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_sync_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("fsync", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_seek_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("lseek", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_rename_path(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("rename", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_mkstemps(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call(
            "mkstemps",
            from,
            platform_imports,
            instructions,
            relocations,
        )?;
        Ok(())
    }

    fn emit_temp_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        const BUFFER_SLOT: usize = 24;
        const CAPACITY_SLOT: usize = 32;

        let env_ok = format!("{from}_tmpdir_env_ok");
        let env_len_loop = format!("{from}_tmpdir_env_len_loop");
        let env_len_done = format!("{from}_tmpdir_env_len_done");
        let copy_loop = format!("{from}_tmpdir_copy_loop");
        let copy_done = format!("{from}_tmpdir_copy_done");
        let fallback = format!("{from}_tmpdir_fallback");
        let done = format!("{from}_tmpdir_done");

        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), BUFFER_SLOT),
            abi::store_u64("x1", abi::stack_pointer(), CAPACITY_SLOT),
            abi::move_register("x10", abi::return_register()),
        ]);
        for (offset, byte) in b"TMPDIR\0".iter().enumerate() {
            instructions.extend([
                abi::move_immediate("x9", "Byte", &byte.to_string()),
                abi::store_u8("x9", "x10", offset),
            ]);
        }
        self.emit_libc_call("getenv", from, platform_imports, instructions, relocations)?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&env_ok),
            abi::branch(&fallback),
            abi::label(&env_ok),
            abi::load_u64("x11", abi::stack_pointer(), BUFFER_SLOT),
            abi::load_u64("x16", abi::stack_pointer(), CAPACITY_SLOT),
            abi::move_register("x12", abi::return_register()),
            abi::move_register("x13", "x12"),
            abi::move_immediate("x14", "Integer", "0"),
            abi::label(&env_len_loop),
            abi::load_u8("x9", "x13", 0),
            abi::compare_immediate("x9", "0"),
            abi::branch_eq(&env_len_done),
            abi::add_immediate("x13", "x13", 1),
            abi::add_immediate("x14", "x14", 1),
            abi::compare_registers("x14", "x16"),
            abi::branch_ge(&fallback),
            abi::branch(&env_len_loop),
            abi::label(&env_len_done),
            abi::compare_immediate("x14", "0"),
            abi::branch_eq(&fallback),
            abi::move_immediate("x15", "Integer", "0"),
            abi::label(&copy_loop),
            abi::compare_registers("x15", "x14"),
            abi::branch_eq(&copy_done),
            abi::load_u8("x9", "x12", 0),
            abi::store_u8("x9", "x11", 0),
            abi::add_immediate("x12", "x12", 1),
            abi::add_immediate("x11", "x11", 1),
            abi::add_immediate("x15", "x15", 1),
            abi::branch(&copy_loop),
            abi::label(&copy_done),
            abi::store_u8("x31", "x11", 0),
            abi::move_register(abi::return_register(), "x14"),
            abi::branch(&done),
            abi::label(&fallback),
            abi::load_u64("x11", abi::stack_pointer(), BUFFER_SLOT),
        ]);
        for (offset, byte) in b"/tmp\0".iter().enumerate() {
            instructions.extend([
                abi::move_immediate("x9", "Byte", &byte.to_string()),
                abi::store_u8("x9", "x11", offset),
            ]);
        }
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "4"),
            abi::label(&done),
        ]);
        Ok(())
    }

    fn emit_opendir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("opendir", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_readdir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call("readdir", from, platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_closedir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call(
            "closedir",
            from,
            platform_imports,
            instructions,
            relocations,
        )?;
        Ok(())
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
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call(
            "realpath",
            from,
            platform_imports,
            instructions,
            relocations,
        )?;
        Ok(())
    }

    fn emit_variadic_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // The Linux AArch64 ABI passes variadic GP arguments in registers, so the
        // trailing variadic argument in `x2` needs no special handling.
        self.emit_libc_call(base, from, platform_imports, instructions, relocations)
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
