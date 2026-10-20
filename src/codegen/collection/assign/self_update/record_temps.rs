//! plan-176-Q: record-temp scratch (bug-766 B) — copying a self-updated record into scratch.
//! The arm table lives in `collection/assign/self_update/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// The record-temp scratch (bug-766 B).
// ---------------------------------------------------------------------------

/// bug-766 B: the `WITH` items of `x = set(x, k, WITH …)` / `x = append(x, WITH …)`
/// statements in `ops` (recursively) — the items the self-update arms build in
/// the record-temp scratch (`CodeBuilder::lower_self_update_item`).
fn self_update_with_items<'o>(ops: &'o [NirOp], out: &mut Vec<&'o ParameterType>) {
    for op in ops {
        match op {
            NirOp::Assign { name, value } => {
                if let (Some(NirValue::WithUpdate { type_, .. }), Some((_, args))) = (
                    crate::codegen::engine::analysis::borrow_get::copied_item(value),
                    self_update_call_parts(value),
                ) {
                    if matches!(args.first(), Some(NirValue::Local(arg0)) if arg0 == name) {
                        out.push(type_);
                    }
                }
            }
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                self_update_with_items(then_body, out);
                self_update_with_items(else_body, out);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    self_update_with_items(&case.body, out);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => self_update_with_items(body, out),
            _ => {}
        }
    }
}

impl CodeBuilder<'_> {
    /// bug-766 B: the byte size of every value of `type_` when it is a record of
    /// compile-time fixed size with a single-`arena_free` drop — each field a
    /// scalar or a fixed-size inlined record (`record_field_is_inlined_fixed`),
    /// laid out as `8 × fields` slots followed by each inlined field's sub-block
    /// (`emit_build_inlined_record`). `None` for any other type.
    pub(crate) fn fixed_record_size(&self, type_: &ParameterType) -> Option<usize> {
        fn size(builder: &CodeBuilder<'_>, type_: &ParameterType) -> Option<usize> {
            let fields = builder.type_model.record_fields.get(type_)?;
            let mut total = 8 * fields.len();
            for (_, field) in fields {
                if builder.record_field_is_inlined(field) {
                    total += size(builder, field)?;
                }
            }
            Some(total)
        }
        if !self.record_field_is_inlined_fixed(type_)
            || !self.is_freeable_flat_value(type_)
            || *type_ == ParameterType::String
        {
            return None;
        }
        size(self, type_).filter(|total| *total > 0)
    }

    /// bug-766 B: give each scratch-copy `get` binding (`BorrowGets::scratch`) of a
    /// fixed-size record type its region. Offsets follow the name order, so the
    /// layout does not depend on hash order.
    pub(crate) fn install_scratch_copies(
        &mut self,
        scratch: std::collections::HashMap<String, ParameterType>,
    ) {
        let mut names: Vec<(String, ParameterType)> = scratch
            .into_iter()
            .filter(|(name, _)| !self.promotable_vector_locals.contains(name))
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, type_) in names {
            let Some(size) = self.fixed_record_size(&type_) else {
                continue;
            };
            let offset = self.record_temps.size;
            self.record_temps.size += size;
            self.record_temps.bindings.insert(name, (offset, size));
        }
    }

    /// bug-766 B: after [`Self::install_scratch_copies`], give the `WITH` items of
    /// the function's `set`/`append` self-updates one shared region (the arm copies
    /// its item before it lowers anything else, so no two are live at once), and
    /// the function a record-temp scratch slot when any region exists. The slot is
    /// a function-level owned `List OF Integer`, zeroed at entry and freed at
    /// every exit, exactly as the self-update scratch is.
    pub(crate) fn install_record_temps(&mut self, ops: &[NirOp]) {
        let mut items = Vec::new();
        self_update_with_items(ops, &mut items);
        let item_size = items
            .into_iter()
            .filter_map(|type_| self.fixed_record_size(type_))
            .max();
        if let Some(item_size) = item_size {
            self.record_temps.with_item = Some((self.record_temps.size, item_size));
            self.record_temps.size += item_size;
        }
        if self.record_temps.size == 0 {
            return;
        }
        let slot = self.allocate_stack_object("rec_scratch", 8);
        self.record_temps.slot = Some(slot);
        self.active_cleanups
            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                type_: ParameterType::list_of(ParameterType::Integer),
                stack_offset: slot,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            }));
        self.owned_value_slots.push(slot);
    }

    /// bug-766 B: the address of the record-temp region at `offset`, reserving the
    /// function's record-temp scratch first (one allocation per activation: every
    /// reservation asks for the same `size`, so the block never grows or moves).
    pub(crate) fn emit_record_temp_region(
        &mut self,
        offset: usize,
    ) -> Result<crate::codegen::engine::operand::VirtualRegister, String> {
        let slot = self
            .record_temps
            .slot
            .ok_or("native record temp requested in a function without a record-temp scratch")?;
        let need = self.temporary_vreg();
        let need_slot = self.allocate_stack_object("rec_scratch_need", 8);
        self.emit(abi::move_immediate(
            &need,
            "Integer",
            &self.record_temps.size.to_string(),
        ));
        self.emit(abi::store_u64(&need, abi::stack_pointer(), need_slot));
        let data_slot = self.emit_reserve_scratch_block(slot, None, need_slot)?;
        let region = self.allocate_register();
        self.emit(abi::load_u64(&region, abi::stack_pointer(), data_slot));
        if offset != 0 {
            self.emit(abi::add_immediate(&region, &region, offset));
        }
        Ok(region)
    }

    /// bug-766 B: copy the `size` bytes (a multiple of 8) at `src` to `dst`, one
    /// word at a time. Nothing here calls out.
    pub(crate) fn emit_copy_record_words(
        &mut self,
        dst: impl Into<abi::Reg>,
        src: impl Into<abi::Reg>,
        size: usize,
    ) {
        let (dst, src) = (dst.into(), src.into());
        let word = self.allocate_register();
        for offset in (0..size).step_by(8) {
            self.emit(abi::load_u64(&word, &src, offset));
            self.emit(abi::store_u64(&word, &dst, offset));
        }
    }
}
