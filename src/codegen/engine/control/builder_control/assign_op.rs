//! plan-176-U: `lower_assign_op` and `lower_state_assign_op`, the `NirOp::Assign` and
//! `NirOp::StateAssign` arms of the statement dispatch. Called from `lower_ops_inner` in
//! `engine/control/builder_control/statement_dispatch.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Lower one `NirOp::Assign` (`x = v`). Runs inside `lower_ops_inner`'s per-op closure, so its
    /// `?` is attributed to the op by `nir_op_context`.
    pub(super) fn lower_assign_op(
        &mut self,
        op: &NirOp,
        name: &str,
        value: &NirValue,
    ) -> Result<(), String> {
        // Rule 4: whether this store can carry a resource owned
        // inside the container's scope (`record_ownership::
        // StoreFloats`); the store raises what it stores.
        self.statement_temps.assign_target = Some(name.to_string());
        self.statement_temps.assign_floats = self
            .store_floats
            .stores
            .contains(&(value as *const NirValue as usize));
        self.statement_temps.item_floated = false;
        // bug-689: an element-bound binding's update, its write-back
        // (the element already holds the value it writes), and the
        // single-expression update. None changes the list's length, so
        // the facts below stay true.
        let key = crate::codegen::engine::analysis::last_use::op_key(op);
        if self.element_updates.write_backs.contains(&key) {
            return Ok(());
        }
        if let Some(element) = self.element_updates.updates.get(&key).cloned() {
            return self.lower_element_update(&element, Some(key), value);
        }
        if self.try_single_expression_element_update(op, value)? {
            return Ok(());
        }
        // plan-86 G1: reassigning `name` invalidates any `len_of_local`
        // or provable-index fact keyed on it, and any fact naming it
        // as the list `L`.
        self.len_of_local.remove(name);
        self.len_of_local.retain(|_, (l, _)| l != name);
        self.provable_index_locals.remove(name);
        self.provable_index_locals.retain(|_, (l, _)| l != name);
        // A loop-promoted float local is updated in its FP
        // register, not its slot (plan-03 Stage D part 2).
        if let Some(d) = self.promoted_float_locals.get(name).cloned() {
            let result = self.lower_value(value)?;
            self.update_promoted_float(d, &result);
            // Observation boundary: the promoted accumulator's
            // `d`-register holds the named local's value after
            // the update, so check it there — the FP-domain
            // variant keeps the store-to-slot peephole-foldable
            // (plan-16/plan-17).
            self.observe_promoted_float(value, d)?;
            return Ok(());
        }
        let (stack_offset, by_ref, local_type) = {
            let local = self
                .locals
                .get(name)
                .ok_or_else(|| format!("native code assignment unknown local '{name}'"))?;
            (local.stack_offset, local.by_ref, local.type_.clone())
        };
        // `name = f(name, …)` on a uniquely owned `MUT` local
        // mutates the live block in place when an arm of
        // `SELF_UPDATE_ARMS` recognises `f` (plan-142-A, site S1):
        // the arm updates the slot, so skip the general
        // reassignment path entirely. A by-ref capture (S9,
        // plan-142-G) mutates the parent's block through the
        // reference.
        let dest = if by_ref
            && (is_self_update_call(value, name)
                                // plan-146-G: `s = s & t` is a `Binary`, not a
                                // call, and it is an arm at S9 now too.
                                || string_self_append_operands_of(value, &|root| {
                                    matches!(root, NirValue::Local(local) if local == name)
                                })
                                .is_some())
        {
            InPlaceDest::Ref {
                ref_slot: stack_offset,
                block_slot: self.allocate_stack_object("su_ref_block", 8),
            }
        } else {
            InPlaceDest::Direct { slot: stack_offset }
        };
        let site = SelfUpdateSite {
            name,
            type_: local_type.clone(),
            dest,
            by_ref,
            field: None,
        };
        // plan-145-B: `name = WITH name { f := op(name.f, …) }` — a
        // self-update of one of the record local's fields — is a
        // field site of the same seam.
        let field_site = self
            .field_self_update_site(FieldContainer::Record { local: name }, value)
            .map(|(field, field_type, update)| {
                (
                    SelfUpdateSite {
                        name,
                        type_: field_type,
                        // plan-145-H: a by-ref capture's slot
                        // holds the parent slot's address.
                        dest: if by_ref {
                            InPlaceDest::RefField {
                                ref_slot: stack_offset,
                                field_index: field.field_index,
                                path: field.path_indices(),
                            }
                        } else {
                            InPlaceDest::Inlined {
                                block_slot: stack_offset,
                                field_index: field.field_index,
                                path: field.path_indices(),
                                write_back: WriteBack::None,
                            }
                        },
                        by_ref,
                        field: Some(field),
                    },
                    update,
                )
            });
        // plan-145-C: a `WITH` whose every update is a scalar or a
        // pointer field is stored field by field in the record's
        // own block (the `STATE` Layer 1 routine, for a local).
        let in_place = self.try_inplace_self_update(&site, value)?
            || self.try_inplace_scalar_fields(FieldContainer::Record { local: name }, value)?
            || match &field_site {
                Some((site, update)) => self.try_inplace_self_update(site, update)?,
                None => false,
            }
            || self.try_inplace_mixed_with(
                FieldContainer::Record { local: name },
                value,
                Some(stack_offset),
                by_ref,
            )?;
        if !in_place {
            // Reassignment installs a fresh independent block; the old
            // block remains owned by this binding's scope-drop free
            // (the slot is overwritten with the new owner). Deep-copy
            // an aliasing source so the binding stays unaliased.
            let result = self.lower_value_owned(value)?;
            // bug-1026: an aliasing `RECOVER` into a resource-union
            // inline-`TRAP` temp (`RECOVER other`, `RECOVER b.h`,
            // `RECOVER collections::getOr(…)`) lends the temp a
            // record and hands it a COPY of the box: the address the
            // value names is another binding's box, or lies inside a
            // record's or a collection's block, and none of those
            // need outlive the temp. A borrowed element that arrives
            // as the trapped call's own `resultValue` was copied when
            // its `Result` was built. The temp's drop frees the copy
            // and closes nothing (the flag written below is clear).
            let result = if self.trap_owner_flags.contains_key(name)
                && self.resource_union_cleanup(&local_type).is_some()
                && !matches!(value, NirValue::ResultValue { .. })
                && !self.trap_ownership.store_is_owned(value)
            {
                let copied =
                    self.materialize_inline_value_in_arena(&local_type, &result.location)?;
                ValueResult {
                    location: Operand::from(copied),
                    ..result
                }
            } else {
                result
            };
            // Observation boundary: a `Float` reassignment must
            // be finite (plan-17).
            self.observe_float(value, &result)?;
            // bug-755: the scope of the binding's registered close, when
            // it has one. A binding with no close is a reference: it
            // claims nothing, and the old record's owner check below can
            // only match a record this very scope owns.
            let registered_depth =
                self.active_cleanups
                    .iter()
                    .rev()
                    .find_map(|cleanup| match cleanup {
                        ActiveCleanup::Resource(resource) if resource.name == *name => {
                            Some(resource.scope_depth)
                        }
                        _ => None,
                    });
            let assign_slot = if let Some(symbol) = self.resource_cleanup_symbol(&result.type_) {
                let slot = self.allocate_stack_object("resource_assign_value", 8);
                self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
                let cleanup = ResourceCleanup {
                    name: name.to_string(),
                    symbol,
                    state_type: result.type_.state(),
                    has_io_buffers: Self::resource_uses_io_buffers(&result.type_),
                    // The value a store replaces is closed here
                    // and its record kept: the binding's scope has
                    // not exited, and another cleanup of it may
                    // still read the record.
                    frees_record: false,
                    moved_record_only: false,
                    owner_flag_slot: self.trap_owner_flags.get(name).copied(),
                    default_record_slot: self.trap_default_records.get(name).copied(),
                    scope_depth: registered_depth.unwrap_or_else(|| self.current_scope_depth()),
                    reclaim_slot: None,
                };
                self.emit_resource_cleanup_call(&cleanup)?;
                Some(slot)
            } else if let Some(cleanup) = self
                .resource_union_cleanup(&local_type)
                .and_then(|_| self.resource_union_binding_cleanup(name))
            {
                // bug-1037: a resource-union binding that is assigned
                // drops the value it held first, as the concrete arm
                // above does. The only such binding is the inline-`TRAP`
                // desugar's `$trap_valN` (a `RES` binding is not
                // assignable), whose bind materializes a closed default
                // `{tag, record}` box and record that nothing else
                // names: storing the call's result over it kept 16 +
                // 112 bytes per evaluation. The drop is the binding's
                // own registered one, so it answers for the old value
                // exactly as the scope exit would have: the owner flag
                // still describes that value (it is rewritten below,
                // after the store), a lent value loses only the temp's
                // copy of its box, and the slot is nulled. The new value
                // is spilled across it because the drop calls the close
                // op and `arena_free`.
                let slot = self.allocate_stack_object("resource_union_assign_value", 8);
                self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
                // The binding's scope has not exited: the
                // replaced value is closed and its record kept
                // (the default record is freed by its own rule).
                let cleanup = ResourceUnionCleanup {
                    reclaim_slot: None,
                    record_free_tags: Vec::new(),
                    ..cleanup
                };
                self.emit_resource_union_cleanup_call(&cleanup)?;
                Some(slot)
            } else if by_ref && self.is_freeable_flat_value(&result.type_) {
                // plan-142-G: a reference local's reassignment frees the
                // parent binding's previous block, read through the
                // reference, exactly as the owner's own reassignment
                // below would; it leaked before. Nothing in the parent
                // still walks that block: a `forEach` over it walks a
                // snapshot, and so does a `FOR EACH` whose body passes
                // the binding by reference (operand_snapshot.rs).
                let old_slot = self.allocate_stack_object("reassign_ref_old", 8);
                let parent = self.allocate_register();
                self.emit(abi::load_u64(&parent, abi::stack_pointer(), stack_offset));
                let old = self.allocate_register();
                self.emit(abi::load_u64(&old, &parent, 0));
                self.emit(abi::store_u64(&old, abi::stack_pointer(), old_slot));
                let slot = self.allocate_stack_object("reassign_value", 8);
                self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
                // plan-146-G: the owner's block may carry spare
                // bytes an arm in this lambda left there, so the
                // free is sized by the SHARED shadow — and the
                // shadow is reset to 0 below, because the block
                // being installed is a fresh tight one.
                let shared = self.string_shadow_working_copy(name);
                let (shared_slot, shared_holder) = match shared {
                    Some((slot, holder)) => (Some(slot), Some(holder)),
                    None => (None, None),
                };
                self.emit_owned_value_drop(&OwnedValueCleanup {
                    type_: result.type_.clone(),
                    stack_offset: old_slot,
                    capacity_slot: shared_slot,
                    loop_alias_slot: None,
                    result_wrapper: None,
                })?;
                self.reset_shared_string_shadow(shared_holder);
                Some(slot)
            } else if !by_ref
                                && (self.is_freeable_flat_value(&result.type_)
                                    || self.owns_graph(&result.type_))
                                // bug-430: a live `FOR EACH x IN name.field` holds an
                                // alias into this record's block; freeing the block
                                // mid-loop is a use-after-free (this rebuild is the
                                // fallback the in-place record grow declines to for
                                // exactly this reason). Leak the old block instead.
                                && !self
                                    .for_each_iterable_record_fields
                                    .iter()
                                    .any(|(base, _)| base == name)
            {
                // Free the binding's previous block before
                // overwriting the slot. A reassignment installs a
                // fresh independent block; without this the old
                // block leaks (bug-01) — e.g. the value-semantic
                // `list = collections::append(list, complexItem)`
                // fallback (when the item isn't a simple element
                // and the in-place path declines) or a plain
                // `list = otherList`. The new value is spilled
                // across the `arena_free` (which trashes the
                // caller-saved registers) and reloaded, mirroring
                // the thread/resource paths. The slot still holds
                // the old block here, and the freshly computed new
                // value never aliases it (`lower_value_owned`
                // deep-copies aliasing sources; a call result is a
                // fresh block), so the free is sound and once-only.
                // bug-1036: a live `FOR EACH` that walks this
                // binding's own block still reads it, so that one
                // block is handed to the loop, which frees it when
                // it ends (`emit_defer_walked_block`); any other
                // block the binding held is freed here.
                let slot = self.allocate_stack_object("reassign_value", 8);
                self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
                let walked = self.emit_defer_walked_block(name, stack_offset);
                self.emit_owned_value_drop(&OwnedValueCleanup {
                    type_: result.type_.clone(),
                    stack_offset,
                    // bug-560: this drop runs BEFORE the new value is
                    // stored and before `reset_string_capacity_shadow`,
                    // so the shadow still describes the block being
                    // freed — which is the one an earlier self-append
                    // grew. Freeing it tight orphaned the headroom on
                    // every reassignment.
                    capacity_slot: self.string_capacity_slot_for(name, &result.type_),
                    loop_alias_slot: None,
                    result_wrapper: None,
                })?;
                if let Some(walked) = walked {
                    self.emit(abi::label(&walked));
                }
                Some(slot)
            } else {
                None
            };
            // What to store: the value itself, or a GPR reloaded
            // from the thread/resource cleanup slot (never a
            // `Float`, so always GP-native there). A `d`-native
            // `Float` is stored via `str d` (plan-01
            // float-dnative).
            let store_value = if let Some(slot) = assign_slot {
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, abi::stack_pointer(), slot));
                ValueResult {
                    origin: None,
                    type_: result.type_.clone(),
                    location: Operand::from(register),
                    text: String::new(),
                }
            } else {
                result.clone()
            };
            if by_ref {
                // A reference local (non-escaping `MUT` by-ref capture): write
                // through the slot pointer so the live parent binding is
                // updated, not a local copy.
                let slot_pointer = self.allocate_register();
                self.emit(abi::load_u64(
                    &slot_pointer,
                    abi::stack_pointer(),
                    stack_offset,
                ));
                self.store_value_at(&store_value, &slot_pointer, 0);
            } else {
                self.store_value_at(&store_value, abi::stack_pointer(), stack_offset);
                // bug-755: the binding's scope claims the new record when
                // it is unowned, exactly as the bind does.
                if let (Some(depth), Some(_)) = (registered_depth, assign_slot) {
                    if self.resource_cleanup_symbol(&result.type_).is_some() {
                        self.emit_resource_claim_at(stack_offset, depth)?;
                    }
                }
                // A resource-union binding whose drop closes its active
                // variant claims the new variant's record the same way:
                // the drop is owner-checked, so an unclaimed record (the
                // inline-`TRAP` desugar fills `$trap_valN` by assignment)
                // would never be closed.
                if let Some(depth) =
                    self.active_cleanups
                        .iter()
                        .rev()
                        .find_map(|cleanup| match cleanup {
                            ActiveCleanup::ResourceUnion(union)
                                if union.name == *name && union.closes_variant =>
                            {
                                Some(union.scope_depth)
                            }
                            _ => None,
                        })
                {
                    self.emit_resource_union_claim_at(stack_offset, depth)?;
                }
                // bug-805: a container assigned a call result adopts the
                // resources the callee handed out, exactly as a bind of
                // one does (`call_assigned_owner_containers` gave it its
                // owned-lists at its bind).
                if self.adopts_from_trapped_call(value)
                    && (self.owned_list_heads.keys().any(|(list, _)| list == name)
                        || self.dynamic_owned_list(name).is_some())
                {
                    self.emit_owned_list_seed_from_collection(name, stack_offset, &local_type)?;
                }
            }
            // bug-648: record whether this store handed a `Mixed` temp
            // an owned value — strictly after the old value's drop
            // above, which must read the flag of the value it releases.
            if let Some(&flag) = self.trap_owner_flags.get(name) {
                if self.trap_ownership.store_is_owned(value) {
                    let owned = self.temporary_vreg();
                    self.emit(abi::move_immediate(&owned, "Integer", "1"));
                    self.emit(abi::store_u64(&owned, abi::stack_pointer(), flag));
                } else {
                    self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), flag));
                }
            }
            // A reference local never folds to a constant (see Bind).
            let constant = if by_ref {
                None
            } else {
                self.local_constant_value(value)
            };
            if let Some(local) = self.locals.get_mut(name) {
                local.constant = constant;
            }
            // A non-self-append reassignment installs a fresh tight
            // buffer; clear any stale self-append capacity headroom.
            if !by_ref {
                self.reset_string_capacity_shadow(name);
            }
        }
        Ok(())
    }

    /// Lower one `NirOp::StateAssign` (`r.state = v`, `r.state.f = v`). Runs inside `lower_ops_inner`'s
    /// per-op closure, so its `?` is attributed to the op by `nir_op_context`.
    pub(super) fn lower_state_assign_op(
        &mut self,
        resource: &str,
        value: &NirValue,
    ) -> Result<(), String> {
        // bug-424 Layer 1: a scalar `s.state.field = v` mutates the
        // existing STATE block in place; only a whole-record replace
        // (or a not-yet-in-place inlined field) falls through here.
        if self.try_inplace_scalar_fields(FieldContainer::State { resource }, value)? {
            return Ok(());
        }
        // bug-430 Layer 2, plan-145-B: `s.state.f = op(s.state.f, …)`
        // on a collection field is a field site of the self-update
        // seam; the arm that recognises `op` mutates the field
        // inside the existing STATE block and republishes it.
        if let Some((field, field_type, update)) =
            self.field_self_update_site(FieldContainer::State { resource }, value)
        {
            let site = SelfUpdateSite {
                name: resource,
                type_: field_type,
                dest: InPlaceDest::StateField {
                    resource: resource.to_string(),
                    field_index: field.field_index,
                    path: field.path_indices(),
                },
                by_ref: false,
                field: Some(field),
            };
            if self.try_inplace_self_update(&site, update)? {
                return Ok(());
            }
        }
        // plan-145-C: a collection field and scalar fields in one
        // `WITH` over the payload.
        if self.try_inplace_mixed_with(FieldContainer::State { resource }, value, None, false)? {
            return Ok(());
        }
        // Replace the resource's `STATE` payload: store the new
        // record pointer into the resource record's state slot.
        // The resource value is itself a pointer, so the update
        // is visible to the owner and every other pointer to it.
        let local = self
            .locals
            .get(resource)
            .ok_or_else(|| format!("native code state assignment unknown local '{resource}'"))?;
        let stack_offset = local.stack_offset;
        // A resource union value is a `{tag, record-ptr}` block; the
        // STATE lives in the active variant's record at `+8`
        // (plan-74). Concrete resources address their record directly.
        let resource_type = local.type_.clone();
        // bug-644: this arm REBUILDS the whole STATE record into a
        // fresh block and republishes it through the record's
        // `RESOURCE_OFFSET_STATE` slot. The block it displaces has no
        // other owner — every alias of the handle reads the slot
        // (§15), so once the slot names the new block nothing can
        // reach the old one — and it used to leak, one STATE record
        // per `s.state.field = v` that is not an in-place store
        // (32 B an iteration for a `String` field, which never takes
        // the inline-scalar fast path above).
        //
        // Freeing it is gated on two things. The size must be
        // derivable exactly as the scope-exit drop derives it
        // (`emit_free_resource_state_block` →
        // `emit_inlined_block_size_from_ptr_slot`); a STATE type that
        // sizer cannot answer for keeps the old, leaking behavior
        // instead of turning a program that builds today into a
        // codegen error. And no live `FOR EACH` may be walking this
        // resource's STATE: such a loop snapshots an alias INTO the
        // block, so freeing it mid-iteration is a use-after-free —
        // the same reason the in-place grow declines for
        // `for_each_iterable_state_fields` (bug-430). Leak it there,
        // matching the reassign arm's `for_each` carve-outs. A STATE type
        // is always a record TYPE (the front end refuses anything else,
        // TYPE_STATE_INVALID), which that sizer answers for when the model
        // knows the record.
        let replaced_state_type = resource_type.state().filter(|state| {
            self.type_model.record_fields.contains_key(state)
                && !self
                    .for_each_iterable_state_fields
                    .iter()
                    .any(|(res, _)| res == resource)
                && !self
                    .for_each_iterable_record_fields
                    .iter()
                    .any(|(base, _)| base == resource)
                && !self.for_each_iterable_locals.iter().any(|n| n == resource)
        });
        // plan-134-E: the replacement is stored into the resource.
        // bug-678: the resource keeps the POINTER, so it must own the
        // block — an aliasing source (`h.state = r`) is copied, exactly
        // as a binding's reassignment copies one. The field store
        // (`lower_value_stored_field`) returned `r`'s own block, which
        // `r` and the resource then both freed, and which an in-place
        // update of `r` changed under the resource.
        let result = self.lower_value_owned(value)?;
        // A register-native vector STATE payload materializes to its
        // block here (identity otherwise; plan-01-vector).
        let result = self.vector_value_as_block(result)?;
        // The raw block pointer is stored into the resource's STATE
        // slot (below), so this store takes ownership — claim the
        // temp so the statement-scope free never reclaims a block the
        // resource still points at (plan-25).
        self.claim_pending_temp(&result);
        // Observation boundary: a `Float` resource STATE payload
        // must be finite (plan-17).
        self.observe_float(value, &result)?;
        let value_slot = self.allocate_stack_object("state_assign_value", 8);
        self.store_value_at(&result, abi::stack_pointer(), value_slot);
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), stack_offset));
        let ptr = self.emit_resource_record_ptr(&block, &resource_type)?;
        // bug-644: capture the outgoing block BEFORE the publish
        // overwrites the slot. It is read here rather than before the
        // operands on purpose: an operand that itself reaches a STATE
        // assignment (the `G25` shape) has already republished and
        // freed by now, so the slot names the block this statement is
        // actually displacing and neither block is freed twice.
        let replaced_slot = replaced_state_type.as_ref().map(|_| {
            let slot = self.allocate_stack_object("state_assign_replaced", 8);
            let previous = self.allocate_register();
            self.emit(abi::load_u64(&previous, &ptr, RESOURCE_OFFSET_STATE));
            self.emit(abi::store_u64(&previous, abi::stack_pointer(), slot));
            slot
        });
        let val = self.allocate_register();
        self.emit(abi::load_u64(&val, abi::stack_pointer(), value_slot));
        self.emit(abi::store_u64(&val, &ptr, RESOURCE_OFFSET_STATE));
        // bug-644: reclaim the displaced block, AFTER the new one is
        // published so no window exists where the slot names freed
        // memory. Two run-time guards keep this to the one block that
        // is definitively gone:
        //   * a null STATE pointer is skipped — the first assignment
        //     on a record whose `emit_resource_state_init` has not run
        //     (or a resource handed in already drained) has nothing to
        //     free, and `arena_free(0)` faults.
        //     `emit_shallow_block_free` null-guards.
        //   * old == published is skipped — an in-place store or an
        //     `InlineGrow` realloc republishes THE SAME block, and
        //     freeing it is a use-after-free of state the resource
        //     still reads through `RESOURCE_OFFSET_STATE`. The two
        //     in-place layers above return early, so this arm should
        //     never see that, but the identity test costs two
        //     instructions and is what makes the free unconditionally
        //     safe rather than safe-by-reachability-argument.
        if let (Some(state_type), Some(replaced_slot)) = (replaced_state_type, replaced_slot) {
            let kept = self.label("state_assign_replaced_kept");
            let previous = self.allocate_register();
            self.emit(abi::load_u64(
                &previous,
                abi::stack_pointer(),
                replaced_slot,
            ));
            let published = self.allocate_register();
            self.emit(abi::load_u64(&published, abi::stack_pointer(), value_slot));
            self.emit(abi::compare_registers(&previous, &published));
            self.emit(abi::branch_eq(&kept));
            self.emit_shallow_block_free(&state_type, replaced_slot)?;
            self.emit(abi::label(&kept));
        }
        Ok(())
    }
}
