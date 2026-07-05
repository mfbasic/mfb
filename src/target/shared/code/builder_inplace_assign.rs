use super::*;

use super::builder_control::string_self_append_operands;

impl CodeBuilder<'_> {

    /// Recognize `name = collections::append(name, item)` for a single element
    /// appended to a uniquely-owned `MUT` list local, and lower it as an in-place
    /// grow (plan-01 §4.2). Returns `true` when handled (the local's slot was
    /// updated in place); `false` to fall back to the general reassignment path.
    ///
    /// Soundness: under MFBASIC value semantics every binding owns its buffer and
    /// copy-insertion deep-copies any aliasing assignment, so the local's buffer
    /// has no live alias. `FOR EACH` snapshots the buffer pointer and count at
    /// loop entry, and in-place append only writes *beyond* that snapshot count
    /// without moving existing entries or payloads, so iteration is unaffected.
    /// Reference (`by_ref`) locals are excluded — their slot holds a pointer to
    /// the parent slot, not the buffer — and bulk `append(list, otherList)` is
    /// excluded (the item must be a single element).
    pub(super) fn try_inplace_append_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        if crate::builtins::native_builtin_target(target) != Some("append") || args.len() != 2 {
            return Ok(false);
        }
        let NirValue::Local(arg0) = &args[0] else {
            return Ok(false);
        };
        if arg0 != name {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let list_type = local.type_.clone();
        let Some(element_type) = super::list_element_type(&list_type) else {
            return Ok(false);
        };
        if super::CollectionTypeLayout::from_type(&list_type).is_none() {
            return Ok(false);
        }
        // Commit only for a statically-known single element of the list's element
        // type. A bulk `append(list, otherList)` has item type == list_type and
        // falls through to the general (concatenating) path.
        match self.static_type_name(&args[1]) {
            Some(item_type) if item_type == element_type => {}
            _ => return Ok(false),
        }
        let item = self.lower_value(&args[1])?;
        // Observation boundary: an in-place appended `Float` must be finite
        // (plan-17).
        self.observe_float(&args[1], &item)?;
        // Materialize a `d`-native float before the payload spill (plan-01).
        let item = self.materialize_float(item)?;
        let item_slot = self.allocate_stack_object("inplace_append_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        self.lower_list_append_in_place(stack_offset, item_slot, &list_type, &element_type)?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// Recognize `name = collections::set(name, index, item)` on a uniquely-owned
    /// `MUT` **list** local and lower it as an in-place overwrite (plan-02 §4.1).
    /// When the replacement payload fits the target slot (`newLen <= oldLen`, the
    /// fixed-width and same-size record cases always do) the value bytes are
    /// overwritten at the entry's `valueOffset` and `valueLength` patched — no
    /// allocation, no copy. Otherwise it falls back to the rebuild (remove+insert)
    /// path, which is always correct (D1). Returns `true` when handled.
    ///
    /// Soundness mirrors `try_inplace_append_assign`: value semantics + copy
    /// insertion guarantee the buffer is unaliased, and `by_ref` locals are
    /// excluded. Unlike append, an overwrite is observable to an enclosing
    /// `FOR EACH` over the same binding, so that case is excluded
    /// (`for_each_iterable_locals`). The map overload (Phase 3) is the same shape:
    /// scan for the key, overwrite the value in place when it fits, append a new
    /// entry into spare slot/data headroom otherwise (geometric grow when full).
    pub(super) fn try_inplace_set_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        if crate::builtins::native_builtin_target(target) != Some("set") || args.len() != 3 {
            return Ok(false);
        }
        let NirValue::Local(arg0) = &args[0] else {
            return Ok(false);
        };
        if arg0 != name {
            return Ok(false);
        }
        if self.for_each_iterable_locals.iter().any(|n| n == name) {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let collection_type = local.type_.clone();
        if super::CollectionTypeLayout::from_type(&collection_type).is_none() {
            return Ok(false);
        }
        if let Some(element_type) = super::list_element_type(&collection_type) {
            // The list `set` item is always a single element of type `T`
            // (syntaxcheck-enforced), so — unlike append's bulk-vs-single gate — no
            // static element-type check is needed; the post-lowering `item.type_`
            // check catches any mismatch.
            let index = self.lower_value(&args[1])?;
            if index.type_ != "Integer" {
                return Err(format!(
                    "native collection set list index must be Integer, got {}",
                    index.type_
                ));
            }
            let index_slot = self.allocate_stack_object("inplace_set_index", 8);
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            let item = self.lower_value(&args[2])?;
            // Observation boundary: an in-place replacement `Float` element must
            // be finite (plan-17).
            self.observe_float(&args[2], &item)?;
            if item.type_ != element_type {
                return Err(format!(
                    "native collection set list item must be {element_type}, got {}",
                    item.type_
                ));
            }
            let item = self.materialize_float(item)?;
            let item_slot = self.allocate_stack_object("inplace_set_item", 8);
            self.emit(abi::store_u64(
                &item.location,
                abi::stack_pointer(),
                item_slot,
            ));
            self.lower_list_set_in_place(
                stack_offset,
                index_slot,
                item_slot,
                &collection_type,
                &element_type,
            )?;
            if let Some(local) = self.locals.get_mut(name) {
                local.constant = None;
            }
            return Ok(true);
        }
        if let Some((key_type, value_type)) = super::map_type_parts(&collection_type) {
            let key = self.lower_value(&args[1])?;
            // Observation boundary: an in-place `Float` map key must be finite
            // (plan-17).
            self.observe_float(&args[1], &key)?;
            if key.type_ != key_type {
                return Err(format!(
                    "native collection set map key must be {key_type}, got {}",
                    key.type_
                ));
            }
            let key = self.materialize_float(key)?;
            let key_slot = self.allocate_stack_object("inplace_set_key", 8);
            self.emit(abi::store_u64(&key.location, abi::stack_pointer(), key_slot));
            let val = self.lower_value(&args[2])?;
            // Observation boundary: an in-place `Float` map value must be finite
            // (plan-17).
            self.observe_float(&args[2], &val)?;
            if val.type_ != value_type {
                return Err(format!(
                    "native collection set map value must be {value_type}, got {}",
                    val.type_
                ));
            }
            let val = self.materialize_float(val)?;
            let value_slot = self.allocate_stack_object("inplace_set_value", 8);
            self.emit(abi::store_u64(&val.location, abi::stack_pointer(), value_slot));
            self.lower_map_set_in_place(
                stack_offset,
                key_slot,
                value_slot,
                &collection_type,
                &key_type,
                &value_type,
            )?;
            if let Some(local) = self.locals.get_mut(name) {
                local.constant = None;
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Recognize `name = collections::prepend(name, item)` on a uniquely-owned
    /// `MUT` list local and lower it as an in-place prepend (plan-02 §3): shift the
    /// live lookup entries right by one and write the new entry at index 0, with the
    /// new element's payload appended to the spare data tail — no per-op allocation
    /// (geometric grow only when full). Still O(n) per op (the entry shift), but it
    /// drops the alloc + double-copy the value-semantic insert did each call. Like
    /// `set`, the entry shift is observable to an enclosing `FOR EACH` over the same
    /// binding, so that case is excluded. Returns `true` when handled.
    pub(super) fn try_inplace_prepend_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        if crate::builtins::native_builtin_target(target) != Some("prepend") || args.len() != 2 {
            return Ok(false);
        }
        let NirValue::Local(arg0) = &args[0] else {
            return Ok(false);
        };
        if arg0 != name {
            return Ok(false);
        }
        if self.for_each_iterable_locals.iter().any(|n| n == name) {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let list_type = local.type_.clone();
        let Some(element_type) = super::list_element_type(&list_type) else {
            return Ok(false);
        };
        if super::CollectionTypeLayout::from_type(&list_type).is_none() {
            return Ok(false);
        }
        // `prepend` always takes a single element of the list element type
        // (a bulk form is rejected in `lower_collection_prepend`), so no static
        // gate is needed; the post-lowering check catches any mismatch.
        let item = self.lower_value(&args[1])?;
        // Observation boundary: an in-place prepended `Float` must be finite
        // (plan-17).
        self.observe_float(&args[1], &item)?;
        if item.type_ != element_type {
            return Err(format!(
                "native collection prepend item must be {element_type}, got {}",
                item.type_
            ));
        }
        let item = self.materialize_float(item)?;
        let item_slot = self.allocate_stack_object("inplace_prepend_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        self.lower_list_prepend_in_place(stack_offset, item_slot, &list_type, &element_type)?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// Recognize `name = name & x` (and the left-associated chain
    /// `name = name & a & b …`) on a uniquely-owned `MUT` `String` local and lower
    /// it as an in-place self-append (plan-02 §4.1, the string sibling of
    /// `try_inplace_append_assign`). The grown buffer carries geometric capacity
    /// headroom tracked in a frame-local shadow slot, so each append writes the
    /// operand's bytes into the spare tail and bumps the length — amortized O(1) —
    /// instead of `lower_string_concat` allocating a fresh tight buffer every time.
    /// The shadow never escapes: any copy/return/transfer reads only `len` bytes,
    /// freezing the value to the canonical tight `[len][bytes][NUL]` form (D9). A
    /// `String` can never be a `FOR EACH` iterable, so this needs no iterator gate.
    /// Returns `true` when handled.
    pub(super) fn try_inplace_concat_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        // Only fire for a name we pre-allocated a capacity shadow for (a self-append
        // target discovered by the prescan); the shadow is reset on every other
        // bind/assign so it always reflects the live buffer's spare bytes.
        let Some(&shadow_slot) = self.string_capacity_slots.get(name) else {
            return Ok(false);
        };
        let Some(operands) = string_self_append_operands(value, name) else {
            return Ok(false);
        };
        for operand in operands {
            self.lower_string_self_append_one(stack_offset, shadow_slot, operand)?;
        }
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// Append one `String` operand's bytes to the grown self-append buffer whose
    /// pointer lives at `name_slot`, using/maintaining the spare-capacity shadow at
    /// `shadow_slot`. Writes into the spare tail when `rlen <= spare`; otherwise
    /// allocates a geometric-headroom buffer, copies the current bytes + the
    /// operand, and repoints `name_slot`. Mirrors `lower_list_append_in_place`.
    fn lower_string_self_append_one(
        &mut self,
        name_slot: usize,
        shadow_slot: usize,
        operand: &NirValue,
    ) -> Result<(), String> {
        let right = self.lower_value(operand)?;
        if right.type_ != "String" {
            return Err(format!(
                "native string self-append operand must be String, got {}",
                right.type_
            ));
        }
        let right_slot = self.allocate_stack_object("concat_self_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        let newlen_slot = self.allocate_stack_object("concat_self_newlen", 8);
        let newcap_slot = self.allocate_stack_object("concat_self_newcap", 8);
        let newbuf_slot = self.allocate_stack_object("concat_self_newbuf", 8);

        let ptr = self.temporary_vreg();
        let len = self.temporary_vreg();
        let right_ptr = self.temporary_vreg();
        let rlen = self.temporary_vreg();
        let newlen = self.temporary_vreg();
        let spare = self.temporary_vreg();
        let newcap = self.temporary_vreg();
        let step_scratch = self.temporary_vreg();
        let zero = self.temporary_vreg();
        let dst = self.temporary_vreg();

        let regrow = self.label("concat_self_regrow");
        let write = self.label("concat_self_write");
        let alloc_ok = self.label("concat_self_alloc_ok");
        let cap_keep = self.label("concat_self_cap_keep");
        let done = self.label("concat_self_done");

        // newlen = len + rlen; decide in-place vs regrow on rlen vs spare.
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&len, &ptr, 0)); // len
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(&rlen, &right_ptr, 0)); // rlen
        self.emit(abi::add_registers(&newlen, &len, &rlen));
        self.emit(abi::store_u64(&newlen, abi::stack_pointer(), newlen_slot));
        self.emit(abi::load_u64(&spare, abi::stack_pointer(), shadow_slot)); // spare
        self.emit(abi::compare_registers(&rlen, &spare));
        self.emit(abi::branch_hi(&regrow)); // rlen > spare → regrow
        self.emit(abi::branch(&write));

        // --- Regrow: alloc newcap_payload + 9; copy old + operand; install. ---
        self.emit(abi::label(&regrow));
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&len, &ptr, 0)); // len
        self.emit(abi::load_u64(&spare, abi::stack_pointer(), shadow_slot)); // spare
        self.emit(abi::add_registers(&right_ptr, &len, &spare)); // current payload capacity
        self.emit_geometric_step(
            &right_ptr,
            &newcap,
            &step_scratch,
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "concat_self_step",
        );
        // newcap_payload = max(step, newlen).
        self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
        self.emit(abi::compare_registers(&newcap, &newlen));
        self.emit(abi::branch_hi(&cap_keep));
        self.emit(abi::branch_eq(&cap_keep));
        self.emit(abi::move_register(&newcap, &newlen));
        self.emit(abi::label(&cap_keep));
        self.emit(abi::store_u64(&newcap, abi::stack_pointer(), newcap_slot));
        // alloc size = 8 (len word) + newcap_payload + 1 (NUL).
        self.emit(abi::add_immediate(abi::return_register(), &newcap, 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), newbuf_slot));
        // newbuf[0] = newlen.
        self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
        self.emit(abi::store_u64(&newlen, "x1", 0));
        // Copy the current bytes (len) to newbuf+8.
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&len, &ptr, 0)); // len
        self.emit(abi::add_immediate(&ptr, &ptr, 8)); // old data
        self.emit(abi::add_immediate(&dst, "x1", 8)); // new data
        self.emit_copy_bytes(&dst, &ptr, &len, "concat_self_old");
        // Copy the operand bytes (rlen) to newbuf+8+len. dst now points at +8+len.
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(&rlen, &right_ptr, 0)); // rlen
        self.emit(abi::add_immediate(&right_ptr, &right_ptr, 8)); // operand data
        self.emit_copy_bytes(&dst, &right_ptr, &rlen, "concat_self_new");
        // NUL terminator at newbuf+8+newlen.
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::store_u8(&zero, &dst, 0));
        // Install new buffer; spare = newcap_payload - newlen.
        self.emit(abi::load_u64("x1", abi::stack_pointer(), newbuf_slot));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&newcap, abi::stack_pointer(), newcap_slot));
        self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
        self.emit(abi::subtract_registers(&newcap, &newcap, &newlen));
        self.emit(abi::store_u64(&newcap, abi::stack_pointer(), shadow_slot));
        self.emit(abi::branch(&done));

        // --- In place: write operand bytes into the spare tail. ---
        self.emit(abi::label(&write));
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&len, &ptr, 0)); // len
        self.emit(abi::add_immediate(&dst, &ptr, 8));
        self.emit(abi::add_registers(&dst, &dst, &len)); // dst = ptr+8+len
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(&rlen, &right_ptr, 0)); // rlen
        self.emit(abi::add_immediate(&right_ptr, &right_ptr, 8)); // operand data
        self.emit_copy_bytes(&dst, &right_ptr, &rlen, "concat_self_inplace");
        // NUL after the new end; ptr[0] = newlen; spare -= rlen.
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::store_u8(&zero, &dst, 0));
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
        self.emit(abi::store_u64(&newlen, &ptr, 0));
        self.emit(abi::load_u64(&spare, abi::stack_pointer(), shadow_slot));
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(&rlen, &right_ptr, 0)); // rlen
        self.emit(abi::subtract_registers(&spare, &spare, &rlen));
        self.emit(abi::store_u64(&spare, abi::stack_pointer(), shadow_slot));
        self.emit(abi::label(&done));
        Ok(())
    }
}
