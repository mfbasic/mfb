//! plan-176-Q: the host-path arms — the app resource prefix and rejecting `.`/`..` components.
//! The arm tables and the shadow/regrow helpers live in
//! `collection/assign/string_self_update/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// plan-146-D: the `os::appResourcePath` prefix — `<executable dir>[/<suffix>]/` —
    /// written into a frame buffer once per call of the function holding the
    /// self-update, and its byte length returned in a slot.
    ///
    /// The copying lowering acquires the executable path from the host on every
    /// call (`os/func_app_resource_path.rs:lower_app_resource_path`); an arm may not (that
    /// is an allocation per statement), and a running process's own executable path
    /// cannot change, so the arm asks `os::executablePath()` once and caches the
    /// block it returns in `string_app_resource_base` (freed by the function's scope
    /// drop, `prescan_string_app_resource_base`). The prefix is then the same
    /// computation the helper does: strip `strip` components off the end of that
    /// path, append `/`, and — in an app build — the mode's resource suffix and a
    /// second `/` (`os/gen_paths.rs:resource_base_offset`). Those suffix bytes are
    /// compile-time constants, written as immediates rather than a data object.
    ///
    /// Returns `(prefix_ptr_slot, prefix_len_slot)`.
    pub(super) fn emit_app_resource_prefix(&mut self) -> Result<(usize, usize), String> {
        // In a lambda the cache is the CREATOR's, reached through the closure
        // environment (a lambda's own would be re-acquired on every element). The
        // holder address is spilled once, before any call: `%closure_env` is a
        // call-boundary token, not a pinned register.
        let holder_slot = match self.string_app_resource_base_env {
            Some(index) => {
                let slot = self.allocate_stack_object("str_resource_base_owner", 8);
                let holder = self.allocate_register();
                self.emit(abi::load_u64(
                    &holder,
                    CLOSURE_ENV_REGISTER,
                    closure_env_word_offset(index),
                ));
                self.emit(abi::store_u64(&holder, abi::stack_pointer(), slot));
                Some(slot)
            }
            None => None,
        };
        let slot = match holder_slot {
            // A working copy of the creator's cached block; stored back below.
            Some(holder_slot) => {
                let slot = self.allocate_stack_object("str_resource_base_copy", 8);
                let holder = self.allocate_register();
                let block = self.allocate_register();
                self.emit(abi::load_u64(&holder, abi::stack_pointer(), holder_slot));
                self.emit(abi::load_u64(&block, &holder, 0));
                self.emit(abi::store_u64(&block, abi::stack_pointer(), slot));
                slot
            }
            None => self
                .string_app_resource_base
                .ok_or("native os.appResourcePath self-update in a function with no base slot")?,
        };
        let have = self.label("inplace_str_respath_have");
        let block = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&block, "0"));
        self.emit(abi::branch_ne(&have));
        let helper = crate::target::shared::runtime::catalog::spec_for_call("os.executablePath")
            .ok_or("os.executablePath has no runtime helper spec")?
            .helper;
        let base = self.lower_runtime_helper_call(helper, "os.executablePath", &[], false)?;
        // The helper's result is a fresh block the statement scope would free at the
        // end of this statement; the cache slot takes it over instead, and the
        // function's scope drop frees it (`prescan_string_app_resource_base`). Without
        // this claim the cached pointer dangled from the next statement on — the
        // third lambda call read freed memory and died.
        self.claim_pending_temp(&base);
        self.emit(abi::store_u64(&base.location, abi::stack_pointer(), slot));
        if let Some(holder_slot) = holder_slot {
            // Publish it to the creator, which owns and frees it.
            let holder = self.allocate_register();
            let block = self.allocate_register();
            self.emit(abi::load_u64(&block, abi::stack_pointer(), slot));
            self.emit(abi::load_u64(&holder, abi::stack_pointer(), holder_slot));
            self.emit(abi::store_u64(&block, &holder, 0));
        }
        self.emit(abi::label(&have));

        let module_name = self.module_name.clone();
        let (strip, suffix) =
            crate::codegen::builtins::os::resource_base_offset(self.build_mode, &module_name);
        // The prefix buffer: the executable path's directory part, plus `/`, plus
        // the suffix and a second `/` in an app build.
        let extra = if suffix.is_empty() {
            1
        } else {
            suffix.len() + 2
        };
        let ptr_slot = self.allocate_stack_object("inplace_str_respath_prefix_ptr", 8);
        let len_slot = self.allocate_stack_object("inplace_str_respath_prefix_len", 8);
        let scratch_need = self.allocate_stack_object("inplace_str_respath_need", 8);

        let path = self.temporary_vreg();
        let path_len = self.temporary_vreg();
        let bytes = self.temporary_vreg();
        let scan = self.temporary_vreg();
        let left = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let need = self.temporary_vreg();
        let scan_loop = self.label("inplace_str_respath_scan");
        let scan_hit = self.label("inplace_str_respath_scan_hit");
        let prefix_ready = self.label("inplace_str_respath_prefix_ready");
        let fail = self.label("inplace_str_respath_fail");

        self.emit(abi::load_u64(&path, abi::stack_pointer(), slot));
        self.emit(abi::load_u64(&path_len, &path, 0));
        self.emit(abi::add_immediate(&bytes, &path, 8));
        // Scan back for the `strip`-th separator. The executable path uses the
        // host's separator: `/` everywhere but Windows, where it is `\`.
        let separator = if self.platform.target().starts_with("windows") {
            "92"
        } else {
            "47"
        };
        self.emit(abi::move_register(&scan, &path_len));
        self.emit(abi::move_immediate(&left, "Integer", &strip.to_string()));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_immediate(&scan, "0"));
        self.emit(abi::branch_eq(&fail));
        self.emit(abi::subtract_immediate(&scan, &scan, 1));
        self.emit(abi::add_registers(&byte, &bytes, &scan));
        self.emit(abi::load_u8(&byte, &byte, 0));
        self.emit(abi::compare_immediate(&byte, separator));
        self.emit(abi::branch_eq(&scan_hit));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&scan_hit));
        self.emit(abi::subtract_immediate(&left, &left, 1));
        self.emit(abi::compare_immediate(&left, "0"));
        self.emit(abi::branch_eq(&prefix_ready));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&fail));
        self.raise_error("os.appResourcePath", "ErrUnsupported")?;
        self.emit(abi::label(&prefix_ready));
        // The prefix is `scan` directory bytes plus `extra` joined bytes; build it
        // in the function's self-update scratch, which is reused across statements.
        self.emit(abi::add_immediate(&need, &scan, extra));
        self.emit(abi::store_u64(&need, abi::stack_pointer(), len_slot));
        self.emit(abi::store_u64(&need, abi::stack_pointer(), scratch_need));
        let dest = self.emit_reserve_self_update_scratch(scratch_need)?;
        let out = self.temporary_vreg();
        let src = self.temporary_vreg();
        let count = self.temporary_vreg();
        self.emit(abi::load_u64(&out, abi::stack_pointer(), dest));
        self.emit(abi::store_u64(&out, abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(&path, abi::stack_pointer(), slot));
        self.emit(abi::add_immediate(&src, &path, 8));
        self.emit(abi::load_u64(&count, abi::stack_pointer(), len_slot));
        self.emit(abi::subtract_immediate(&count, &count, extra));
        self.emit_copy_bytes(&out, &src, &count, "inplace_str_respath_dir");
        // `out` now points just past the directory bytes: write `/`, the suffix and
        // its `/` as immediates (a compile-time constant, so no data object).
        let mut joined = String::from("/");
        if !suffix.is_empty() {
            joined.push_str(&suffix);
            joined.push('/');
        }
        let literal = self.temporary_vreg();
        for (i, b) in joined.bytes().enumerate() {
            self.emit(abi::move_immediate(&literal, "Byte", &b.to_string()));
            self.emit(abi::store_u8(&literal, &out, i));
        }
        Ok((ptr_slot, len_slot))
    }

    /// plan-146-D: `os::appResourcePath` rejects a relative path with a `.` or `..`
    /// component (`os/gen_paths.rs:emit_reject_dot_component`, raised as
    /// `ErrInvalidPath` before anything is built). The arm checks the binding's own
    /// bytes the same way, before it writes one.
    pub(super) fn emit_reject_dot_components(
        &mut self,
        value_slot: usize,
        member: &str,
    ) -> Result<(), String> {
        let ptr = self.temporary_vreg();
        let len = self.temporary_vreg();
        let bytes = self.temporary_vreg();
        let index = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let comp_len = self.temporary_vreg();
        let all_dots = self.temporary_vreg();
        let walk = self.label("inplace_str_respath_walk");
        let boundary = self.label("inplace_str_respath_boundary");
        let not_boundary = self.label("inplace_str_respath_char");
        let next = self.label("inplace_str_respath_next");
        let not_dot = self.label("inplace_str_respath_not_dot");
        let end = self.label("inplace_str_respath_end");
        let bad = self.label("inplace_str_respath_bad");
        let component_ok = self.label("inplace_str_respath_component_ok");
        let done = self.label("inplace_str_respath_validated");

        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&len, &ptr, 0));
        self.emit(abi::add_immediate(&bytes, &ptr, 8));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&comp_len, "Integer", "0"));
        self.emit(abi::move_immediate(&all_dots, "Integer", "1"));
        self.emit(abi::label(&walk));
        self.emit(abi::compare_registers(&index, &len));
        self.emit(abi::branch_ge(&end));
        self.emit(abi::add_registers(&byte, &bytes, &index));
        self.emit(abi::load_u8(&byte, &byte, 0));
        // `/` is a component boundary on every target and `\` on Windows too —
        // exactly the helper's set (`gen_host_paths.rs:emit_validate_relative`).
        // Treating `\` as a boundary on POSIX as well made the arm raise
        // `ErrInvalidPath` for `a\..\b`, which the copying call accepts there
        // (plan-157-B Correction B-1): the arm must agree with the helper exactly.
        self.emit(abi::compare_immediate(&byte, "47"));
        self.emit(abi::branch_eq(&boundary));
        if self.platform.target().starts_with("windows") {
            self.emit(abi::compare_immediate(&byte, "92"));
            self.emit(abi::branch_eq(&boundary));
        }
        self.emit(abi::branch(&not_boundary));
        self.emit(abi::label(&boundary));
        self.emit_reject_dot_component_check(&comp_len, &all_dots, &bad, &component_ok);
        self.emit(abi::label(&component_ok));
        self.emit(abi::move_immediate(&comp_len, "Integer", "0"));
        self.emit(abi::move_immediate(&all_dots, "Integer", "1"));
        self.emit(abi::branch(&next));
        self.emit(abi::label(&not_boundary));
        self.emit(abi::add_immediate(&comp_len, &comp_len, 1));
        self.emit(abi::compare_immediate(&byte, "46"));
        self.emit(abi::branch_eq(&not_dot));
        self.emit(abi::move_immediate(&all_dots, "Integer", "0"));
        self.emit(abi::label(&not_dot));
        self.emit(abi::branch(&next));
        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&walk));
        self.emit(abi::label(&end));
        self.emit_reject_dot_component_check(&comp_len, &all_dots, &bad, &done);
        self.emit(abi::label(&bad));
        self.raise_error(member, "ErrInvalidPath")?;
        self.emit(abi::label(&done));
        Ok(())
    }

    /// One component's verdict: an all-dots component of length 1 (`.`) or 2 (`..`)
    /// branches to `bad`, anything else to `ok`.
    fn emit_reject_dot_component_check(
        &mut self,
        comp_len: &VirtualRegister,
        all_dots: &VirtualRegister,
        bad: &str,
        ok: &str,
    ) {
        self.emit(abi::compare_immediate(all_dots, "0"));
        self.emit(abi::branch_eq(ok));
        self.emit(abi::compare_immediate(comp_len, "1"));
        self.emit(abi::branch_eq(bad));
        self.emit(abi::compare_immediate(comp_len, "2"));
        self.emit(abi::branch_eq(bad));
        self.emit(abi::branch(ok));
    }
}
