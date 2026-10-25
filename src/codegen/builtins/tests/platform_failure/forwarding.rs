//! plan-176-H: `impl CodegenPlatform for FailAt`, which forwards every trait method (so a
//! new trait method is a compile error here). `FailAt` and its helpers live in
//! `platform_failure/mod.rs`; as a child module this impl reaches their private fields.

use super::*;

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
        handle_reg: crate::target::shared::abi::Reg,
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
    fn emit_release_marshalled(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_release_marshalled(from, platform_imports, instructions, relocations)
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
        base_register: crate::target::shared::abi::Reg,
        original_offset: usize,
        modified_offset: usize,
        disable_echo: bool,
        disable_canonical: bool,
        scratch: [crate::target::shared::abi::Reg; 2],
        instructions: &mut Vec<CodeInstruction>,
    ) {
        self.inner.emit_apply_raw_mode(
            base_register,
            original_offset,
            modified_offset,
            disable_echo,
            disable_canonical,
            scratch,
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
        mode: crate::target::shared::abi::Reg,
        mask: crate::target::shared::abi::Reg,
        expected: crate::target::shared::abi::Reg,
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
        dst: crate::target::shared::abi::Reg,
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
        nameptr: crate::target::shared::abi::Reg,
        namelen: crate::target::shared::abi::Reg,
        byte: crate::target::shared::abi::Reg,
        scratch: crate::target::shared::abi::Reg,
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
        size_reg: crate::target::shared::abi::Reg,
        instructions: &mut Vec<CodeInstruction>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_arena_map(size_reg, instructions)
    }
    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_arena_unmap(instructions)
    }
    fn emit_host_memory_probe(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        ram: crate::target::shared::abi::Reg,
        scratch: crate::target::shared::abi::Reg,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_host_memory_probe(
            from,
            platform_imports,
            ram,
            scratch,
            instructions,
            relocations,
        )
    }
    fn host_memory_probe_name(&self) -> &'static str {
        self.inner.host_memory_probe_name()
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
        scratch: crate::target::shared::abi::Reg,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner.emit_set_nonblocking(
            fd_offset,
            flags_offset,
            scratch,
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
    fn emit_stack_limit(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        stack: crate::codegen::engine::types::ThreadStack,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.tick()?;
        self.inner
            .emit_stack_limit(from, platform_imports, stack, instructions, relocations)
    }
    fn app_mode_data_objects(
        &self,
        project_name: &str,
        debug_hooks: bool,
        uses_mouse: bool,
        uses_canvas: bool,
    ) -> Vec<CodeDataObject> {
        self.inner
            .app_mode_data_objects(project_name, debug_hooks, uses_mouse, uses_canvas)
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
