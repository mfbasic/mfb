//! plan-176-Q: in-place map merge and `mapValues`. The shared map/scratch emitters live in
//! `collection/assign/builder_inplace_setmap/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// `m = collections::merge(m, n, preferB)`.
    pub(crate) fn try_inplace_merge_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some(target) = self.resolve_self_update(site, value, "merge", 3) else {
            return Ok(false);
        };
        let map_type = target.collection_type.clone();
        let Some((key_type, value_type)) =
            typed_map_type_parts(&map_type).map(|(k, v)| (k.clone(), v.clone()))
        else {
            return Ok(false);
        };
        // plan-145-E: a new key grows the map. `preferB` is lowered into a slot
        // before any grow, so only the other map can be a stale view.
        if !self.field_realloc_admitted(site, &target.args[1..2]) {
            return Ok(false);
        }
        let opened = self.open_inplace_dest(&target.dest)?;
        let alias = site.is_self(&target.args[1]);
        // Source order: the other map, then preferB.
        let other = if alias {
            None
        } else {
            Some(self.lower_other_collection(&target.args[1], &map_type, "merge")?)
        };
        let prefer = self.lower_value(&target.args[2])?;
        if prefer.type_ != ParameterType::Boolean {
            return Err(format!(
                "native in-place merge preferB must be Boolean, got {}",
                prefer.type_
            ));
        }
        let prefer_slot = self.allocate_stack_object("inplace_merge_prefer", 8);
        self.emit(abi::store_u64(
            &prefer.location,
            abi::stack_pointer(),
            prefer_slot,
        ));
        let dest = self.inplace_collection_slot(&opened)?;
        // `merge(m, m, p)` is `m` whatever `p` is.
        if let Some(other) = other {
            let grow = self.inplace_inline_grow(&opened)?;
            let mut sizes = Vec::new();
            let key_size = self.emit_borrow_area_size(other, Payload::Key, &key_type);
            let value_size = self.emit_borrow_area_size(other, Payload::Value, &value_type);
            sizes.extend(key_size);
            sizes.extend(value_size);
            let (key_area, value_area) = if sizes.is_empty() {
                (None, None)
            } else {
                let areas = self.emit_scratch_areas(&sizes)?;
                let mut areas = areas.into_iter();
                let key_area = key_size.and_then(|_| areas.next());
                let value_area = value_size.and_then(|_| areas.next());
                (key_area, value_area)
            };
            let (entries, bytes) = self.emit_map_batch_room(other, None);
            self.emit_map_reserve(dest, entries, bytes, &map_type, grow)?;
            let (index_slot, top, done) = self.emit_map_loop_head(other, "inplace_merge");
            let key_slot = self.emit_map_entry_borrowed(
                other,
                index_slot,
                Payload::Key,
                &key_type,
                key_area,
                "inplace_merge_key",
            )?;
            let set = self.label("inplace_merge_set");
            let skip = self.label("inplace_merge_skip");
            let prefer = self.temporary_vreg();
            self.emit(abi::load_u64(&prefer, abi::stack_pointer(), prefer_slot));
            self.emit(abi::compare_immediate(&prefer, "0"));
            self.emit(abi::branch_ne(&set));
            let has = self.emit_key_membership(
                dest,
                key_slot,
                &key_type,
                "inplace_merge_has",
                &map_type,
            )?;
            self.emit(abi::compare_immediate(&has.location, "0"));
            self.emit(abi::branch_ne(&skip));
            self.emit(abi::label(&set));
            let value_slot = self.emit_map_entry_borrowed(
                other,
                index_slot,
                Payload::Value,
                &value_type,
                value_area,
                "inplace_merge_value",
            )?;
            self.lower_map_set_in_place(
                dest,
                key_slot,
                value_slot,
                &map_type,
                &key_type,
                &value_type,
                grow,
            )?;
            self.emit(abi::label(&skip));
            self.emit_map_loop_next(index_slot, &top, &done);
        }
        self.close_field_dest(&target.dest, &opened)?;
        self.clear_self_update_constant(site.name);
        Ok(true)
    }

    /// `m = collections::mapValues(m, f)`, `f` returning `m`'s value type.
    pub(crate) fn try_inplace_map_values_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some(target) = self.resolve_self_update(site, value, "mapValues", 2) else {
            return Ok(false);
        };
        let map_type = target.collection_type.clone();
        let Some(value_type) = typed_map_type_parts(&map_type).map(|(_, v)| v.clone()) else {
            return Ok(false);
        };
        // plan-145-D/E: a variable-width value's results are written to room the
        // map reserves, which may reallocate — at a field, through `InlineGrow` at
        // the last inlined field. The callable operand is no view into the owner.
        if !value_type_is_fixed(&value_type) && !self.field_realloc_admitted(site, &[]) {
            return Ok(false);
        }
        let opened = self.open_inplace_dest(&target.dest)?;
        let action = self.lower_value(&target.args[1])?;
        let output_type = typed_callable_return_type(&action.type_)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "native in-place mapValues action must be a function, got {}",
                    action.type_
                )
            })?;
        if output_type != value_type {
            return Err(format!(
                "native in-place mapValues of {map_type} needs a FUNC returning {value_type}, \
                 got {output_type}"
            ));
        }
        self.require_direct_callable("mapValues", &action)?;
        let action_slot = self.allocate_stack_object("inplace_mapvalues_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let dest = self.inplace_collection_slot(&opened)?;
        // bug-760 B1: `f` runs for every entry — drop the tombstones first
        // (bug-768: a field's map may hold them too).
        self.emit_map_settle_slot(dest);
        let is_string = value_type == ParameterType::String;
        // Results: one word per entry.
        let need_slot = self.allocate_stack_object("inplace_mapvalues_need", 8);
        let base = self.temporary_vreg();
        let need = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), dest));
        self.emit(abi::load_u64(&need, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::shift_left_immediate(&need, &need, 3));
        self.emit(abi::store_u64(&need, abi::stack_pointer(), need_slot));
        let results_slot = self.emit_reserve_self_update_scratch(need_slot)?;
        let result_slot = self.allocate_stack_object("inplace_mapvalues_result", 8);

        // Pass 1: results[i] = f(value_i); a failure frees what was produced.
        let (index_slot, top, done) = self.emit_map_loop_head(dest, "inplace_mapvalues");
        let item_slot = self.emit_map_entry_payload(
            dest,
            index_slot,
            Payload::Value,
            &value_type,
            "inplace_mapvalues_item",
        )?;
        let item = self.temporary_vreg();
        self.emit(abi::load_u64(&item, abi::stack_pointer(), item_slot));
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        let callee = self.temporary_vreg();
        self.emit(abi::load_u64(&callee, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&callee);
        let ok = self.label("inplace_mapvalues_ok");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok));
        // A failing `f`: free the parked results — a `String` or any flat block
        // (bug-677) — and a `String` value's materialized copy.
        if self.callback_result_is_block(&value_type) {
            let regs = [
                RESULT_TAG_REGISTER,
                RESULT_VALUE_REGISTER,
                RESULT_ERROR_MESSAGE_REGISTER,
                RESULT_ERROR_SOURCE_REGISTER,
            ];
            let saved: Vec<usize> = regs
                .iter()
                .map(|_| self.allocate_stack_object("inplace_mapvalues_fail", 8))
                .collect();
            for (reg, slot) in regs.iter().zip(&saved) {
                self.emit(abi::store_u64(reg, abi::stack_pointer(), *slot));
            }
            self.free_collection_loop_item(item_slot, &value_type)?;
            let unwind = self.label("inplace_mapvalues_unwind");
            let unwound = self.label("inplace_mapvalues_unwound");
            self.emit(abi::label(&unwind));
            let index = self.temporary_vreg();
            let results = self.temporary_vreg();
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::compare_immediate(&index, "0"));
            self.emit(abi::branch_eq(&unwound));
            self.emit(abi::subtract_immediate(&index, &index, 1));
            self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::load_u64(&results, abi::stack_pointer(), results_slot));
            self.emit(abi::shift_left_immediate(&index, &index, 3));
            self.emit(abi::add_registers(&results, &results, &index));
            self.emit(abi::load_u64(&results, &results, 0));
            self.emit(abi::store_u64(&results, abi::stack_pointer(), result_slot));
            self.free_callback_result(result_slot, &value_type)?;
            self.emit(abi::branch(&unwind));
            self.emit(abi::label(&unwound));
            for (reg, slot) in regs.iter().zip(&saved) {
                self.emit(abi::load_u64(reg, abi::stack_pointer(), *slot));
            }
        }
        self.emit_callback_failure_exit(&[], None)?;
        self.emit(abi::label(&ok));
        let produced = self.temporary_vreg();
        self.emit(abi::move_register(&produced, RESULT_VALUE_REGISTER));
        let results = self.temporary_vreg();
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&results, abi::stack_pointer(), results_slot));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::shift_left_immediate(&index, &index, 3));
        self.emit(abi::add_registers(&results, &results, &index));
        self.emit(abi::store_u64(&produced, &results, 0));
        if is_string {
            let kept = self.label("inplace_mapvalues_item_kept");
            let carried = self.temporary_vreg();
            self.emit(abi::load_u64(&carried, abi::stack_pointer(), item_slot));
            self.emit(abi::compare_registers(&carried, &produced));
            self.emit(abi::branch_eq(&kept));
            self.free_collection_loop_item(item_slot, &value_type)?;
            self.emit(abi::label(&kept));
        }
        self.emit_map_loop_next(index_slot, &top, &done);

        // Room for every result written back (a longer value goes to the tail).
        let entries_slot = self.allocate_stack_object("inplace_mapvalues_entries", 8);
        let bytes_slot = self.allocate_stack_object("inplace_mapvalues_bytes", 8);
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            entries_slot,
        ));
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), bytes_slot));
        if !value_type_is_fixed(&value_type) {
            let (index_slot, top, done) = self.emit_map_loop_head(dest, "inplace_mapvalues_room");
            self.emit_load_result_word(results_slot, index_slot, result_slot);
            let len_slot = self.emit_payload_length_to_stack(
                &PayloadSlot {
                    slot: result_slot,
                    type_: value_type.clone(),
                },
                "inplace_mapvalues_len",
            )?;
            let total = self.temporary_vreg();
            let len = self.temporary_vreg();
            self.emit(abi::load_u64(&total, abi::stack_pointer(), bytes_slot));
            self.emit(abi::load_u64(&len, abi::stack_pointer(), len_slot));
            self.emit(abi::add_registers(&total, &total, &len));
            self.emit(abi::add_immediate(&total, &total, 16));
            self.emit(abi::store_u64(&total, abi::stack_pointer(), bytes_slot));
            self.emit_map_loop_next(index_slot, &top, &done);
            let grow = self.inplace_inline_grow(&opened)?;
            self.emit_map_reserve(dest, entries_slot, bytes_slot, &map_type, grow)?;
        }

        // Pass 2: write each result into its own entry, in place — the key is
        // never read, so no key is materialized.
        let (index_slot, top, done) = self.emit_map_loop_head(dest, "inplace_mapvalues_write");
        self.emit_load_result_word(results_slot, index_slot, result_slot);
        self.emit_map_entry_value_write(dest, index_slot, result_slot, &value_type)?;
        // The parked result was copied into its entry (bug-677: any flat block).
        self.free_callback_result(result_slot, &value_type)?;
        self.emit_map_loop_next(index_slot, &top, &done);
        self.close_field_dest(&target.dest, &opened)?;
        self.clear_self_update_constant(site.name);
        Ok(true)
    }
}
