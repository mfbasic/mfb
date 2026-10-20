//! plan-176-U: `lower_value` and the pending-temp machinery — pending, union-box, resource,
//! statement and fresh-string temps; claim, drop and free. The entry points and shared types
//! live in `engine/value/builder_values/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn lower_value(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        // bug-592: plan-86 E's borrow flag belongs to exactly one frame — the
        // borrowed `get` node the `Bind` arm armed it for. Taking the armed bit
        // here and clearing the flag otherwise means every operand frame below
        // lowers normally (copy + statement-scope free). Restored on the way out.
        let borrow_saved = std::mem::replace(
            &mut self.borrow_get_result,
            std::mem::take(&mut self.borrow_get_armed),
        );
        // bug-770 C: the headroom-copy flag is scoped to one frame the same way.
        let headroom_saved = std::mem::replace(
            &mut self.headroom_copy_result,
            std::mem::take(&mut self.headroom_copy_armed),
        );
        // bug-778: the unboxed trap result belongs to exactly one frame — the
        // trapped producer the unboxed `Bind` armed it for. Every operand frame
        // below sees `None`, so only that producer can fill the slots.
        let unboxed_saved = std::mem::replace(
            &mut self.unboxed_trap_active,
            self.unboxed_trap_pending.take(),
        );
        // bug-744: whether THIS frame's value is discarded (an `Eval` statement, a
        // `Nothing` function's `RETURN`). Taken here so every operand frame below is
        // a value position.
        let value_discarded = std::mem::take(&mut self.value_discarded_armed);
        // bug-760 B1: whether THIS frame's consumer reads a `Map`/`Set` binding
        // tombstone-aware (`len`, a hash lookup). Taken here so it covers exactly
        // this frame.
        let settle_exempt = std::mem::take(&mut self.map_settle_exempt_armed);
        // Track the source location of the node being lowered so that any error
        // freshly created while lowering it (overflow, divide-by-zero, helper
        // failure, conversion failure) stamps a real `ErrorLoc`. The save/restore
        // ensures that after recursively lowering operands/arguments the outer
        // node's location is back in place before its own fallible emit runs.
        let saved_loc = self.current_loc;
        if let Some(loc) = value_loc(value) {
            self.current_loc = loc;
        }
        // plan-118-A: attribute the instructions this expression emits to its
        // kind (and call target). Exclusive — an operand's own frame subtracts
        // itself from this one — so the report partitions the module.
        crate::codegen::engine::expansion::enter(
            || crate::codegen::engine::expansion::value_key(value),
            self.instructions.len(),
        );
        // bug-496: note which of this node's operands must be snapshotted before
        // a later sibling's call can reassign the storage they point into.
        let snapshot_mark = self.push_operand_snapshot_frame(value);
        // bug-536 shape B: drop any fresh-String mark left by an emitter that ran
        // outside a `lower_value` frame, so this node can only ever be credited
        // with provenance its own lowering established.
        self.fresh_string_block = None;
        let result = self.lower_value_inner(value);
        // bug-766 B: a `WITH` item built in the record-temp scratch is no block of
        // its own — nothing may free it.
        let placed_in_record_temp = self
            .record_temps
            .placed
            .take_if(|node| *node == value as *const NirValue as usize)
            .is_some();
        // bug-768: whether THIS frame read a record field or a `STATE` payload
        // (`lower_field_access_of`). Taken here, on every frame's exit, so an
        // operand's read never leaks into its consumer's frame.
        let record_member = std::mem::take(&mut self.record_member_read);
        // bug-760 B1: a `Map`/`Set` read out of a binding may hold tombstones (an
        // in-place removal leaves them there); every consumer but a
        // tombstone-aware one gets the block settled first, so no copy, iteration,
        // comparison, callee or thread ever sees one. A no-op check when the dead
        // count is 0.
        //
        // bug-768: an in-place removal at a record or `STATE` field leaves them in
        // the owner's inlined sub-block, so the same rule covers the two routes by
        // which that sub-block reaches a consumer: the field read itself
        // (`rec.xs`, `f.state.xs`, `o.inner.xs`, `o.inner`, `f.state`) and the
        // owner read whole (`LET r2 = rec`, an argument, a `RETURN`, `toString`,
        // a comparison, a store, a thread transfer). The owner read that is only
        // the base of a field read (`rec.before`) is exempt
        // (`lower_field_access_of` arms `map_settle_exempt_armed` for it), so a
        // sibling-field read never pays a settle.
        let result = match result {
            Ok(result)
                if !settle_exempt
                    && matches!(value, NirValue::Local(_) | NirValue::Global { .. })
                    && crate::codegen::collection::map::map_mutate::is_map_or_set_type(
                        &result.type_,
                    ) =>
            {
                Ok(self.emit_map_settle_value(result))
            }
            Ok(result)
                if !settle_exempt
                    && (record_member
                        || matches!(value, NirValue::Local(_) | NirValue::Global { .. })) =>
            {
                Ok(self.emit_tombstone_holder_settle(result))
            }
            other => other,
        };
        // bug-744: a `Nothing`-typed call lowers to a `void` location — the callee
        // leaves no value in `RESULT_VALUE_REGISTER`, and a statement call needs
        // none. `Nothing` is still a value (§4.6), so a `FUNC … AS Nothing` call in
        // value position (a binding, an argument, an operand, a literal element, a
        // field) yields the unit value `NOTHING`, which is 0 exactly as
        // `lower_default_value` and a `NOTHING` constant make it.
        let result = match result {
            Ok(result)
                if !value_discarded
                    && result.type_ == ParameterType::Nothing
                    && result.location == "void" =>
            {
                let register = self.allocate_register();
                self.emit(abi::move_immediate(&register, "Integer", "0"));
                Ok(ValueResult {
                    location: Operand::from(register),
                    ..result
                })
            }
            other => other,
        };
        // The mark a producer set while lowering THIS node. An operand's own
        // `lower_value` cleared and consumed its mark before returning, so what
        // survives here was set after the last operand — by this node's own
        // emitter.
        let fresh_string = self.fresh_string_block.take();
        self.operand_snapshot_wanted.truncate(snapshot_mark);
        crate::codegen::engine::expansion::exit(self.instructions.len());
        self.current_loc = saved_loc;
        let mut resource_temp = Ok(());
        if let Ok(result) = &result {
            let fresh_string = fresh_string.is_some_and(|block| block == result.location);
            if !placed_in_record_temp {
                self.register_pending_temp(value, result, fresh_string);
            }
            // bug-1012: a resource-union box this node made belongs to its
            // statement until a binding, a `RETURN` or a module-level store takes it.
            self.register_union_box_temp(value, result);
            // bug-842: a producer call's resources that this node's consumer does not
            // take are closed at the end of the statement. The root of a binding, an
            // assignment, a module-level store, an `Eval` or a `RETURN` is taken by
            // that statement itself, so only a NESTED producer is a temporary.
            let root_taken = value_discarded
                || self.resource_temp_exempt == Some(value as *const NirValue as usize);
            if !root_taken {
                resource_temp = self.register_resource_temp(value, result);
            }
        }
        let result = resource_temp.and(result);
        // bug-496: if THIS value is such an operand of the enclosing node, hand
        // its consumer a statement-scope deep copy instead of the live pointer.
        let mut result = result.and_then(|result| self.snapshot_aliased_operand(value, result));
        // Stamp the source `NirValue` so a pre-lowered `abi_inline` body can run any
        // NIR-structural analysis (bounds-check elision, float-finiteness, folding)
        // off the `ValueResult` — the value is pre-lowered, but the details are kept.
        if let Ok(vr) = &mut result {
            if vr.origin.is_none() {
                vr.origin = Some(value.clone());
            }
        }
        self.borrow_get_result = borrow_saved;
        self.headroom_copy_result = headroom_saved;
        self.unboxed_trap_active = unboxed_saved;
        result
    }

    /// Register a freshly produced, freeable-flat heap value as a statement-scope
    /// temporary to be freed unless an owner claims it (plan-25 temp-lifetime
    /// fix). Only *fresh arena blocks* qualify — exactly the values copy-insertion
    /// treats as ownable without a copy: not an aliasing source / static string
    /// (`value_needs_owning_copy`), not runtime-managed (thread-owned), and a
    /// freeable-flat type. A bare `String` additionally needs `fresh_string`
    /// provenance from its producer (bug-536 shape B; see `mark_fresh_string`).
    /// The block pointer is spilled to a fresh slot so the
    /// eventual `arena_free` survives the intervening register clobbers; the live
    /// register in `result` is left untouched for the immediate consumer.
    fn register_pending_temp(
        &mut self,
        value: &NirValue,
        result: &ValueResult,
        fresh_string: bool,
    ) {
        // plan-86 E: a read-only `get`-borrow returns an ALIAS into the container's
        // data region (not a fresh block), so it must NOT be registered for the
        // statement-scope free — freeing it would `arena_free` INTO the container and
        // corrupt the free list. The `borrow_get_result` flag is set only while
        // lowering such a borrow's initializer.
        if self.borrow_get_result {
            return;
        }
        // A register-native vector has no arena block yet; it is registered as a
        // temp only when materialized (`vector_value_as_block`), so skip it here
        // (its marker location is not a real block pointer to spill/free).
        if Self::is_vector_native(result) {
            return;
        }
        if !self.pending_temp_is_freeable(value, &result.type_, fresh_string) {
            return;
        }
        let slot = self.allocate_stack_object("pending_temp", 8);
        self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
        self.pending_temp_frees.push(PendingTemp {
            type_: result.type_.clone(),
            slot,
            location: result.location.clone(),
            shallow: false,
            resource: false,
            union_box: false,
        });
    }

    /// bug-1012: whether lowering `value` yields a resource-union `{tag, record-ptr}`
    /// box that is a fresh 16-byte arena block nothing else holds.
    ///
    /// Two producers make one. A union wrap of a resource variant allocates it. A
    /// call to a function in this program whose declared return is a resource
    /// union hands one back: its `RETURN` either moves out a box that function
    /// alone owned (`returned_resource_union_owns_box`) or copies the value into a
    /// new block (`store_pending_success_result`, for a parameter, a field, an
    /// element), so the caller is the box's only holder either way.
    ///
    /// Everything else is a view of a box someone else keeps — a local, a field
    /// (the record holds the box inline), a `collections::get` element (so does the
    /// list), a capture — or a call this cannot read (a builtin, a call through a
    /// function value), which answers `false` and keeps today's behaviour.
    pub(crate) fn value_yields_fresh_union_box(
        &self,
        value: &NirValue,
        type_: &ParameterType,
    ) -> bool {
        if !self.is_resource_union_type(type_) {
            return false;
        }
        match value {
            NirValue::UnionWrap { member_type, .. } => self.is_resource_nominal(member_type),
            NirValue::Call { target, .. } => {
                !self.locals.contains_key(target)
                    && self
                        .functions
                        .get(target.as_str())
                        .is_some_and(|f| self.resource_union_cleanup(&f.returns).is_some())
            }
            _ => false,
        }
    }

    /// bug-1012: register the resource-union box `value` just made as a
    /// statement-scope temporary.
    ///
    /// A box used to have an owner only when a binding took it. One made for a
    /// call argument, a collection element or a record field was never freed —
    /// 16 B per evaluation. Every holder that outlives the statement copies the
    /// box's two words (a collection element and a record field hold them inline,
    /// an owned-list node and a closure environment keep their own copy), so
    /// nothing reads the box after its statement and the statement frees it.
    ///
    /// An owning consumer takes it first, exactly as it takes a fresh flat block:
    /// a binding, an assignment or a module-level store through
    /// [`Self::lower_value_owned`]'s claim, a `RETURN` through
    /// [`Self::claim_union_box_temp`]. The slot joins the prologue zero-init, and
    /// the free null-guards and nulls it, so a producer a path skipped (a
    /// short-circuited operand, an earlier loop iteration) frees nothing.
    fn register_union_box_temp(&mut self, value: &NirValue, result: &ValueResult) {
        if self.borrow_get_result || !self.value_yields_fresh_union_box(value, &result.type_) {
            return;
        }
        let slot = self.allocate_stack_object("union_box_temp", 8);
        self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
        self.owned_value_slots.push(slot);
        self.pending_temp_frees.push(PendingTemp {
            type_: result.type_.clone(),
            slot,
            location: result.location.clone(),
            shallow: false,
            resource: false,
            union_box: true,
        });
    }

    /// bug-1012: a consumer that keeps `result`'s box by pointer past the statement
    /// takes it over. Unlike [`Self::claim_pending_temp`] this looks past the
    /// entries registered after the box's own (the resource temporary of the call
    /// that produced it), because a box temp's identity token is unique to it.
    pub(crate) fn claim_union_box_temp(&mut self, result: &ValueResult) {
        if let Some(index) = self
            .pending_temp_frees
            .iter()
            .rposition(|temp| temp.union_box && temp.location == result.location)
        {
            self.pending_temp_frees.remove(index);
        }
    }

    /// bug-1012: free the 16-byte box in `slot` when it holds one, and null it.
    pub(crate) fn emit_union_box_free(&mut self, slot: usize) {
        let done = self.label("union_box_free_done");
        let block = self.temporary_vreg();
        self.emit(abi::load_u64(block, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(block, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::move_register(abi::c_arg(0), block));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "16"));
        self.emit_arena_free_call();
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
        self.emit(abi::label(&done));
    }

    /// bug-842: register a producer call's resource result as a resource temporary.
    /// A resource produced inline — a call argument, the target of a member read, an
    /// element of a literal passed on — used to be claimed by nothing and closed by
    /// nothing. Its statement now closes it when it is still unowned at the end
    /// (`emit_resource_temp_close`): one a binding, a container or a global took by
    /// then has an owner and is left to it, and one a callee was handed and handed
    /// back keeps its owner.
    ///
    /// Only a call whose result IS a resource (or a resource union) is a producer
    /// here. An aggregate a call returns may be a view of resources someone else
    /// holds without an owner word to say so — `canvas::groupItems` hands back the
    /// group store's own items — so closing what it reaches is not this rule's to
    /// decide. A borrowed element (`collections::get`, a `poll`) produces nothing.
    ///
    /// The aggregate a function body returns is taken by the statement too
    /// ([`StatementTemps`], [`Self::adopt_statement_temp`]), in every position.
    fn register_resource_temp(
        &mut self,
        value: &NirValue,
        result: &ValueResult,
    ) -> Result<(), String> {
        if self.borrow_get_result {
            return Ok(());
        }
        let (NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. }) = value
        else {
            return Ok(());
        };
        if Self::target_returns_borrowed_resource(target) {
            return Ok(());
        }
        let leaf = self.is_resource_leaf(&result.type_);
        if !leaf && !self.call_releases_into_statement(value, target, &result.type_) {
            return Ok(());
        }
        let slot = self.allocate_stack_object("resource_temp", 8);
        self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
        // A slot a skipped producer (a short-circuited operand, an earlier loop
        // iteration's path) never wrote must read null, not stack garbage.
        self.owned_value_slots.push(slot);
        if leaf && !self.is_closure_leaf(&result.type_) {
            // Rule 5: the statement takes the record at once (its token is the
            // next scope depth's), so a callee it is handed to cannot take it and
            // a taker that outlives the statement raises it.
            let depth = self.current_scope_depth() + 1;
            let union = self.resource_union_cleanup(&result.type_).is_some();
            // Only the temporary that TAKES the record keeps it: a call that
            // hands back a record the statement (or anything further out)
            // already owns (`ident(mk())`) registers nothing, so one record has
            // one statement temporary and its close-and-free runs once.
            let kept = self.label("resource_temp_taken");
            let skipped = self.label("resource_temp_not_taken");
            let record = self.allocate_register();
            self.emit(abi::load_u64(record, abi::stack_pointer(), slot));
            self.emit(abi::compare_immediate(record, "0"));
            self.emit(abi::branch_eq(&kept));
            if union {
                self.emit(abi::load_u64(record, record, 8));
                self.emit(abi::compare_immediate(record, "0"));
                self.emit(abi::branch_eq(&skipped));
            }
            let owner = self.allocate_register();
            self.emit(abi::load_u64(owner, record, RESOURCE_OFFSET_OWNER));
            let token = self.emit_scope_token(depth)?;
            // Unsigned, as `emit_resource_float`: taken only when below the token.
            self.emit_branch_if_takeable(owner, token, &kept);
            self.emit(abi::label(&skipped));
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
            self.emit(abi::label(&kept));
            if union {
                self.emit_resource_union_claim_at(slot, depth)?;
            } else {
                self.emit_resource_claim_at(slot, depth)?;
            }
            self.statement_temps.leaf_depths.insert(slot, depth);
        }
        if !leaf {
            self.adopt_statement_temp(slot, result)?;
            // The adoption allocated list nodes, and an arena call destroys every
            // caller-saved register: hand the consumer the value back from its slot.
            self.emit(abi::load_u64(
                abi::Reg::from(&result.location),
                abi::stack_pointer(),
                slot,
            ));
        }
        let token = self.allocate_register();
        self.pending_temp_frees.push(PendingTemp {
            type_: result.type_.clone(),
            slot,
            location: Operand::from(token),
            shallow: false,
            resource: true,
            union_box: false,
        });
        Ok(())
    }

    /// Rule 5 (`mfb spec language resource-management`, "The owner of a resource"):
    /// whether the call `value` (to `target`, answering `type_`) can hand the
    /// caller's statement resources a `RETURN` released: its result is an
    /// aggregate reaching a resource, and a function body ran to produce it. That
    /// is a call to a function with a body, a call through a function value, and
    /// a natively lowered builtin handed a function value (it runs the callback
    /// in this statement, and the callback's `RETURN` releases into the result:
    /// `collections::transform(xs, open)`). How the function was reached, and
    /// which expression the call sits in, is not asked.
    ///
    /// A natively lowered builtin that runs no function has no `RETURN`: what
    /// its result reaches already has a cleanup scope (a view,
    /// `canvas::groupItems`; the caller's own handles, `collections::take`).
    fn call_releases_into_statement(
        &self,
        value: &NirValue,
        target: &str,
        type_: &ParameterType,
    ) -> bool {
        if !self.type_carries_resources(type_) {
            return false;
        }
        if self.locals.contains_key(target) || self.functions.contains_key(target) {
            return true;
        }
        let (NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }) = value
        else {
            return false;
        };
        args.iter().any(|arg| match arg {
            NirValue::Closure { .. } | NirValue::FunctionRef { .. } => true,
            NirValue::Local(name) => self
                .locals
                .get(name)
                .is_some_and(|local| matches!(local.type_, ParameterType::Func(..))),
            _ => false,
        })
    }

    /// Rule 5: the statement takes, at once, every resource the call result in
    /// `slot` reaches that the callee released. The statement is a scope one
    /// inside the current one (its token is the next depth's), so every taker
    /// that outlives it (a binding, a store into a container some binding holds,
    /// a module-level store, a `RETURN`) raises what it keeps past this token by
    /// the ordinary float, and nothing a callee does can take it. Each record
    /// taken is listed, one list per resource leaf kind; a record that already
    /// has an owner is left to it and not listed.
    fn adopt_statement_temp(&mut self, slot: usize, result: &ValueResult) -> Result<(), String> {
        let type_ = &result.type_;
        let depth = self.current_scope_depth() + 1;
        let mut lists = Vec::new();
        for leaf in self.resource_leaf_types(type_) {
            let drop = self.resource_leaf_drop(&leaf)?;
            let head_slot = self.allocate_stack_object("resource_temp_owned", 8);
            // Empty on every path that does not reach the producer, like `slot`.
            self.owned_value_slots.push(head_slot);
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), head_slot));
            lists.push(OwnedListCleanup {
                name: String::new(),
                head_slot,
                drop,
                owns_elements: true,
                reclaim_slot: None,
                scope_depth: depth,
                leaf,
            });
        }
        // A function value the result reaches carries its resources in its
        // environment: they are taken through its routine, onto a dynamic list.
        let dynamic = if self.type_reaches_closure_leaf(type_) {
            let head_slot = self.allocate_stack_object("resource_temp_dynamic", 8);
            self.owned_value_slots.push(head_slot);
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), head_slot));
            lists.push(OwnedListCleanup {
                name: String::new(),
                head_slot,
                drop: OwnedListDrop::Dynamic,
                owns_elements: false,
                reclaim_slot: None,
                scope_depth: depth,
                leaf: ParameterType::Unknown,
            });
            Some(crate::codegen::resource::closure_carry::DynamicList {
                head_slot,
                scope_depth: depth,
            })
        } else {
            None
        };
        let heads: Vec<(ParameterType, usize, bool)> = lists
            .iter()
            .filter(|list| !matches!(list.drop, OwnedListDrop::Dynamic))
            .map(|list| {
                (
                    list.leaf.clone(),
                    list.head_slot,
                    matches!(list.drop, OwnedListDrop::Union { .. }),
                )
            })
            .collect();
        self.emit_for_each_resource(slot, type_, &mut |builder, leaf_slot, leaf| {
            let leaf = crate::codegen::engine::types::typed_strip_res_marker(leaf);
            if builder.is_closure_leaf(leaf) {
                let Some(list) = dynamic else {
                    return Ok(());
                };
                let (address_slot, token_slot) = builder.emit_dynamic_list_arguments(list)?;
                return builder.emit_closure_resource_op(
                    leaf_slot,
                    CLOSURE_ENV_OP_ADOPT,
                    &[address_slot, token_slot],
                );
            }
            let Some((_, head_slot, union)) =
                heads.iter().find(|(kind, _, _)| kind == leaf).or_else(|| {
                    heads
                        .iter()
                        .find(|(kind, _, _)| kind.without_state() == leaf.without_state())
                })
            else {
                return Err(format!(
                    "resource temporary reaches '{leaf}', which it has no list for"
                ));
            };
            builder.emit_owned_list_push_node(
                *head_slot,
                Some((depth, *union)),
                *union,
                leaf_slot,
                true,
                None,
            )
        })?;
        self.statement_temps.drops.insert(slot, lists);
        Ok(())
    }

    /// Rule 5: the statement-end drain of what a call result handed the statement
    /// ([`Self::adopt_statement_temp`]): close each record the statement still
    /// owns, then free its buffers, `STATE` and record. Every step nulls what it
    /// consumed, so a drain reached again on this slot (another exit path, a loop
    /// that skipped the producer) does nothing. The result's own block is a flat
    /// temporary with its own free.
    fn emit_statement_temp_drop(&mut self, temp: &PendingTemp) -> Result<bool, String> {
        let Some(lists) = self.statement_temps.drops.get(&temp.slot).cloned() else {
            return Ok(false);
        };
        for list in &lists {
            self.emit_owned_list_drain(list)?;
        }
        Ok(true)
    }

    /// Whether a fresh block produced by `value` (of static type `type_`) may be
    /// freed by THIS frame at statement scope — the one ownership question behind
    /// [`Self::register_pending_temp`], extracted so the inline-`TRAP` `Result`
    /// sites can ask it with the identical rules (bug-561).
    ///
    /// A bare `String` result is freed only with **provenance** (bug-536 shape B).
    /// A record/union/Result/collection temp is a self-contained fresh arena block
    /// (a nested `String` field is byte-inlined, so one `arena_free` reclaims it),
    /// but a *standalone* `String` produced by a call may be a shared rodata
    /// constant NOT loaded through the tracked static-string path, or a non-owned
    /// view into an argument — freeing one is a wild `arena_free` (SIGBUS on
    /// rodata, free-list corruption on a borrow). plan-25 therefore exempted every
    /// String, which made `acc = acc + len(toString(i))` leak 64 bytes per
    /// evaluation for the life of the process.
    ///
    /// `fresh_string` is set only when the shared String producers
    /// (`emit_materialize_string_from_bytes`, `_mfb_rt_string_concat`, the
    /// `toString` formatter helpers — each of which returns the block of an
    /// `arena_alloc` it just made) marked THIS node's own result operand. It is
    /// fail-closed: a producer that does not mark keeps the old leak, and no
    /// unmarked value can ever be freed.
    ///
    /// bug-536 shape B-2: a *native* producer marks its own block, but a
    /// `.mfb`-bodied / user callee's block is made inside the callee, where no
    /// mark can reach this frame. `call_returns_fresh_string` is that callee's own
    /// promise, read from its NIR before any lowering runs, so it is
    /// order-independent and identical to the promise the callee delivered.
    pub(crate) fn pending_temp_is_freeable(
        &self,
        value: &NirValue,
        type_: &ParameterType,
        fresh_string: bool,
    ) -> bool {
        if !(self.is_freeable_flat_value(type_) || self.owns_graph(type_))
            || self.value_needs_owning_copy(value)
            || Self::value_is_runtime_managed(value)
        {
            return false;
        }
        // bug-781: a function reference (or a capture-less closure) is the static
        // descriptor every reference shares — no block, so nothing for a temp to free.
        if matches!(value, NirValue::FunctionRef { .. })
            || matches!(value, NirValue::Closure { captures, .. } if captures.is_empty())
        {
            return false;
        }
        if *type_ == ParameterType::String
            && !fresh_string
            && !self.call_returns_fresh_string(value)
        {
            return false;
        }
        true
    }

    /// bug-536 shape B: record that `block` names a `String` arena block this
    /// builder has **just allocated** and that no other owner holds — the sole
    /// way a bare `String` temp becomes eligible for the statement-scope free.
    ///
    /// Call it only from a producer that (a) allocated the block itself through
    /// `_mfb_arena_alloc` on the path that reaches this call, and (b) returns
    /// exactly this operand as the value's result. `lower_value` re-checks (b) by
    /// comparing the mark against the node's own `ValueResult.location`, so a
    /// producer that materializes an *interior* String (a pad character, a merge
    /// key written into a payload) and returns something else is not opted in —
    /// its mark simply fails to match and the value keeps leaking, which is the
    /// safe direction.
    pub(crate) fn mark_fresh_string(&mut self, block: impl Into<Operand>) {
        self.fresh_string_block = Some(block.into());
    }

    /// Register an **interior** fresh `String` block — one a lowering allocates
    /// for its own use and never returns — for the statement-scope free.
    ///
    /// `mark_fresh_string` cannot cover these: it is honoured only when the
    /// marked operand IS the value's result, and an interior block by definition
    /// is not. Without this the block has no owner at all and leaks for the life
    /// of the process (`strings::padLeft(s, n)` with the default padChar leaked
    /// its one-byte pad String on every call — bug-536 shape B). The caller must
    /// have allocated the block itself on the path reaching this call, exactly as
    /// for `mark_fresh_string`.
    pub(crate) fn register_fresh_string_temp(&mut self, block: impl Into<Operand>) {
        let location = block.into();
        let slot = self.allocate_stack_object("pending_temp", 8);
        self.emit(abi::store_u64(&location, abi::stack_pointer(), slot));
        self.pending_temp_frees.push(PendingTemp {
            type_: ParameterType::String,
            slot,
            location,
            shallow: false,
            resource: false,
            union_box: false,
        });
    }

    /// bug-561: the sibling of [`Self::register_call_result_payload_temp`] for the
    /// raw inline-builtin `TRAP` paths, where the producer's block is still in a
    /// register rather than a slot.
    ///
    /// `lower_inline_builtin_raw` / `lower_inline_infallible_raw` run the member's
    /// ordinary lowering directly, BYPASSING `lower_value` — which is the only
    /// place `register_pending_temp` is called. So a member that allocates a fresh
    /// block (`strings::mid`, `collections::get` on a `List OF String`) had its
    /// block copied into the `Result` and then abandoned, where the same member
    /// outside a `TRAP` is freed at statement end. This re-runs the registration
    /// the bypass skipped, on the exact node the non-raw path would have carried,
    /// so the provenance answers are identical.
    ///
    /// Emit only on the member's success fall-through: an error exit has already
    /// branched to the capture label, so the spill below never runs on it.
    pub(super) fn register_raw_member_result_temp(
        &mut self,
        target: &str,
        args: &[NirValue],
        success: &ValueResult,
    ) {
        // The mark the member's own producer set, matched against its result
        // operand exactly as `lower_value` matches it (bug-536 shape B).
        let fresh_string = self
            .fresh_string_block
            .take()
            .is_some_and(|block| block == success.location);
        let node = NirValue::Call {
            target: target.to_string(),
            args: args.to_vec(),
            loc: self.current_loc,
        };
        self.register_pending_temp(&node, success, fresh_string);
    }

    /// bug-561: register the block a fallible callee returned as a
    /// statement-scope temp, from the SLOT it was spilled into.
    ///
    /// The inline-`TRAP` desugar binds `$trap_resN : Result OF T = CallResult(f(..))`,
    /// and the lowering builds that `Result` by **copying** the callee's block
    /// into a freshly allocated `{tag, size, payload}` block
    /// (`emit_build_result_inline`). The callee's own block is dead the instant
    /// that copy finishes — and nothing freed it, so every fallible call in an
    /// expression leaked its whole payload: measured 64 B per call for a `String`
    /// and 256 B for a `List OF Integer`, 25 MB / 50 MB at 200k / 400k iterations.
    /// A scalar payload has no block, which is why `Result OF Integer` never
    /// leaked and the bug looked type-independent when it is not.
    ///
    /// The provenance is [`Self::pending_temp_is_freeable`] — the same question,
    /// with the same answers, that the plain-`Call` path already asks before
    /// freeing a call result. A `String` therefore still needs the callee's own
    /// `function_returns_fresh_string` promise, a `thread.*` result is still
    /// runtime-managed and untouched, and a param-borrow or rodata result is
    /// still copied rather than freed. Anything unproven keeps leaking.
    ///
    /// **This must be emitted on the Ok path only.** `source_slot` holds the raw
    /// success register, which on the error path holds an error code, not a
    /// block. Registering here spills into a slot written only on that path; the
    /// statement-end drop null-guards and nulls it (bug-246 / bug-440), exactly
    /// as it does for any conditionally-initialized owned temp, so an iteration
    /// that fails frees nothing.
    pub(crate) fn register_call_result_payload_temp(
        &mut self,
        value: &NirValue,
        type_: &ParameterType,
        source_slot: usize,
    ) {
        if self.borrow_get_result {
            return;
        }
        if !self.pending_temp_is_freeable(value, type_, false) {
            return;
        }
        // bug-781: a function-value payload is not copied into the `Result` — its word
        // IS the callee's closure pointer, which the wrapper now owns. Nothing is dead.
        if !self.result_payload_is_block(type_) {
            return;
        }
        let slot = self.allocate_stack_object("call_result_payload_temp", 8);
        let pointer = self.allocate_register();
        self.emit(abi::load_u64(&pointer, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(&pointer, abi::stack_pointer(), slot));
        // The identity token is this fresh register, which no owner's
        // `ValueResult` names — the `Result` block the enclosing node yields is a
        // different operand — so `claim_pending_temp` can never mistake this temp
        // for the one an owning binding took over.
        // plan-134: a recursive payload's top block was byte-copied into the `Result`, so
        // the graph it points into moved with those bytes — only the block itself is dead.
        let shallow = !self.is_freeable_flat_value(type_) && self.owns_graph(type_);
        self.pending_temp_frees.push(PendingTemp {
            type_: type_.clone(),
            slot,
            location: Operand::from(pointer),
            shallow,
            resource: false,
            union_box: false,
        });
    }

    /// Re-identify the most recently registered pending temp, because an
    /// **identity** lowering is about to return the same block under a different
    /// operand.
    ///
    /// A `PendingTemp` carries two things: the `slot` its `arena_free` reads (the
    /// block pointer) and the `location` that says which `ValueResult` *is* that
    /// block. Only `location` is an identity token, and `claim_pending_temp` /
    /// `lower_returned_value` compare against it to decide that an owner has taken
    /// the block over. So a lowering that hands back its argument's block under a
    /// NEW operand silently breaks the chain: the owner's claim no longer matches,
    /// the statement-scope free still runs, and the owner is left holding freed
    /// memory.
    ///
    /// That is not hypothetical — it is a **use-after-free** that predates
    /// bug-536 shape B-2 and is fixed here because B-2 makes it reachable from far
    /// more programs. `toString`'s `String` arm is the identity (it returns its own
    /// argument), and it spills the argument and reloads it into a fresh register,
    /// so `LET a AS String = toString("x" & toString(i))` bound `a` to the concat
    /// block, freed that block at statement end, and read it back after reuse:
    /// measured on the pre-fix compiler as a wrong value and then a SIGSEGV.
    ///
    /// Retargeting emits **nothing** — the free still reads the same slot — it only
    /// moves the identity token forward so the one owner can still claim the one
    /// block. Gated on the tail entry actually being this argument's registration
    /// (`from`), so an identity applied to a value that registered no temp cannot
    /// steal an enclosing expression's sibling temp.
    pub(crate) fn retarget_pending_temp(&mut self, from: &Operand, to: &Operand) {
        if let Some(temp) = self.pending_temp_frees.last_mut() {
            if temp.location == *from {
                temp.location = to.clone();
            }
        }
    }

    /// Exempt the just-produced temporary from the statement-scope free because an
    /// owning consumer (a binding, a `RETURN`, a resource `STATE` store, a
    /// thread-spawn move) now owns its block and will free it exactly once. The
    /// outermost node's temp is always the most recently registered, so matching
    /// the tail entry's origin register is precise.
    /// Whether [`Self::claim_pending_temp`] would claim `result` — i.e. it is the
    /// statement's most recent pending temporary, a block this frame freshly allocated
    /// and still owns.
    ///
    /// bug-655: `thread.start` may only hand the seed block's ownership to the thread
    /// when this is true. A seed that is a STRING LITERAL is a static symbol, not arena
    /// memory (freeing it is a bus error), and a seed that is a `Local` is still owned
    /// by that binding, whose own scope-drop frees it (freeing it again is a double free
    /// — segfault). Both were measured before this gate existed; neither is hypothetical.
    pub(crate) fn pending_temp_would_be_claimed(&self, result: &ValueResult) -> bool {
        self.pending_temp_frees
            .last()
            .is_some_and(|temp| temp.location == result.location)
    }

    pub(crate) fn claim_pending_temp(&mut self, result: &ValueResult) {
        if self
            .pending_temp_frees
            .last()
            .is_some_and(|temp| temp.location == result.location)
        {
            self.pending_temp_frees.pop();
        }
    }

    /// Free every pending temporary registered above `watermark`, most-recent
    /// first (the scope-drop convention). Reuses the owned-value drop (null-guard +
    /// type-sized `arena_free`).
    pub(crate) fn drop_pending_temps_to(&mut self, watermark: usize) -> Result<(), String> {
        while self.pending_temp_frees.len() > watermark {
            let temp = self
                .pending_temp_frees
                .pop()
                .expect("watermark within bounds");
            self.emit_pending_temp_free(&temp)?;
        }
        Ok(())
    }

    /// [`Self::drop_pending_temps_to`] for a record constructor's argument temps: the
    /// record byte-inlines each argument block, so those blocks are dead — but the
    /// resources an argument carries are now the record's references, still live, so
    /// a resource temporary (bug-842) stays registered and is closed with the rest
    /// of its statement's temps.
    pub(crate) fn drop_arg_block_temps_to(&mut self, watermark: usize) -> Result<(), String> {
        let watermark = watermark.min(self.pending_temp_frees.len());
        let above = self.pending_temp_frees.split_off(watermark);
        let mut kept = Vec::new();
        for temp in above.into_iter().rev() {
            // bug-1012: a box temporary stays too — the resource temporary of the
            // call that returned it reads the record through it at statement end.
            if temp.resource || temp.union_box {
                kept.push(temp);
            } else {
                self.emit_pending_temp_free(&temp)?;
            }
        }
        kept.reverse();
        self.pending_temp_frees.extend(kept);
        Ok(())
    }

    /// Free every pending temporary on the path being emitted WITHOUT forgetting them: a
    /// failing call's error exit leaves the statement, so the statement-end drop the
    /// success path still reaches never runs on it. Each free nulls its slot, and every
    /// slot joins the prologue zero-init, so a temp that path never wrote is skipped.
    pub(crate) fn emit_pending_temp_frees_in_place(&mut self) -> Result<(), String> {
        self.emit_pending_temp_frees_in_place_from(0)
    }

    /// [`Self::emit_pending_temp_frees_in_place`] for the temps above `depth` only: an
    /// `EXIT`/`CONTINUE` leaves the statements inside its loop's body, not the loop (bug-620).
    pub(crate) fn emit_pending_temp_frees_in_place_from(
        &mut self,
        depth: usize,
    ) -> Result<(), String> {
        let depth = depth.min(self.pending_temp_frees.len());
        let temps = self.pending_temp_frees[depth..].to_vec();
        for temp in temps.iter().rev() {
            self.emit_pending_temp_free(temp)?;
        }
        Ok(())
    }

    /// [`Self::emit_pending_temp_frees_in_place`] for the temps below `depth` only:
    /// the enclosing statements' temps, which a `FAIL` jumps past (bug-780). The
    /// `FAIL`'s own temps sit above `depth` and are the catcher's to adopt.
    pub(crate) fn emit_pending_temp_frees_in_place_below(
        &mut self,
        depth: usize,
    ) -> Result<(), String> {
        let depth = depth.min(self.pending_temp_frees.len());
        let temps = self.pending_temp_frees[..depth].to_vec();
        for temp in temps.iter().rev() {
            self.emit_pending_temp_free(temp)?;
        }
        Ok(())
    }

    fn emit_pending_temp_free(&mut self, temp: &PendingTemp) -> Result<(), String> {
        // bug-842: a resource temporary closes its resource when nobody took it —
        // unless this path's `RETURN` already did — and owns no block to free. The
        // slot is nulled so a loop that re-reaches this drop without the producer
        // running again closes nothing.
        if temp.resource {
            // bug-999: a temporary the statement owns outright frees what it
            // closes. Its drop is the same on every path, a `RETURN`'s included:
            // nothing a returned value carries can be among its records.
            if self.emit_statement_temp_drop(temp)? {
                self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), temp.slot));
                return Ok(());
            }
            if !self.resource_temps_closed_on_path {
                let depth = self.statement_temps.leaf_depths.get(&temp.slot).copied();
                self.emit_resource_temp_close(temp.slot, &temp.type_, depth)?;
            }
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), temp.slot));
            return Ok(());
        }
        if temp.union_box {
            self.emit_union_box_free(temp.slot);
            return Ok(());
        }
        if temp.shallow {
            return self.emit_shallow_block_free(&temp.type_, temp.slot);
        }
        self.emit_owned_value_drop(&OwnedValueCleanup {
            type_: temp.type_.clone(),
            stack_offset: temp.slot,
            capacity_slot: None,
            loop_alias_slot: None,
            result_wrapper: None,
        })
    }

    /// bug-842: close every pending resource temporary now, at a `RETURN` whose
    /// value can carry resources (after `emit_claim_returned_value`, before the
    /// release). The interior drop this path emits next only nulls their slots.
    pub(crate) fn close_resource_temps_before_return(&mut self) -> Result<(), String> {
        let temps = self.pending_temp_frees.clone();
        for temp in temps.iter().rev() {
            // A statement-owned aggregate is drained whole by the interior
            // drop this path emits next, which does not consult the flag below.
            if temp.resource && !self.statement_temps.drops.contains_key(&temp.slot) {
                let depth = self.statement_temps.leaf_depths.get(&temp.slot).copied();
                self.emit_resource_temp_close(temp.slot, &temp.type_, depth)?;
            }
        }
        // The list itself stays: an enclosing statement's temps are still its own on
        // every path that does not take this `RETURN`. Only the interior drop this
        // path emits next must not close them again.
        self.resource_temps_closed_on_path = true;
        Ok(())
    }

    /// Discard pending temporaries above `watermark` WITHOUT freeing them.
    ///
    /// bug-567 corrected the reasoning this used to carry. The old comment gave two
    /// justifications for truncating **every** temp a control-transfer statement
    /// registered, and only the first is true:
    ///
    /// * *"a returned temp is moved to the caller"* — true, but only of the ONE
    ///   temp [`Self::claim_pending_temp`] has already popped.
    /// * *"an interior free would be unreachable dead code after the branch"* —
    ///   **false.** It is unreachable only because it would be emitted *after* the
    ///   branch, which is a property of where the free is placed, not of the
    ///   program. `RETURN "v" & toString(n MOD 10)` leaked the inner `toString`
    ///   block, 64 B per call, while `RETURN s & ">"` — one operator fewer, so no
    ///   interior temp — was flat.
    ///
    /// So this is now reachable only from the one call site in `lower_ops_inner`,
    /// and only for a [`TransferTemps`] class where forgetting is the *correct*
    /// answer: the block is adopted by another owner (`Fail`) or the process ends
    /// (`ExitProgram`). A `RETURN` frees its own interior temps before it branches
    /// ([`Self::drop_interior_temps_before_branch`]), and reaching here with any
    /// left is reported as a codegen error rather than silently truncated.
    pub(crate) fn clear_pending_temps_to(&mut self, watermark: usize) {
        self.pending_temp_frees.truncate(watermark);
    }

    /// bug-567: free the pending temporaries a control-transfer statement's own
    /// expression left behind — the INTERIOR ones, that no owner claimed — at a
    /// point that is still reachable, with the escaping value parked across the
    /// `arena_free` calls (which clobber every caller-saved register).
    ///
    /// The soundness argument is not a new one: it is exactly the argument that
    /// licenses [`Self::drop_pending_temps_to`] at the end of every *ordinary*
    /// statement. A registered pending temp is by construction a fresh, solely
    /// owned arena block — [`Self::register_pending_temp`] admits nothing else —
    /// and `LET x AS String = wrap("ab")` already frees the identical interior
    /// block at statement scope. The only thing a `RETURN` changed was *placement*.
    ///
    /// The caller must therefore call this only once the escaping value is
    /// **standalone**: claimed, moved (`plan_returned_move`), or deep-copied by
    /// `lower_returned_value` / `store_pending_success_result`. `escaping` says
    /// where that value currently lives; the returned operand is where it lives
    /// afterwards, and is `None` unless it had to be parked and reloaded here.
    ///
    /// Belt and braces on top of that argument, in the shape bugs 565/569/571/572
    /// established: **every** interior free is guarded by a runtime
    /// pointer-identity compare against the escaping block, so even a lowering that
    /// handed the return the same pointer as an interior temp cannot have it freed
    /// underneath the caller. Soundness is local, not a whole-program proof — and
    /// it is uniform across both `RETURN` lowerings, because the guard reads a
    /// SLOT and both paths have one (this routine parks a register-resident value;
    /// the cleanup-bearing path hands over the slot
    /// `store_pending_success_result` already wrote).
    ///
    /// bug-620: the temps BELOW `watermark` belong to the statements enclosing the `RETURN` —
    /// an `IF` whose condition made one — whose statement-end drops the `ret` jumps past.
    /// They are freed here too, with the same guard, but in place: each free nulls its slot
    /// and the fall-through path still owns them.
    ///
    /// Emits **nothing at all** when no temp is pending, which is every `RETURN` in
    /// the tree bar the concat shapes and the ones inside such a statement — so
    /// codegen is byte-identical wherever the bug was not.
    pub(crate) fn drop_interior_temps_before_branch(
        &mut self,
        watermark: usize,
        escaping: EscapingValue,
    ) -> Result<Option<Operand>, String> {
        if self.pending_temp_frees.is_empty() {
            self.resource_temps_closed_on_path = false;
            return Ok(match escaping {
                EscapingValue::InRegister(location) => Some(location),
                EscapingValue::InSlot(_) | EscapingValue::None => None,
            });
        }
        // The slot the guard compares against. A register-resident value is spilled
        // to one here, because `arena_free` clobbers every caller-saved register,
        // and reloaded afterwards; a value the caller already stored is guarded
        // against its existing slot and needs neither.
        let (parked, reload) = match escaping {
            EscapingValue::InRegister(location) => {
                let slot = self.allocate_stack_object("return_escaping_value", 8);
                self.emit(abi::store_u64(&location, abi::stack_pointer(), slot));
                (Some(slot), true)
            }
            EscapingValue::InSlot(slot) => (Some(slot), false),
            EscapingValue::None => (None, false),
        };
        let interior = self
            .pending_temp_frees
            .split_off(watermark.min(self.pending_temp_frees.len()));
        let enclosing = self.pending_temp_frees.clone();
        for temp in interior
            .into_iter()
            .rev()
            .chain(enclosing.into_iter().rev())
        {
            // A resource temporary's slot is no block of its own to compare: what
            // it drops is decided by the owner word of each record (a record the
            // returned value carries was raised past the statement), and a
            // statement list's nodes are freed either way. Comparing its slot let
            // a returned block that reused a freed argument block's address skip
            // the drain.
            let kept = match parked.filter(|_| !temp.resource) {
                Some(escaping_slot) => {
                    let kept = self.label("return_temp_escaped");
                    let block = self.temporary_vreg();
                    let escaped = self.temporary_vreg();
                    self.emit(abi::load_u64(&block, abi::stack_pointer(), temp.slot));
                    self.emit(abi::load_u64(&escaped, abi::stack_pointer(), escaping_slot));
                    self.emit(abi::compare_registers(&block, &escaped));
                    self.emit(abi::branch_eq(&kept));
                    Some(kept)
                }
                None => None,
            };
            self.emit_pending_temp_free(&temp)?;
            if let Some(kept) = kept {
                self.emit(abi::label(&kept));
            }
        }
        self.resource_temps_closed_on_path = false;
        if !reload {
            return Ok(None);
        }
        let slot = parked.expect("a reloadable escaping value was parked");
        let reloaded = self.allocate_register();
        self.emit(abi::load_u64(&reloaded, abi::stack_pointer(), slot));
        Ok(Some(Operand::from(reloaded)))
    }
}
