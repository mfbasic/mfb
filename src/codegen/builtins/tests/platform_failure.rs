//! An emit failure PAST THE FIRST one in a body must propagate too.
//!
//! `registry_bodies.rs` and `abi_inline.rs` already lower every registry body
//! with an EMPTY import list, on all five backends, which forces the FIRST
//! `platform.emit_external_call(…)?` in each body to fail. That is the import
//! guard, and it is covered.
//!
//! What it cannot reach is every `?` after it. A body is a sequence — open a
//! socket, set it non-blocking, connect, read — and the first failure aborts
//! the rest, so the second and later `?` sites stay exactly as dead as before.
//! They are the single largest identifiable class left in the coverage gap: 931
//! `?`-propagation lines across the files below the floor, 580 of them in
//! `src/codegen/builtins` alone, and `tls/gen_openssl.rs` has 80 by itself.
//!
//! Each one is a real contract. A body that swallowed the failure to emit
//! `connect` would emit a function that opens a socket and then reads from one
//! nothing connected — not a crash, a wrong program.
//!
//! [`FailAt`] wraps a real platform and fails its Nth fallible call. Driving N
//! from 0 to however many a clean lowering of that body makes walks its `?`
//! sites one at a time, and each must come back as a refusal carrying the
//! injected message.
//!
//! Two details that decide whether this measures anything:
//!
//! **It delegates rather than stubs.** `CodegenPlatform` has 36 defaulted
//! methods, so an impl overriding only the interesting ones would substitute
//! DEFAULT behaviour for the real platform's everywhere else — lowering a
//! different body than the one under test. The forwarding impl is generated
//! from the trait definition, so a method added to the trait is a compile error
//! here rather than a silent hole.
//!
//! **It grants the import each call names.** `emit_external_call` refuses a
//! symbol the plan never declared, which is what the empty-list sweep tests; if
//! this ran with the same empty list every body would stop at its first call
//! again and there would be nothing new. So the wrapper inserts the symbol into
//! the map it forwards. That is the one place it lies to the emitter, and it
//! lies in the direction that lets the body run.

use std::cell::Cell;
use std::collections::HashMap;

use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::BuilderHarness;
use crate::codegen::engine::types::{
    AppEntrySpec, ArenaInitSymbols, CodeDataObject, CodeFunction, CodeInstruction, CodeRelocation,
    CodegenPlatform, FsPathOperation, PlatformFamily, ProgramEntrySpec, TerminalControlCall,
};
use crate::codegen::engine::util::vreg_frame::Vregs;
use crate::codegen::registry::{registry, AbiCtx, Body};
use crate::os::linux::flavor::LinuxFlavor;

/// The message the injected failure carries, distinctive enough that a refusal
/// from another cause cannot be mistaken for it.
const INJECTED: &str = "$injected platform emit failure";

/// A real platform whose Nth fallible call is replaced by a failure.
struct FailAt<'a> {
    inner: &'a dyn CodegenPlatform,
    /// The index of the one call to fail. `usize::MAX` fails none, which is how
    /// the clean run counts them.
    target: Cell<usize>,
    made: Cell<usize>,
}

impl<'a> FailAt<'a> {
    fn new(inner: &'a dyn CodegenPlatform, at: usize) -> Self {
        FailAt {
            inner,
            target: Cell::new(at),
            made: Cell::new(0),
        }
    }

    /// A platform that never fails, used to count the calls a clean lowering of
    /// one body makes.
    fn counting(inner: &'a dyn CodegenPlatform) -> Self {
        FailAt::new(inner, usize::MAX)
    }

    fn calls(&self) -> usize {
        self.made.get()
    }

    /// Fail EXACTLY the target call, and let every other one through.
    ///
    /// Not "fail from the target onward". A wrapper that stayed failed made the
    /// sweep insensitive to the very defect it is for: a body that swallowed
    /// call #n would run on to call #n+1, fail THERE, and return the same `Err`
    /// the test was looking for. Verified by making `os/func_has_env.rs`
    /// swallow its `getenv` emit — the sweep passed, until this.
    fn tick(&self) -> Result<(), String> {
        let index = self.made.get();
        self.made.set(index + 1);
        if index == self.target.get() {
            return Err(INJECTED.to_string());
        }
        Ok(())
    }

    /// The caller's import map with `symbol` declared, so the emitter's import
    /// guard passes and the body proceeds to its next call.
    fn granting(imports: &HashMap<String, String>, symbol: &str) -> HashMap<String, String> {
        let mut granted = imports.clone();
        granted.insert(symbol.to_string(), "c".to_string());
        granted
    }
}

impl CodegenPlatform for FailAt<'_> {
    fn target(&self) -> &'static str {
        self.inner.target()
    }
    fn family(&self) -> PlatformFamily {
        self.inner.family()
    }
    fn arch(&self) -> &'static str {
        self.inner.arch()
    }
    fn backend(&self) -> &'static dyn crate::codegen::engine::mir::Backend {
        self.inner.backend()
    }
    fn emit_arena_start_time(
        &self,
        entry_symbol: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_arena_start_time(entry_symbol, platform_imports, instructions, relocations)
    }
    fn emit_lib_open(
        &self,
        filename_symbol: &str,
        vendored: bool,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_lib_open(
            filename_symbol,
            vendored,
            from,
            platform_imports,
            instructions,
            relocations,
        )
    }
    fn emit_lib_get_sym(
        &self,
        handle_reg: &str,
        symbol_symbol: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_lib_get_sym(
            handle_reg,
            symbol_symbol,
            from,
            platform_imports,
            instructions,
            relocations,
        )
    }
    fn entry_args_in_registers(&self) -> bool {
        self.inner.entry_args_in_registers()
    }
    fn defers_arg_capture(&self) -> bool {
        self.inner.defers_arg_capture()
    }
    fn emit_build_argv_utf8(
        &self,
        _entry_symbol: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_build_argv_utf8(
            _entry_symbol,
            _platform_imports,
            _instructions,
            _relocations,
        )
    }
    fn entry_stack_misaligned_on_entry(&self) -> bool {
        self.inner.entry_stack_misaligned_on_entry()
    }
    fn libc(&self) -> Option<crate::manifest::libraries::Libc> {
        self.inner.libc()
    }
    fn emit_enable_vt_output(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_enable_vt_output(_from, _platform_imports, _instructions, _relocations)
    }
    fn emit_console_utf8(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_console_utf8(_from, _platform_imports, _instructions, _relocations)
    }
    fn emit_verify_nofollow(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_verify_nofollow(_from, _platform_imports, _instructions, _relocations)
    }
    fn emit_verify_within(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_verify_within(_from, _platform_imports, _instructions, _relocations)
    }
    fn emit_env_get(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_env_get(_from, _platform_imports, _instructions, _relocations)
    }
    fn emit_env_set(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_env_set(_from, _platform_imports, _instructions, _relocations)
    }
    fn emit_heap_alloc(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_heap_alloc(from, platform_imports, instructions, relocations)
    }
    fn emit_heap_free(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_heap_free(from, platform_imports, instructions, relocations)
    }
    fn emit_os_wide_string(
        &self,
        _which: &str,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_os_wide_string(
            _which,
            _from,
            _platform_imports,
            _instructions,
            _relocations,
        )
    }
    fn emit_apply_raw_mode(
        &self,
        base_register: &str,
        original_offset: usize,
        modified_offset: usize,
        disable_echo: bool,
        disable_canonical: bool,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        self.inner.emit_apply_raw_mode(
            base_register,
            original_offset,
            modified_offset,
            disable_echo,
            disable_canonical,
            instructions,
        )
    }
    fn emit_terminal_control_call(
        &self,
        call: TerminalControlCall,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_terminal_control_call(
            call,
            from,
            platform_imports,
            instructions,
            relocations,
        )
    }
    fn emit_program_exit(
        &self,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_program_exit(from, instructions, relocations)
    }
    fn emit_write(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_write(from, platform_imports, instructions, relocations)
    }
    fn emit_poll_input(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_poll_input(from, platform_imports, instructions, relocations)
    }
    fn emit_is_terminal(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_is_terminal(from, platform_imports, instructions, relocations)
    }
    fn emit_terminal_size(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_terminal_size(from, platform_imports, instructions, relocations)
    }
    fn emit_path_exists(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_path_exists(from, platform_imports, instructions, relocations)
    }
    fn emit_path_stat(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_path_stat(from, platform_imports, instructions, relocations)
    }
    fn emit_stat_is_kind(
        &self,
        stat_offset: usize,
        expected_kind: &str,
        mode: &str,
        mask: &str,
        expected: &str,
        found: &str,
        missing: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        self.inner.emit_stat_is_kind(
            stat_offset,
            expected_kind,
            mode,
            mask,
            expected,
            found,
            missing,
            instructions,
        )
    }
    fn emit_current_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_current_directory(from, platform_imports, instructions, relocations)
    }
    fn emit_environ_pointer(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_environ_pointer(from, platform_imports, instructions, relocations)
    }
    fn emit_fs_path_operation(
        &self,
        from: &str,
        operation: FsPathOperation,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_fs_path_operation(
            from,
            operation,
            platform_imports,
            instructions,
            relocations,
        )
    }
    fn emit_errno(
        &self,
        from: &str,
        dst: Operand,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_errno(from, dst, platform_imports, instructions, relocations)
    }
    fn emit_external_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_external_call(
            base,
            from,
            &Self::granting(platform_imports, base),
            instructions,
            relocations,
        )
    }
    fn emit_open_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_open_file(from, platform_imports, instructions, relocations)
    }
    fn emit_read_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_read_file(from, platform_imports, instructions, relocations)
    }
    fn emit_close_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_close_file(from, platform_imports, instructions, relocations)
    }
    fn emit_sync_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_sync_file(from, platform_imports, instructions, relocations)
    }
    fn emit_seek_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_seek_file(from, platform_imports, instructions, relocations)
    }
    fn emit_rename_path(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_rename_path(from, platform_imports, instructions, relocations)
    }
    fn emit_mkstemps(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_mkstemps(from, platform_imports, instructions, relocations)
    }
    fn emit_random_bytes(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_random_bytes(from, platform_imports, instructions, relocations)
    }
    fn emit_temp_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_temp_directory(from, platform_imports, instructions, relocations)
    }
    fn emit_opendir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_opendir(from, platform_imports, instructions, relocations)
    }
    fn emit_readdir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_readdir(from, platform_imports, instructions, relocations)
    }
    fn emit_closedir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_closedir(from, platform_imports, instructions, relocations)
    }
    fn emit_read_dir_entry(
        &self,
        prefix: &str,
        nameptr: &str,
        namelen: &str,
        byte: &str,
        scratch: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        self.inner
            .emit_read_dir_entry(prefix, nameptr, namelen, byte, scratch, instructions)
    }
    fn emit_realpath(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_realpath(from, platform_imports, instructions, relocations)
    }
    fn emit_arena_map(
        &self,
        size_reg: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_arena_map(size_reg, instructions)
    }
    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_arena_unmap(instructions)
    }
    fn addrinfo_addr_offset(&self) -> usize {
        self.inner.addrinfo_addr_offset()
    }
    fn sol_socket(&self) -> &'static str {
        self.inner.sol_socket()
    }
    fn so_reuseaddr(&self) -> &'static str {
        self.inner.so_reuseaddr()
    }
    fn so_rcvtimeo(&self) -> &'static str {
        self.inner.so_rcvtimeo()
    }
    fn so_sndtimeo(&self) -> &'static str {
        self.inner.so_sndtimeo()
    }
    fn so_rcvbuf(&self) -> &'static str {
        self.inner.so_rcvbuf()
    }
    fn ipproto_ip(&self) -> &'static str {
        self.inner.ipproto_ip()
    }
    fn ip_ttl(&self) -> &'static str {
        self.inner.ip_ttl()
    }
    fn ip_recvttl(&self) -> &'static str {
        self.inner.ip_recvttl()
    }
    fn cmsg_ip_ttl_type(&self) -> &'static str {
        self.inner.cmsg_ip_ttl_type()
    }
    fn clock_monotonic(&self) -> &'static str {
        self.inner.clock_monotonic()
    }
    fn socket_would_block_code(&self) -> &'static str {
        self.inner.socket_would_block_code()
    }
    fn socket_message_size_code(&self) -> &'static str {
        self.inner.socket_message_size_code()
    }
    fn socket_in_progress_code(&self) -> &'static str {
        self.inner.socket_in_progress_code()
    }
    fn so_error(&self) -> &'static str {
        self.inner.so_error()
    }
    fn emit_set_nonblocking(
        &self,
        fd_offset: usize,
        flags_offset: usize,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_set_nonblocking(
            fd_offset,
            flags_offset,
            from,
            platform_imports,
            instructions,
            relocations,
        )
    }
    fn emit_restore_blocking(
        &self,
        _fd_offset: usize,
        _scratch_offset: usize,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_restore_blocking(
            _fd_offset,
            _scratch_offset,
            _from,
            _platform_imports,
            _instructions,
            _relocations,
        )
    }
    fn emit_net_startup(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_net_startup(_from, _platform_imports, _instructions, _relocations)
    }
    fn emit_net_shutdown(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_net_shutdown(_from, _platform_imports, _instructions, _relocations)
    }
    fn emit_variadic_external_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_variadic_external_call(
            base,
            from,
            &Self::granting(platform_imports, base),
            instructions,
            relocations,
        )
    }
    fn emit_app_program_entry(
        &self,
        _spec: &AppEntrySpec,
        _platform_imports: &HashMap<String, String>,
    ) -> Option<Result<Vec<CodeFunction>, String>> {
        self.inner.emit_app_program_entry(_spec, _platform_imports)
    }
    fn emit_program_entry(
        &self,
        spec: &ProgramEntrySpec<'_>,
        platform_imports: &HashMap<String, String>,
    ) -> Result<CodeFunction, String> {
        self.tick()?;
        self.inner.emit_program_entry(spec, platform_imports)
    }
    fn emit_thread_trampoline(
        &self,
        platform_imports: &HashMap<String, String>,
        uses_stdin: bool,
        arena_init: ArenaInitSymbols,
    ) -> Result<CodeFunction, String> {
        self.tick()?;
        self.inner
            .emit_thread_trampoline(platform_imports, uses_stdin, arena_init)
    }
    fn emit_tls_block_trampolines(&self, server: bool) -> Vec<CodeFunction> {
        self.inner.emit_tls_block_trampolines(server)
    }
    fn app_mode_data_objects(&self, project_name: &str) -> Vec<CodeDataObject> {
        self.inner.app_mode_data_objects(project_name)
    }
    fn app_mode_reconcile_data_objects(&self) -> Vec<CodeDataObject> {
        self.inner.app_mode_reconcile_data_objects()
    }
    fn emit_app_io_write(
        &self,
        _symbol: &str,
        _stderr: bool,
        _newline: bool,
        _term_state_offset: Option<usize>,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        self.inner.emit_app_io_write(
            _symbol,
            _stderr,
            _newline,
            _term_state_offset,
            _platform_imports,
            _instructions,
            _relocations,
        )
    }
    fn emit_app_io_flush(
        &self,
        _symbol: &str,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        self.inner
            .emit_app_io_flush(_symbol, _instructions, _relocations)
    }
    fn emit_app_io_input(
        &self,
        _symbol: &str,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        self.inner
            .emit_app_io_input(_symbol, _instructions, _relocations)
    }
    fn emit_app_raw_input_mode(
        &self,
        _symbol: &str,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        self.inner
            .emit_app_raw_input_mode(_symbol, _instructions, _relocations)
    }
    fn emit_app_io_is_terminal(
        &self,
        _symbol: &str,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        self.inner
            .emit_app_io_is_terminal(_symbol, _instructions, _relocations)
    }
    fn emit_app_term_helper(
        &self,
        _call: &str,
        _symbol: &str,
        _term_state_offset: usize,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        self.inner.emit_app_term_helper(
            _call,
            _symbol,
            _term_state_offset,
            _instructions,
            _relocations,
        )
    }
    fn emit_app_mode_reconcile(
        &self,
        _symbol: &str,
        _presentation_mode_offset: usize,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        self.inner.emit_app_mode_reconcile(
            _symbol,
            _presentation_mode_offset,
            _instructions,
            _relocations,
        )
    }
    fn emit_canvas_blit(
        &self,
        _symbol: &str,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        self.inner
            .emit_canvas_blit(_symbol, _instructions, _relocations)
    }
    fn emit_metal_init(
        &self,
        _symbol: &str,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        self.inner
            .emit_metal_init(_symbol, _instructions, _relocations)
    }
    fn emit_metal_draw(
        &self,
        _symbol: &str,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        self.inner
            .emit_metal_draw(_symbol, _instructions, _relocations)
    }
}

/// The five real backends. Copied in shape from `registry_bodies.rs` and for
/// the same reason: a body that reaches the OS branches on `family()` and has a
/// different call sequence in each arm, so sweeping one backend leaves the
/// others' `?` sites exactly as dead as they were.
fn os_seam_platforms() -> Vec<(&'static str, Box<dyn CodegenPlatform>)> {
    vec![
        (
            "macos-aarch64",
            Box::new(crate::target::macos_aarch64::code::Platform) as Box<dyn CodegenPlatform>,
        ),
        (
            "windows-x86_64",
            Box::new(crate::target::win_x86_64::code::Platform),
        ),
        (
            "linux-aarch64",
            Box::new(crate::target::linux_common::code::Platform::for_test(
                crate::target::linux_aarch64::code::Aarch64,
                LinuxFlavor::Glibc,
            )),
        ),
        (
            "linux-x86_64",
            Box::new(crate::target::linux_common::code::Platform::for_test(
                crate::target::linux_x86_64::code::X86_64,
                LinuxFlavor::Glibc,
            )),
        ),
        (
            "linux-riscv64",
            Box::new(crate::target::linux_common::code::Platform::for_test(
                crate::target::linux_riscv64::code::Riscv64,
                LinuxFlavor::Glibc,
            )),
        ),
    ]
}

/// Lower one `abi_function` body with `platform`, swallowing a panic.
///
/// A body that PANICS is not a finding here: `AppSupport::require_gtk`
/// hard-stops an ISA with no app-mode port at the boundary, deliberately, and
/// `registry_bodies.rs` already asserts that is the only one. This sweep is
/// about the bodies that RUN, so a panic just ends that body's walk.
fn lower_body(
    lower: crate::codegen::registry::AbiFunction,
    params: &[crate::codegen::registry::Parameter],
    call: &str,
    platform: &dyn CodegenPlatform,
) -> Option<Result<ValueResult, String>> {
    let mut vregs = Vregs::new();
    let args: Vec<ValueResult> = params
        .iter()
        .map(|param| ValueResult {
            type_: param.ty.clone(),
            location: Operand::from(vregs.next().as_str()),
            text: param.name.to_string(),
            origin: None,
        })
        .collect();
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("_mfb_rt_probe", platform);
    let base = harness.abi_ctx(platform);
    let ctx = AbiCtx { call, ..base };

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let lowered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        lower(&mut builder, &args, &ctx)
    }));
    std::panic::set_hook(hook);
    lowered.ok()
}

/// Every fallible platform call an `abi_function` body makes, failed one at a
/// time, must come back as a refusal.
///
/// The clean run of each body comes first and its call count is that body's
/// loop bound, so this measures the surface rather than hardcoding it: a body
/// that grows another `emit_*` is covered the day it lands.
///
/// A failure that does NOT propagate is the defect this is written for — the
/// body returns `Ok`, having emitted a sequence with a hole in it. The
/// assertion names the member and the index so the call site is findable.
#[test]
fn an_emit_failure_past_the_first_one_still_propagates() {
    let mut swept = 0usize;
    let mut injected = 0usize;
    let mut swallowed: Vec<String> = Vec::new();

    for (target, platform) in os_seam_platforms() {
        for package in registry().packages() {
            for function in package.functions() {
                for implementation in function.implementations() {
                    let Body::AbiFunction { lower, .. } = implementation.body else {
                        continue;
                    };
                    let call = format!("{}.{}", package.import_name(), function.name);

                    // How many fallible platform calls a clean lowering makes.
                    // A body that refuses outright, or panics, contributes none:
                    // refusing is how a member says it is not implemented here.
                    let counting = FailAt::counting(platform.as_ref());
                    let Some(Ok(_)) = lower_body(lower, &implementation.params, &call, &counting)
                    else {
                        continue;
                    };
                    let calls = counting.calls();
                    if calls == 0 {
                        continue;
                    }
                    swept += 1;

                    for n in 0..calls {
                        let failing = FailAt::new(platform.as_ref(), n);
                        injected += 1;
                        match lower_body(lower, &implementation.params, &call, &failing) {
                            Some(Err(message)) if message.contains(INJECTED) => {}
                            Some(Err(message)) => swallowed.push(format!(
                                "{target} {call} call #{n}: a different refusal came \
                                 back, so something between the emit and the caller \
                                 replaced the error rather than propagating it: \
                                 {message}"
                            )),
                            Some(Ok(_)) => swallowed.push(format!(
                                "{target} {call} call #{n}: the emit failed and the \
                                 body returned Ok. It swallowed the failure, so the \
                                 sequence it emitted is missing that call."
                            )),
                            None => swallowed.push(format!(
                                "{target} {call} call #{n}: the body PANICKED on the \
                                 error path, though it lowered cleanly without the \
                                 injection. An error path that panics is worse than \
                                 one that propagates."
                            )),
                        }
                    }
                }
            }
        }
    }

    assert!(
        swallowed.is_empty(),
        "{} of {injected} injected failures did not propagate:\n{}",
        swallowed.len(),
        swallowed.join("\n")
    );
    // The row that keeps the rest honest: every assertion above holds against a
    // sweep that found no bodies at all.
    // Measured at 331 bodies / 1,997 injections when this landed; the bound is
    // set well below so ordinary registry churn does not red it, and well above
    // zero so the sweep going quiet does.
    assert!(
        swept > 250 && injected > 1500,
        "only {swept} bodies and {injected} injected failures across five \
         backends -- the sweep stopped finding the bodies it is for"
    );
}
