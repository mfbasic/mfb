//! plan-176-U: `lower_for_each` and the walked-block deferral (`emit_defer_walked_block`). The
//! statement entry (`lower_ops`) and shared types live in
//! `engine/control/builder_control/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn lower_for_each(
        &mut self,
        name: &str,
        type_: &ParameterType,
        iterable: &NirValue,
        body: &[NirOp],
    ) -> Result<(), String> {
        let collection = self.temporary_vreg();
        let remaining = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let payload_off = self.temporary_vreg();
        let payload_len = self.temporary_vreg();
        // bug-666: a global iterable lowers to the global's live block, which a
        // `StoreGlobal` in the body (or in anything it calls) frees mid-loop. When
        // the body can reach one, walk a copy instead: the operand snapshot makes
        // it a statement-scope temporary, so the end of this `FOR EACH` statement
        // frees it on every exit edge. A body that only reads keeps borrowing.
        let snapshot_mark = self.operand_snapshot_wanted.len();
        if let Some(global) = global_root(iterable) {
            if self.ops_reach_store(body, StoreLeaf::Global(global)) {
                self.operand_snapshot_wanted
                    .push(iterable as *const NirValue as usize);
            }
        }
        // plan-142-F: likewise a local the body writes. Walking the binding's own
        // block would forbid every in-place self-update in the body (the loop holds
        // its pointer and count), so the loop walks a copy made once here — its
        // entry-time snapshot, freed with the statement — and the binding is free
        // to be mutated in place. A body that never writes it keeps borrowing.
        let owns_local_iterable = match iterable {
            NirValue::Local(local_name) => {
                self.locals
                    .get(local_name)
                    .is_some_and(|local| self.is_freeable_flat_value(&local.type_))
                    && ops_write_local(body, local_name)
            }
            _ => false,
        };
        // plan-142-G: `FOR EACH v IN r.items` whose body hands `r` to a callback by
        // reference — the callback's reassignment of `r` frees `r`'s old block, and
        // with it the field the loop walks. (A direct `r = …` in the body keeps
        // the old block alive instead: `for_each_iterable_record_fields`.)
        let owns_field_iterable = !matches!(iterable, NirValue::Local(_))
            && local_root(iterable).is_some_and(|root| {
                let mut captured = std::collections::HashSet::new();
                collect_address_taken_locals(body, &mut captured);
                captured.contains(root)
            });
        // plan-145-H (sites S7/T7): `FOR EACH v IN r.b` (or `IN h.state.b`) whose
        // body writes the owner walks a copy made once here, too. The field's own
        // block is then free for the body's in-place updates (`G15`/`G16` read
        // only the tracking lists, which this loop no longer joins), and the
        // rebuild's free guards no longer keep — and so leak — the displaced
        // block. A body that never writes the owner keeps borrowing.
        let owns_owner_field_iterable = match iterable {
            NirValue::MemberAccess { target, .. } => match target.as_ref() {
                NirValue::Local(base) => {
                    self.locals.contains_key(base.as_str()) && ops_write_local(body, base)
                }
                NirValue::MemberAccess {
                    target: inner,
                    member,
                } if member == "state" => {
                    matches!(inner.as_ref(), NirValue::Local(res) if ops_write_state(body, res))
                }
                _ => false,
            },
            _ => false,
        };
        if owns_local_iterable || owns_field_iterable || owns_owner_field_iterable {
            self.operand_snapshot_wanted
                .push(iterable as *const NirValue as usize);
        }
        let iterable_value = self.lower_value(iterable);
        self.operand_snapshot_wanted.truncate(snapshot_mark);
        let iterable_value = iterable_value?;
        if !typed_is_collection_type(&iterable_value.type_) {
            return Err(format!(
                "native code FOR EACH target '{}' is not a collection",
                iterable_value.type_
            ));
        }
        // Structural destructure (plan-104-D). The render that used to happen
        // here "for the string-typed payload emitters below" is gone with
        // plan-111-E: those emitters take the type.
        let map_entry_types = match &iterable_value.type_ {
            ParameterType::MapOf(..) => {
                let (key, value) =
                    typed_map_type_parts(&iterable_value.type_).ok_or_else(|| {
                        format!(
                            "native code FOR EACH target '{}' is not a valid map type",
                            iterable_value.type_
                        )
                    })?;
                Some((key.clone(), value.clone()))
            }
            _ => None,
        };
        let list_element_type = typed_list_element_type(&iterable_value.type_).cloned();
        // A `Set OF T` iterates its Map-shaped entries yielding the element `T`
        // (the entry key), not a `MapEntry` (plan-63-B). Computed here so the loop
        // body below can read the key payload directly into the loop local.
        let set_element_type = typed_set_element_type(&iterable_value.type_).cloned();
        let item_value_type = list_element_type.as_ref();
        let collection_slot = self.allocate_stack_object("for_each_collection", 8);
        let cursor_slot = self.allocate_stack_object("for_each_cursor", 8);
        let remaining_slot = self.allocate_stack_object("for_each_remaining", 8);
        let local_slot = self.allocate_stack_object(name, 8);
        let entry_payload_slot = if map_entry_types.is_some() {
            Some(self.allocate_stack_object("for_each_map_entry", 16))
        } else {
            None
        };

        self.emit(abi::store_u64(
            &iterable_value.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        // When the iterable is a plain local, its live buffer is the one this loop
        // snapshots and re-reads each step; record it so an in-place `set`/`prepend`
        // that overwrites an existing entry (observable to this iterator) is
        // excluded for the binding inside the loop body (plan-02 §4.1, D1).
        // A loop walking its own copy (plan-142-F) observes nothing of the binding.
        let pushed_iterable = match iterable {
            NirValue::Local(local_name) if !owns_local_iterable => {
                self.for_each_iterable_locals.push(local_name.clone());
                true
            }
            _ => false,
        };
        // bug-1036: a body that reassigns the binding this loop walks replaces the
        // block the loop still reads. The reassignment cannot free it, so it hands
        // it to this loop (`emit_defer_walked_block`), which frees it on every exit:
        // the `OwnedValue` cleanup below sits outside the body's scope, so `RETURN`
        // and the other function exits run it, an outer loop's `EXIT FOR` /
        // `CONTINUE FOR` unwinds through it, and this loop's own end drops it at
        // `end_label`. Registered exactly where the reassignment would have freed
        // the block: an owned binding whose block it frees.
        let entry_block = match iterable {
            NirValue::Local(local_name) if pushed_iterable && ops_write_local(body, local_name) => {
                self.locals
                    .get(local_name)
                    .filter(|local| {
                        !local.by_ref
                            && (self.is_freeable_flat_value(&local.type_)
                                || self.owns_graph(&local.type_))
                    })
                    .map(|local| (local_name.clone(), local.type_.clone()))
            }
            _ => None,
        };
        let entry_block = entry_block.map(|(local_name, type_)| {
            let deferred_slot = self.allocate_stack_object("for_each_replaced", 8);
            self.emit(abi::store_u64(
                abi::ZERO,
                abi::stack_pointer(),
                deferred_slot,
            ));
            self.for_each_entry_blocks.push(ForEachEntryBlock {
                name: local_name,
                collection_slot,
                deferred_slot,
            });
            let cleanup = OwnedValueCleanup {
                type_,
                stack_offset: deferred_slot,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            };
            self.active_cleanups
                .push(ActiveCleanup::OwnedValue(cleanup.clone()));
            (self.active_cleanups.len() - 1, cleanup)
        });
        // bug-430: `FOR EACH x IN resource.state.field` snapshots an ALIAS into the
        // STATE collection's inlined buffer, so record it — an in-place
        // `resource.state.field = append(...)` in the body must NOT reallocate+free
        // that buffer out from under the live iterator (it falls back to the
        // non-freeing whole-record rebuild instead).
        let pushed_state_field = if owns_owner_field_iterable {
            false
        } else if let NirValue::MemberAccess { target, member } = iterable {
            if let NirValue::MemberAccess {
                target: inner,
                member: inner_member,
            } = target.as_ref()
            {
                if inner_member == "state" {
                    if let NirValue::Local(res) = inner.as_ref() {
                        self.for_each_iterable_state_fields
                            .push((res.clone(), member.clone()));
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        // bug-430: `FOR EACH x IN local.field` over a MUT record local snapshots an
        // alias into the record's inlined collection buffer — record it so an
        // in-place record-field grow in the body falls back to the rebuild.
        let pushed_record_field = if owns_owner_field_iterable {
            false
        } else if let NirValue::MemberAccess { target, member } = iterable {
            if let NirValue::Local(base) = target.as_ref() {
                self.for_each_iterable_record_fields
                    .push((base.clone(), member.clone()));
                true
            } else {
                false
            }
        } else {
            false
        };
        self.emit(abi::load_u64(
            &collection,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            &remaining,
            &collection,
            COLLECTION_OFFSET_COUNT,
        ));
        // A kind-2 list has no entry table: the cursor carries a byte OFFSET from
        // the data base instead of an entry pointer, and strides by payloadSize
        // (plan-57-D). The Map arm below is unaffected — maps keep their entries.
        let list_payload = item_value_type.and_then(kind2_payload_size);
        if list_payload.is_some() {
            self.emit(abi::move_immediate(&cursor, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(
                &cursor,
                &collection,
                COLLECTION_HEADER_SIZE,
            ));
        }
        self.emit(abi::store_u64(&cursor, abi::stack_pointer(), cursor_slot));
        self.emit(abi::store_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));

        // bug-571: `(item slot, alias-witness slot)` for every payload this loop
        // MATERIALISES — the `String` arms below and nothing else. Filled inside
        // the loop (the spill must re-record the witness each iteration) and
        // registered as scope-drop obligations of the body's own cleanup scope
        // once the body scope is opened.
        let mut owned_item_slots: Vec<(usize, usize)> = Vec::new();
        let loop_label = self.label("for_each_loop");
        let end_label = self.label("for_each_end");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&end_label));
        self.emit(abi::load_u64(&cursor, abi::stack_pointer(), cursor_slot));
        if let (Some(entry_payload_slot), Some((key_type, value_type))) =
            (entry_payload_slot, map_entry_types.as_ref())
        {
            self.emit(abi::load_u64(
                &payload_off,
                &cursor,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64(
                &payload_len,
                &cursor,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit(abi::load_u64(
                &collection,
                abi::stack_pointer(),
                collection_slot,
            ));
            let (key_value, key_alias) = self.emit_load_map_payload_with_alias_base(
                key_type,
                &collection,
                &payload_off,
                &payload_len,
            )?;
            // bug-571: only the `String` arm allocates, so only it spills the alias
            // witness and only it registers a drop. Every other key type emits
            // exactly what it emitted before.
            if *key_type == ParameterType::String {
                owned_item_slots.push((
                    entry_payload_slot,
                    self.spill_to_slot("for_each_key_alias", &key_alias),
                ));
            }
            self.emit(abi::store_u64(
                key_value,
                abi::stack_pointer(),
                entry_payload_slot,
            ));
            self.emit(abi::load_u64(&cursor, abi::stack_pointer(), cursor_slot));
            self.emit(abi::load_u64(
                &payload_off,
                &cursor,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &payload_len,
                &cursor,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64(
                &collection,
                abi::stack_pointer(),
                collection_slot,
            ));
            let (item_value, value_alias) = self.emit_load_map_payload_with_alias_base(
                value_type,
                &collection,
                &payload_off,
                &payload_len,
            )?;
            if *value_type == ParameterType::String {
                owned_item_slots.push((
                    entry_payload_slot + 8,
                    self.spill_to_slot("for_each_value_alias", &value_alias),
                ));
            }
            self.emit(abi::store_u64(
                item_value,
                abi::stack_pointer(),
                entry_payload_slot + 8,
            ));
            self.emit(abi::add_immediate(
                &payload_off,
                abi::stack_pointer(),
                entry_payload_slot,
            ));
            self.emit(abi::store_u64(
                &payload_off,
                abi::stack_pointer(),
                local_slot,
            ));
        } else if let Some(set_element_type) = set_element_type.as_ref() {
            // `Set OF T`: read the entry's KEY payload (the element) into the loop
            // local as `T`. Entries stride by `COLLECTION_ENTRY_SIZE` (the common
            // advance below, since `list_payload` is `None` for a Set).
            self.emit(abi::load_u64(
                &payload_off,
                &cursor,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64(
                &payload_len,
                &cursor,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit(abi::load_u64(
                &collection,
                abi::stack_pointer(),
                collection_slot,
            ));
            let (item_value, item_alias) = self.emit_load_map_payload_with_alias_base(
                set_element_type,
                &collection,
                &payload_off,
                &payload_len,
            )?;
            if *set_element_type == ParameterType::String {
                owned_item_slots.push((
                    local_slot,
                    self.spill_to_slot("for_each_item_alias", &item_alias),
                ));
            }
            self.emit(abi::store_u64(item_value, abi::stack_pointer(), local_slot));
        } else {
            let item_value_type = item_value_type.ok_or_else(|| {
                format!(
                    "native code FOR EACH target '{}' is not a list",
                    iterable_value.type_
                )
            })?;
            if let Some(payload) = list_payload {
                self.emit(abi::move_register(&payload_off, &cursor));
                self.emit(abi::move_immediate(
                    &payload_len,
                    "Integer",
                    &payload.to_string(),
                ));
            } else {
                self.emit(abi::load_u64(
                    &payload_off,
                    &cursor,
                    COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
                ));
                self.emit(abi::load_u64(
                    &payload_len,
                    &cursor,
                    COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
                ));
            }
            self.emit(abi::load_u64(
                &collection,
                abi::stack_pointer(),
                collection_slot,
            ));
            let (item_value, item_alias) = self.emit_load_collection_payload_with_alias_base(
                item_value_type,
                &collection,
                &payload_off,
                &payload_len,
            )?;
            if *item_value_type == ParameterType::String {
                owned_item_slots.push((
                    local_slot,
                    self.spill_to_slot("for_each_item_alias", &item_alias),
                ));
            }
            self.emit(abi::store_u64(item_value, abi::stack_pointer(), local_slot));
        }
        self.emit(abi::load_u64(&cursor, abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate(
            &cursor,
            &cursor,
            list_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::store_u64(&cursor, abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
        self.emit(abi::store_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));

        let previous = self.locals.insert(
            name.to_string(),
            LocalValue {
                type_: type_.clone(),
                stack_offset: local_slot,
                constant: None,
                by_ref: false,
            },
        );
        self.clear_local_constants();
        // bug-571: open the body's cleanup scope HERE rather than inside
        // `lower_loop_body`, so the per-iteration item drops registered just below
        // live INSIDE it and every exit runs them. `body_scope_start` is the depth
        // `lower_ops` would have captured on its own, so with no item registered
        // this is the identical scope and the identical code.
        //
        // §14.7: "At normal scope exit, `RETURN`, `EXIT FOR`…, `CONTINUE FOR`…,
        // `FAIL`, `PROPAGATE`, or auto-propagated errors, live bindings are dropped
        // in reverse declaration order within each scope." The loop variable is
        // such a binding, and this is the scope. Every one of those edges is served
        // by an existing emitter — the tail of `lower_ops_inner` (fall-through),
        // `emit_cleanup_branch_to_depth` (`EXIT FOR`/`CONTINUE FOR`, which jump
        // around the fall-through path entirely), and `emit_current_result_exit`
        // (`RETURN`/`FAIL`/auto-propagate) — so the fix is the registration, not a
        // new drop point per edge.
        let body_scope_start = self.active_cleanups.len();
        self.cleanup_scope_starts.push(body_scope_start);
        for (item_slot, alias_slot) in &owned_item_slots {
            self.active_cleanups
                .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                    type_: ParameterType::String,
                    stack_offset: *item_slot,
                    capacity_slot: None,
                    loop_alias_slot: Some(*alias_slot),
                    result_wrapper: None,
                }));
            self.owned_value_slots.push(*item_slot);
        }
        self.loop_stack.push(LoopLabels {
            kind: crate::ast::LoopKind::For,
            name: Some(name.to_string()),
            continue_label: loop_label.clone(),
            exit_label: end_label.clone(),
            // Captured BEFORE the item drops were pushed, so `EXIT FOR` and
            // `CONTINUE FOR` unwind through them.
            cleanup_depth: body_scope_start,
            temp_depth: self.pending_temp_frees.len(),
        });
        self.enclosing_loop_reassigned.push(
            crate::codegen::engine::function::collect_reassigned_locals(body),
        );
        let body_result = self.lower_ops_inner(body, body_scope_start);
        self.enclosing_loop_reassigned.pop();
        self.cleanup_scope_starts.pop();
        self.loop_stack.pop();
        body_result?;
        if pushed_iterable {
            self.for_each_iterable_locals.pop();
        }
        if pushed_state_field {
            self.for_each_iterable_state_fields.pop();
        }
        if pushed_record_field {
            self.for_each_iterable_record_fields.pop();
        }
        if let Some(previous) = previous {
            self.locals.insert(name.to_string(), previous);
        } else {
            self.locals.remove(name);
        }
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&end_label));
        if let Some((index, cleanup)) = entry_block {
            self.for_each_entry_blocks.pop();
            debug_assert_eq!(index + 1, self.active_cleanups.len());
            self.active_cleanups.remove(index);
            self.emit_owned_value_drop(&cleanup)?;
        }
        self.clear_local_constants();
        Ok(())
    }

    /// bug-1036: before a reassignment of `name` (slot `stack_offset`) frees the
    /// block the binding held, hand that block to the outermost live `FOR EACH`
    /// still walking it: store it in that loop's `deferred_slot` and branch to the
    /// returned label, past the free, which the caller places after its drop. A
    /// block no loop walks falls through to the free. `None` (nothing emitted) when
    /// no live loop walks `name`'s own block.
    pub(crate) fn emit_defer_walked_block(
        &mut self,
        name: &str,
        stack_offset: usize,
    ) -> Option<String> {
        let loops: Vec<(usize, usize)> = self
            .for_each_entry_blocks
            .iter()
            .filter(|entry| entry.name == name)
            .map(|entry| (entry.collection_slot, entry.deferred_slot))
            .collect();
        if loops.is_empty() {
            return None;
        }
        let deferred = self.label("reassign_old_walked");
        for (collection_slot, deferred_slot) in loops {
            let next = self.label("reassign_old_not_walked");
            let old = self.temporary_vreg();
            let walked = self.temporary_vreg();
            self.emit(abi::load_u64(&old, abi::stack_pointer(), stack_offset));
            self.emit(abi::load_u64(
                &walked,
                abi::stack_pointer(),
                collection_slot,
            ));
            self.emit(abi::compare_registers(&old, &walked));
            self.emit(abi::branch_ne(&next));
            self.emit(abi::store_u64(&old, abi::stack_pointer(), deferred_slot));
            self.emit(abi::branch(&deferred));
            self.emit(abi::label(&next));
        }
        Some(deferred)
    }
}
