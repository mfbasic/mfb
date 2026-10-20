//! plan-176-Q: same-length rewrites — the out scratch and ASCII case mapping. The arm tables
//! and the shadow/regrow helpers live in `collection/assign/string_self_update/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// plan-146-E: reserve `need_slot` bytes of the self-update scratch for a
    /// rewrite's output and return the slot holding a pointer to them.
    pub(crate) fn emit_string_out_scratch(&mut self, need_slot: usize) -> Result<usize, String> {
        let data = self.emit_reserve_self_update_scratch(need_slot)?;
        let ptr = self.temporary_vreg();
        let slot = self.allocate_stack_object("inplace_str_out_ptr", 8);
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), data));
        self.emit(abi::store_u64(&ptr, abi::stack_pointer(), slot));
        Ok(slot)
    }

    /// plan-146-E: copy a rewrite's `new_len_slot` scratch bytes at `out_slot` into
    /// the binding's own block, growing it through the shadow first, and store the
    /// new length. The reserve allocates before anything is written, so an
    /// `ErrOutOfMemory` leaves the binding unchanged.
    pub(crate) fn emit_string_out_publish(
        &mut self,
        block_slot: usize,
        shadow_slot: usize,
        out_slot: usize,
        new_len_slot: usize,
    ) -> Result<(), String> {
        self.emit_string_reserve(block_slot, shadow_slot, new_len_slot)?;
        let dst = self.temporary_vreg();
        let src = self.temporary_vreg();
        let count = self.temporary_vreg();
        self.emit(abi::load_u64(&dst, abi::stack_pointer(), block_slot));
        self.emit(abi::add_immediate(&dst, &dst, 8));
        self.emit(abi::load_u64(&src, abi::stack_pointer(), out_slot));
        self.emit(abi::load_u64(&count, abi::stack_pointer(), new_len_slot));
        self.emit_copy_bytes(&dst, &src, &count, "inplace_str_rewrite_back");
        self.emit_string_set_len(block_slot, shadow_slot, new_len_slot);
        Ok(())
    }

    /// plan-146-E: `s = f(s, …)` for a builtin that rewrites `s`'s bytes
    /// ([`STRING_REWRITE_FNS`]). The result cannot be written over `s` as it is
    /// computed — the writer can overtake the reader (`ß` → `SS`, a longer
    /// replacement) — so the lowering builds it in the function's self-update
    /// scratch (reused across statements, so a loop allocates `O(log n)` times) and
    /// the arm copies it back, growing the block through the shadow if it must.
    /// The case maps and `normalizeNfc` take an all-ASCII shortcut first: mapping
    /// is same-length there, and ASCII is already NFC.
    pub(crate) fn try_inplace_string_rewrite_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let Some((target, _)) = self_update_call_parts(value) else {
            return Ok(false);
        };
        let Some(bare) = self_update_builtin(target) else {
            return Ok(false);
        };
        let Some((name, arity, kind)) = STRING_REWRITE_FNS
            .iter()
            .find(|(name, _, _)| *name == bare)
            .map(|(name, arity, kind)| (*name, arity.clone(), *kind))
        else {
            return Ok(false);
        };
        // The scratch contract: an arm that needs one checks for it in its gates,
        // before it emits anything (`emit_reserve_self_update_scratch`).
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some(args) = self.resolve_string_self_update(site, value, name, arity, true) else {
            return Ok(false);
        };
        let args: Vec<NirValue> = args.to_vec();

        let block_slot = site.dest.block_slot();
        let shadow = self
            .string_shadow_slot(site)?
            .ok_or("native String rewrite self-update lost its capacity shadow")?;
        let mut rest = Vec::new();
        for arg in &args[1..] {
            rest.push(self.lower_value(arg)?);
        }
        // The marker slot: its presence proves this arm fired (`ArmId::markers`).
        let marker = self.allocate_stack_object("inplace_str_rewrite", 8);
        let done = self.label("inplace_str_rewrite_done");
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
        self.emit(abi::store_u64(&block, abi::stack_pointer(), marker));
        let value_result = ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(block),
            text: String::new(),
        };

        match kind {
            RewriteKind::CaseMap(map) => {
                // All-ASCII: the mapping is one byte in, one byte out, so it runs
                // over the binding's own bytes with no scratch and no length change.
                self.emit_ascii_case_map_in_place(block_slot, map, &done)?;
                let (out, len_slot) =
                    crate::codegen::builtins::strings::gen_case_map::lower_strings_case_map_out(
                        self,
                        &value_result,
                        map,
                        StringOut::Scratch,
                    )?;
                let out_slot = self.spill_to_slot("inplace_str_rewrite_out", &out.location);
                self.emit_string_out_publish(block_slot, shadow.slot, out_slot, len_slot)?;
            }
            RewriteKind::NormalizeNfc => {
                // An all-ASCII string is already in NFC.
                self.emit_ascii_scan_branch(block_slot, &done);
                let (out, len_slot) =
                    crate::codegen::builtins::strings::func_normalize_nfc::lower_out(
                        self,
                        std::slice::from_ref(&value_result),
                        StringOut::Scratch,
                    )?;
                let out_slot = self.spill_to_slot("inplace_str_rewrite_out", &out.location);
                // `lower_out`'s scratch result is block-shaped: the bytes start at +8.
                let bytes = self.temporary_vreg();
                self.emit(abi::load_u64(&bytes, abi::stack_pointer(), out_slot));
                self.emit(abi::add_immediate(&bytes, &bytes, 8));
                self.emit(abi::store_u64(&bytes, abi::stack_pointer(), out_slot));
                self.emit_string_out_publish(block_slot, shadow.slot, out_slot, len_slot)?;
            }
            RewriteKind::Replace => {
                let (out, len_slot, unchanged_slot) = self.lower_replace_out(
                    &[args[0].clone(), args[1].clone(), args[2].clone()],
                    StringOut::Scratch,
                )?;
                let out_slot = self.spill_to_slot("inplace_str_rewrite_out", &out.location);
                // No match: the binding already holds the result.
                let unchanged = self.temporary_vreg();
                self.emit(abi::load_u64(
                    &unchanged,
                    abi::stack_pointer(),
                    unchanged_slot,
                ));
                self.emit(abi::compare_immediate(&unchanged, "0"));
                self.emit(abi::branch_ne(&done));
                self.emit_string_out_publish(block_slot, shadow.slot, out_slot, len_slot)?;
            }
            RewriteKind::PathNormalize => {
                let (out, len_slot) =
                    self.lower_fs_path_normalize_out(&value_result, StringOut::Scratch)?;
                let out_slot = self.spill_to_slot("inplace_str_rewrite_out", &out.location);
                // The normalized bytes are in a block-shaped scratch region.
                let bytes = self.temporary_vreg();
                self.emit(abi::load_u64(&bytes, abi::stack_pointer(), out_slot));
                self.emit(abi::add_immediate(&bytes, &bytes, 8));
                self.emit(abi::store_u64(&bytes, abi::stack_pointer(), out_slot));
                self.emit_string_out_publish(block_slot, shadow.slot, out_slot, len_slot)?;
            }
        }
        self.emit(abi::label(&done));
        self.publish_string_shadow(&shadow)?;
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// plan-146-E: branch to `done` when the block's bytes are all ASCII (the
    /// SWAR quick-check `lower_strings_case_map` and `func_normalize_nfc` use).
    fn emit_ascii_scan_branch(&mut self, block_slot: usize, done: &str) {
        let block = self.temporary_vreg();
        let len = self.temporary_vreg();
        let bytes = self.temporary_vreg();
        let index = self.temporary_vreg();
        let word = self.temporary_vreg();
        let mask = self.temporary_vreg();
        let left = self.temporary_vreg();
        let scan = self.label("inplace_str_ascii_scan");
        let scan_byte = self.label("inplace_str_ascii_scan_byte");
        let not_ascii = self.label("inplace_str_ascii_no");
        self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
        self.emit(abi::load_u64(&len, &block, 0));
        self.emit(abi::add_immediate(&bytes, &block, 8));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&mask, "Integer", "9259542123273814144"));
        self.emit(abi::label(&scan));
        self.emit(abi::subtract_registers(&left, &len, &index));
        self.emit(abi::compare_immediate(&left, "8"));
        self.emit(abi::branch_lo(&scan_byte));
        self.emit(abi::add_registers(&word, &bytes, &index));
        self.emit(abi::load_u64(&word, &word, 0));
        self.emit(abi::and_registers(&word, &word, &mask));
        self.emit(abi::compare_immediate(&word, "0"));
        self.emit(abi::branch_ne(&not_ascii));
        self.emit(abi::add_immediate(&index, &index, 8));
        self.emit(abi::branch(&scan));
        self.emit(abi::label(&scan_byte));
        self.emit(abi::compare_registers(&index, &len));
        self.emit(abi::branch_ge(done));
        self.emit(abi::add_registers(&word, &bytes, &index));
        self.emit(abi::load_u8(&word, &word, 0));
        self.emit(abi::compare_immediate(&word, "128"));
        self.emit(abi::branch_ge(&not_ascii));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&scan_byte));
        self.emit(abi::label(&not_ascii));
    }

    /// plan-146-E: map the block's bytes where they lie when they are all ASCII,
    /// then branch to `done`; falls through when any byte is not.
    fn emit_ascii_case_map_in_place(
        &mut self,
        block_slot: usize,
        map: crate::codegen::builtins::strings::UnicodeCaseMap,
        done: &str,
    ) -> Result<(), String> {
        let ascii = self.label("inplace_str_ascii_map");
        let slow = self.label("inplace_str_ascii_map_slow");
        self.emit_ascii_scan_branch(block_slot, &ascii);
        self.emit(abi::branch(&slow));
        self.emit(abi::label(&ascii));
        let block = self.temporary_vreg();
        let len = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let left = self.temporary_vreg();
        let loop_label = self.label("inplace_str_ascii_map_loop");
        let loop_done = self.label("inplace_str_ascii_map_done");
        self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
        self.emit(abi::load_u64(&len, &block, 0));
        self.emit(abi::add_immediate(&cursor, &block, 8));
        self.emit(abi::move_register(&left, &len));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&left, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        crate::codegen::builtins::strings::gen_case_map::emit_ascii_case_transform(
            self, map, &byte,
        );
        self.emit(abi::store_u8(&byte, &cursor, 0));
        self.emit(abi::add_immediate(&cursor, &cursor, 1));
        self.emit(abi::subtract_immediate(&left, &left, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));
        self.emit(abi::branch(done));
        self.emit(abi::label(&slow));
        Ok(())
    }
}
