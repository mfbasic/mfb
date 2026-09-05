//! Shared `#[cfg(test)]` codegen-platform stub for unit tests that lower a
//! single native helper and inspect the emitted instruction stream (e.g. the
//! bug-55 error-path resource-release regression guards).
//!
//! It reports a Linux/AArch64 identity (so the `tls`/`crypto` dispatchers take
//! the OpenSSL path) and lowers every libc/variadic call to a plain `bl` to the
//! named function — enough for the helper to lower and register-allocate. The
//! socket-constant accessors return their Linux values so the `net`/`tls`
//! helpers that consult them lower cleanly. The many file/terminal `emit_*`
//! hooks are `unimplemented!()` because the helpers under test never call them.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::target::shared::abi;
use std::collections::HashMap;
pub(crate) struct TestPlatform;

#[rustfmt::skip]
impl CodegenPlatform for TestPlatform {
    fn target(&self) -> &'static str { "linux_aarch64" }
    fn arch(&self) -> &'static str { "aarch64" }
    fn backend(&self) -> &'static dyn crate::codegen::engine::mir::Backend { &crate::arch::aarch64::backend::AARCH64_BACKEND }
    fn emit_apply_raw_mode(&self, _b: &str, _o: usize, _m: usize, _de: bool, _dc: bool, _i: &mut Vec<CodeInstruction>) { unimplemented!("TestPlatform::emit_apply_raw_mode") }
    fn emit_program_exit(&self, _from: &str, _instructions: &mut Vec<CodeInstruction>, _relocations: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_program_exit") }
    fn emit_write(&self, _from: &str, _pi: &HashMap<String, String>, i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> {
        // A plain `bl _write` leaving the request length in the return register
        // (pretend the whole buffer landed) is enough for a caller (the bug-410
        // `term::` present-write loop test) to lower and inspect its retry tail.
        i.push(abi::branch_link("_write"));
        i.push(abi::move_register(abi::return_register(), abi::string_length_register()));
        Ok(())
    }
    fn emit_poll_input(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_poll_input") }
    fn emit_is_terminal(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_is_terminal") }
    fn emit_terminal_size(&self, _from: &str, _pi: &HashMap<String, String>, i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> {
        // A plain `bl _ioctl` marker; the bug-410 present-loop test only inspects
        // the neutral instruction stream, so no real TIOCGWINSZ sequence is needed.
        i.push(abi::branch_link("_ioctl"));
        Ok(())
    }
    fn emit_path_exists(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_path_exists") }
    fn emit_path_stat(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_path_stat") }
    fn emit_stat_is_kind(&self, _so: usize, _ek: &str, _m: &str, _mk: &str, _e: &str, _f: &str, _mi: &str, _i: &mut Vec<CodeInstruction>) { unimplemented!("TestPlatform::emit_stat_is_kind") }
    fn emit_current_directory(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_current_directory") }
    fn emit_environ_pointer(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_environ_pointer") }
    fn emit_fs_path_operation(&self, _from: &str, _op: FsPathOperation, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_fs_path_operation") }
    fn emit_errno(&self, _from: &str, dst: Operand, _pi: &HashMap<String, String>, instructions: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> {
        // Leave a plausible errno value in `dst`; a plain move is enough for
        // helpers (e.g. the non-blocking connect timeout path) to lower and
        // register-allocate.
        instructions.push(abi::move_immediate(dst, "Integer", "0"));
        Ok(())
    }
    fn emit_external_call(&self, base: &str, _from: &str, _pi: &HashMap<String, String>, instructions: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> {
        // A plain `bl` to the named libc function is enough for the helper to
        // lower and register-allocate; the tests inspect the release blocks.
        instructions.push(abi::branch_link(&format!("_{base}")));
        Ok(())
    }
    fn emit_open_file(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_open_file") }
    fn emit_read_file(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_read_file") }
    fn emit_close_file(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_close_file") }
    fn emit_sync_file(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_sync_file") }
    fn emit_seek_file(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_seek_file") }
    fn emit_rename_path(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_rename_path") }
    fn emit_mkstemps(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_mkstemps") }
    fn emit_random_bytes(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_random_bytes") }
    fn emit_temp_directory(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_temp_directory") }
    fn emit_opendir(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_opendir") }
    fn emit_readdir(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_readdir") }
    fn emit_closedir(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_closedir") }
    fn emit_read_dir_entry(&self, _p: &str, _np: &str, _nl: &str, _b: &str, _s: &str, _i: &mut Vec<CodeInstruction>) { unimplemented!("TestPlatform::emit_read_dir_entry") }
    fn emit_realpath(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_realpath") }
    fn emit_arena_map(&self, _size_reg: &str, _instructions: &mut Vec<CodeInstruction>) -> Result<(), String> { unimplemented!("TestPlatform::emit_arena_map") }
    fn emit_arena_unmap(&self, _instructions: &mut Vec<CodeInstruction>) -> Result<(), String> { unimplemented!("TestPlatform::emit_arena_unmap") }
    fn addrinfo_addr_offset(&self) -> usize { 24 }
    fn sol_socket(&self) -> &'static str { "1" }
    fn so_reuseaddr(&self) -> &'static str { "2" }
    fn so_rcvtimeo(&self) -> &'static str { "20" }
    fn so_sndtimeo(&self) -> &'static str { "21" }
    // plan-110-A net::ping constants; this stub mirrors the Linux values, like the
    // socket options above.
    fn so_rcvbuf(&self) -> &'static str { "8" }
    fn ipproto_ip(&self) -> &'static str { "0" }
    fn ip_ttl(&self) -> &'static str { "2" }
    fn ip_recvttl(&self) -> &'static str { "12" }
    fn cmsg_ip_ttl_type(&self) -> &'static str { "2" }
    fn clock_monotonic(&self) -> &'static str { "1" }
    fn socket_would_block_code(&self) -> &'static str { "11" }
    fn socket_message_size_code(&self) -> &'static str { "90" }
    fn socket_in_progress_code(&self) -> &'static str { "115" }
    fn emit_set_nonblocking(&self, _fd: usize, _fl: usize, _from: &str, _pi: &HashMap<String, String>, instructions: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> {
        // The openssl non-blocking connect toggles the socket via this seam; a
        // plain `bl` lets the helper lower so the release-block tests can inspect it.
        instructions.push(abi::branch_link("_fcntl"));
        Ok(())
    }
    fn so_error(&self) -> &'static str { "4" }
    fn emit_variadic_external_call(&self, base: &str, _from: &str, _pi: &HashMap<String, String>, instructions: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> {
        instructions.push(abi::branch_link(&format!("_{base}")));
        Ok(())
    }
    fn emit_app_program_entry(&self, _spec: &AppEntrySpec, _pi: &HashMap<String, String>) -> Option<Result<Vec<CodeFunction>, String>> { None }
    fn emit_program_entry(&self, _spec: &ProgramEntrySpec<'_>, _pi: &HashMap<String, String>) -> Result<CodeFunction, String> { unimplemented!("TestPlatform::emit_program_entry") }
    fn emit_thread_trampoline(&self, _pi: &HashMap<String, String>, _uses_stdin: bool, _arena_init: crate::codegen::engine::types::ArenaInitSymbols) -> Result<CodeFunction, String> { unimplemented!("TestPlatform::emit_thread_trampoline") }
}

/// Whether a label with `name` appears in the instruction stream.
pub(crate) fn has_label(ins: &[CodeInstruction], name: &str) -> bool {
    ins.iter()
        .any(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(name))
}

// --- codegen-stream inspection (the per-file `tests_codegen.rs` suites) ------
//
// This vocabulary lives HERE rather than beside the suites that use it because
// `scripts/coverage-common.sh`'s IGNORE regex excludes `/tests/`, and every one
// of these helpers carries a diagnostic `panic!` for the "not found" case. Such
// an arm is by construction never taken on a green run, so a file full of them
// sits several points below the per-file floor no matter how well the code
// under test is covered — and the only ways out are to drop the diagnostics
// (turning a precise failure into an `unwrap`) or to weaken the assertions.
// Neither is a trade worth making, so the helpers sit outside the denominator
// and the suites keep both their messages and their coverage.

/// A lowered function's instruction stream, with the lookups a codegen
/// assertion needs.
pub(crate) struct Stream<'a> {
    pub(crate) name: &'a str,
    pub(crate) instructions: &'a [CodeInstruction],
}

impl<'a> Stream<'a> {
    pub(crate) fn of(function: &'a CodeFunction) -> Self {
        Self {
            name: &function.name,
            instructions: &function.instructions,
        }
    }

    /// A named operand field's rendered value, or the empty string.
    ///
    /// Empty-for-absent rather than `Option` because every caller compares
    /// against a concrete register/offset/symbol, and no field renders empty.
    pub(crate) fn field(instruction: &CodeInstruction, name: &str) -> String {
        instruction.get(name).unwrap_or_default()
    }

    /// Every label this function defines, as `(index, name)`.
    pub(crate) fn labels(&self) -> Vec<(usize, String)> {
        self.instructions
            .iter()
            .enumerate()
            .filter(|(_, i)| i.op == CodeOp::Label)
            .filter_map(|(n, i)| i.get("name").map(|name| (n, name)))
            .collect()
    }

    /// The index of the first instruction satisfying `predicate`.
    ///
    /// `what` is the human description that appears if there is none — "the
    /// arena allocation", not a rendered predicate.
    pub(crate) fn index_of(
        &self,
        what: &str,
        predicate: impl Fn(&CodeInstruction) -> bool,
    ) -> usize {
        self.instructions
            .iter()
            .position(predicate)
            .unwrap_or_else(|| panic!("{}: no {what} in the emitted stream", self.name))
    }

    /// The index of the first instruction at or after `from` satisfying
    /// `predicate`.
    pub(crate) fn index_after(
        &self,
        from: usize,
        what: &str,
        predicate: impl Fn(&CodeInstruction) -> bool,
    ) -> usize {
        self.instructions[from..]
            .iter()
            .position(predicate)
            .map(|n| from + n)
            .unwrap_or_else(|| panic!("{}: no {what} at or after index {from}", self.name))
    }

    /// The index of the label named `name`.
    pub(crate) fn label_at(&self, name: &str) -> usize {
        self.index_of(&format!("label `{name}`"), |i| {
            i.op == CodeOp::Label && i.get("name").as_deref() == Some(name)
        })
    }

    /// The first label whose name begins with `prefix`.
    ///
    /// Labels carry `CodeBuilder::label`'s per-function counter, which renumbers
    /// whenever an earlier label is added or removed — so a suite that spells a
    /// full label name goes red for an edit that changed nothing it tests.
    pub(crate) fn label_starting(&self, prefix: &str) -> String {
        self.labels()
            .into_iter()
            .map(|(_, name)| name)
            .find(|name| name.starts_with(prefix))
            .unwrap_or_else(|| panic!("{}: no `{prefix}*` label", self.name))
    }

    /// The call targets between two instruction indices (`<indirect>` for a
    /// call through a register).
    pub(crate) fn calls_between(&self, from: usize, to: usize) -> Vec<String> {
        self.instructions[from..to]
            .iter()
            .filter(|i| i.op == CodeOp::BranchLink || i.op == CodeOp::BranchLinkRegister)
            .map(|i| i.get("target").unwrap_or_else(|| "<indirect>".to_string()))
            .collect()
    }

    /// Whether anything in this function branches to `label`.
    pub(crate) fn branches_to(&self, label: &str) -> bool {
        self.instructions
            .iter()
            .any(|i| i.get("target").as_deref() == Some(label))
    }

    /// The condition under which control reaches `label`, as `"eq"`/`"ne"`/....
    ///
    /// Arch-neutral on purpose. x86-64 and AArch64 keep the comparison and the
    /// branch apart (`cmp` then `b.eq`), while riscv64 has no flags register and
    /// fuses them into one `RvBr lhs=a0 rhs=zero cond=eq`. A suite that asserts
    /// "this branch is a `BranchEq`" therefore passes on four backends and cannot
    /// even find the instruction on the fifth — which is how a per-target rule
    /// ends up tested on four targets and assumed on the last one.
    pub(crate) fn branch_condition_to(&self, label: &str) -> String {
        let at = self.index_of(&format!("branch to `{label}`"), |i| {
            i.get("target").as_deref() == Some(label)
        });
        let instruction = &self.instructions[at];
        match instruction.op {
            CodeOp::BranchEq => "eq".to_string(),
            CodeOp::BranchNe => "ne".to_string(),
            CodeOp::BranchLt => "lt".to_string(),
            CodeOp::BranchLe => "le".to_string(),
            CodeOp::BranchGt => "gt".to_string(),
            CodeOp::BranchGe => "ge".to_string(),
            CodeOp::Branch => "always".to_string(),
            // riscv64's fused compare-and-branch carries the condition as a field.
            _ => Self::field(instruction, "cond"),
        }
    }

    /// Every `AddImm <dst>, <stack pointer>, <imm>` in the stream, as its
    /// immediate, in order.
    ///
    /// Frame offsets are NOT the constants the emitter wrote: `finalize_frame`
    /// shifts every sp-relative access up past the callee-saved area, and by a
    /// different amount per backend. A rule about a struct field's offset has to
    /// be stated as a DIFFERENCE between two of these, never as an absolute.
    pub(crate) fn stack_offsets(&self) -> Vec<i64> {
        let sp = ["sp", "rsp"];
        self.instructions
            .iter()
            .filter(|i| i.op == CodeOp::AddImm && sp.contains(&Self::field(i, "src").as_str()))
            .filter_map(|i| Self::field(i, "imm").parse().ok())
            .collect()
    }
}

// --- a `CodeBuilder` + `AbiCtx` for a single `abi_inline` emitter -----------
//
// The whole-program harness (`testutil::code_for_src_on`) is the right tool when
// the question is "what does the compiler emit for this program". It cannot ask
// "what does this emitter do when the platform declares no import for it", or
// "what does the Windows arm emit", without a program and a plan that produce
// that situation — and for a `?` propagation arm no program does, because the
// plan force-declares whatever the body needs.
//
// `BuilderHarness` owns the ~10 tables `CodeBuilder::for_synthetic_function`
// borrows, so a test can build one in two lines and drive an emitter directly.
// It lives here, outside the coverage denominator, for the same reason `Stream`
// does.

/// Owned backing storage for a test [`CodeBuilder`].
///
/// Every table starts empty; add what the emitter under test reads. The lifetime
/// is the harness's own — `builder()` borrows `&'a self`, so the harness must
/// outlive the builder (bind it to a `let` before calling).
pub(crate) struct BuilderHarness<'a> {
    pub(crate) function_symbols: HashMap<String, String>,
    pub(crate) functions: HashMap<String, &'a crate::target::shared::nir::NirFunction>,
    pub(crate) package_return_types: HashMap<String, crate::types::ParameterType>,
    /// Symbol -> library, exactly as a `NativePlan`'s platform imports render.
    /// **Leave it empty to prove a body fails closed** on an undeclared import.
    pub(crate) platform_imports: HashMap<String, String>,
    pub(crate) globals: HashMap<String, crate::codegen::engine::builder::GlobalValue>,
    pub(crate) string_symbols: HashMap<String, String>,
    pub(crate) build_mode: crate::target::NativeBuildMode,
    pub(crate) type_model: crate::codegen::engine::builder::TypeModel,
}

impl Default for BuilderHarness<'_> {
    fn default() -> Self {
        Self {
            function_symbols: HashMap::new(),
            functions: HashMap::new(),
            package_return_types: HashMap::new(),
            platform_imports: HashMap::new(),
            globals: HashMap::new(),
            string_symbols: HashMap::new(),
            build_mode: crate::target::NativeBuildMode::Console,
            type_model: crate::codegen::engine::builder::TypeModel::empty(),
        }
    }
}

impl<'a> BuilderHarness<'a> {
    /// A `CodeBuilder` for a synthesized function named `symbol`, lowering for
    /// `platform`.
    ///
    /// Installs the active MIR backend as every real lowering entry point does,
    /// so register allocation dispatches to the same backend the emitter targets.
    pub(crate) fn builder(
        &'a self,
        symbol: &str,
        platform: &'a dyn CodegenPlatform,
    ) -> crate::codegen::engine::builder::CodeBuilder<'a> {
        crate::codegen::engine::mir::set_backend(platform.backend());
        crate::codegen::engine::builder::CodeBuilder::for_synthetic_function(
            symbol,
            &self.function_symbols,
            &self.functions,
            &self.package_return_types,
            &self.platform_imports,
            platform,
            self.build_mode,
            &self.globals,
            &self.string_symbols,
            self.type_model.clone(),
        )
    }

    /// The `AbiCtx` an `abi_inline` body receives — the inline path's own
    /// defaults (empty `module_name`/`call`, no arena offsets, no globals, no
    /// RNG), so a test is looking at the same context production hands it.
    pub(crate) fn abi_ctx(
        &'a self,
        platform: &'a dyn CodegenPlatform,
    ) -> crate::codegen::registry::AbiCtx<'a> {
        crate::codegen::registry::AbiCtx {
            platform_imports: &self.platform_imports,
            platform,
            build_mode: self.build_mode,
            module_name: "",
            call: "",
            term_state_offset: None,
            presentation_mode_offset: None,
            arena_global_slots: 0,
            uses_rng: false,
        }
    }
}
