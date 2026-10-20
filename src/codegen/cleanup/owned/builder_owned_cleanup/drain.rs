//! plan-176-O: draining a scope's owned list — reclaims, the list walk and take, and dropping an
//! owned value or a loop item. The pass state and the container floats live in
//! `cleanup/owned/builder_owned_cleanup/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Drain an owned-list: walk it head-first, closing each record once. The
    /// close is closed-flag idempotent, so a record reachable through more than
    /// one path closes exactly once (§15.6). A concrete-resource element closes
    /// via its single registered close op; a resource-union element (bug-429) is
    /// tag-dispatched to the active variant's close op and its uniform STATE
    /// block freed — the same drop a lone `RES u AS Union STATE S` binding runs,
    /// via the shared `emit_union_tag_dispatch_drop`.
    ///
    /// **bug-645: the drain reclaims, it does not only close.** Every node it visits
    /// is a block `emit_owned_list_push` allocated (`{record, next}`, or
    /// `{tag, record, next}` for a resource-union leaf) and this loop is the only
    /// reader of — nothing else can name it — so that block is freed
    /// unconditionally. The blocks the node POINTS at are a different question, and
    /// answered by `cleanup.owns_elements` (`record_ownership::owning_collections`):
    /// with the collection proved the one owner, the concrete arm runs the same
    /// `emit_resource_block_reclaim` an `ActiveCleanup::Resource` drop runs, and the
    /// union arm passes its real `record_free_tags`.
    ///
    /// **bug-1053: unproved, the record is still freed, by its last holder.** The
    /// proof fails for readings that are no danger at all (a handle bound on its
    /// own, a copy that stays in the scope, a `LAMBDA` that only reads), and a
    /// close-only drain then kept every record. The record's owner word already
    /// names the one scope that closes it; what a drain cannot know is whether
    /// another cleanup of the same exit still reads that word. So a list with a
    /// `reclaim_slot` splits its drain in two ([`OwnedListPass`]): this walk
    /// closes each record its scope owns, marks it taken (an owner word no claim,
    /// float, release or drain matches) and moves its node to the reclaim list; [`Self::emit_owned_list_reclaims`] frees the taken records
    /// after every cleanup of the exit has run. A second list or binding that
    /// names the same record reads a live owner word that is not its token, and
    /// skips. A list with neither proof nor `reclaim_slot` is close-only.
    ///
    /// bug-1012: the drain frees no resource-union box. A node holds a copy of the
    /// box's two words, so the box stays with whoever made it (a binding, a
    /// statement, or the collection holding the element inline).
    pub(crate) fn emit_owned_list_drain(
        &mut self,
        cleanup: &OwnedListCleanup,
    ) -> Result<(), String> {
        if matches!(cleanup.drop, OwnedListDrop::Dynamic) {
            return self.emit_dynamic_owned_list_drain(cleanup);
        }
        let pass = match cleanup.reclaim_slot {
            Some(reclaim_slot) if !cleanup.owns_elements => OwnedListPass::Take(reclaim_slot),
            _ => OwnedListPass::Drain,
        };
        self.emit_owned_list_walk(cleanup, pass)
    }

    /// bug-1053: free what the drains of `cleanups` took. Emitted once after the
    /// whole cleanup sequence of an exit, so every cleanup that could still read a
    /// taken record's owner word has run. A list that took nothing has an empty
    /// reclaim list (its slot is zeroed with the frame and emptied by each walk).
    pub(crate) fn emit_owned_list_reclaims(
        &mut self,
        cleanups: &[ActiveCleanup],
    ) -> Result<(), String> {
        for cleanup in cleanups.iter().rev() {
            match cleanup {
                ActiveCleanup::OwnedList(list) => self.emit_owned_list_reclaim(list)?,
                ActiveCleanup::Resource(resource) => self.emit_resource_reclaim(resource)?,
                ActiveCleanup::ResourceUnion(union) => self.emit_resource_union_reclaim(union)?,
                ActiveCleanup::OwnedValue(_) => {}
            }
        }
        Ok(())
    }

    /// [`Self::emit_owned_list_reclaims`] for one list.
    pub(crate) fn emit_owned_list_reclaim(
        &mut self,
        cleanup: &OwnedListCleanup,
    ) -> Result<(), String> {
        match cleanup.reclaim_slot {
            Some(reclaim_slot) if !cleanup.owns_elements => {
                self.emit_owned_list_walk(cleanup, OwnedListPass::Reclaim(reclaim_slot))
            }
            _ => Ok(()),
        }
    }

    /// One walk over an owned-list, as `pass` says.
    fn emit_owned_list_walk(
        &mut self,
        cleanup: &OwnedListCleanup,
        pass: OwnedListPass,
    ) -> Result<(), String> {
        let loop_label = self.label("owned_list_drain_loop");
        let done_label = self.label("owned_list_drain_done");
        let close_ok = self.label("owned_list_close_ok");
        // The node pointer must survive the close and the frees below, all of which
        // clobber every caller-saved register (`.ai/compiler.md`), so it lives in a
        // slot from here on.
        let node_slot = self.allocate_stack_object("owned_list_drain_node", 8);
        let payload_slot = self.allocate_stack_object("owned_list_drain_payload", 8);
        // The slot the walk reads its next node from. A drain pops the list's own
        // head; the reclaim pops the reclaim list; a walk in place keeps the list
        // and advances a cursor of its own.
        let cursor_slot = match pass {
            OwnedListPass::Drain | OwnedListPass::Take(_) => cleanup.head_slot,
            OwnedListPass::Reclaim(reclaim_slot) => reclaim_slot,
        };
        // Whether this walk frees what a record points at, and the record.
        let frees = match pass {
            OwnedListPass::Drain => cleanup.owns_elements,
            OwnedListPass::Reclaim(_) => true,
            OwnedListPass::Take(_) => false,
        };
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), cursor_slot));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), node_slot));
        // Advance past this node before the call(s) below, which clobber
        // caller-saved registers; the loop reloads the cursor from memory. A
        // concrete node's field 0 is the record pointer to close; a union node
        // begins with its own `{tag@0, record-ptr@8}` copy of the box.
        match &cleanup.drop {
            OwnedListDrop::Dynamic => {
                return Err(format!(
                    "the dynamic owned-list of '{}' reached the per-kind drain (bug-843)",
                    cleanup.name
                ));
            }
            OwnedListDrop::Concrete {
                close,
                state_type,
                has_io_buffers,
                frees_record,
            } => {
                let close = close.clone();
                let state_type = state_type.clone();
                let has_io_buffers = *has_io_buffers;
                let frees_record = *frees_record;
                self.emit(abi::load_u64(abi::return_register(), &scratch9, 0));
                self.emit(abi::store_u64(
                    abi::return_register(),
                    abi::stack_pointer(),
                    payload_slot,
                ));
                self.emit(abi::load_u64(&scratch10, &scratch9, 8));
                self.emit(abi::store_u64(
                    &scratch10,
                    abi::stack_pointer(),
                    cursor_slot,
                ));
                let node_done = self.label("owned_list_node_done");
                // A node's record is never null in a well-formed list, but the close
                // helper dereferences the closed flag, so guard it like every other
                // resource drop (bug-246).
                self.emit(abi::compare_immediate(abi::return_register(), "0"));
                self.emit(abi::branch_eq(&node_done));
                if !matches!(pass, OwnedListPass::Reclaim(_)) {
                    // bug-755: close only a record this list's scope still owns. A
                    // record that was a reference here (owned further out) or that a
                    // `RETURN` released to the caller is someone else's to close.
                    // (The reclaim list holds only records this list took.)
                    let record = self.allocate_register();
                    self.emit(abi::move_register(&record, abi::return_register()));
                    self.emit_skip_unless_owned(
                        &abi::Reg::from(record.clone()),
                        cleanup.scope_depth,
                        &node_done,
                    )?;
                    self.emit(abi::move_register(abi::return_register(), &record));
                    self.emit_symbol_call(&close);
                    self.emit_drop_close_outcome(&close_ok);
                    self.emit(abi::label(&close_ok));
                }
                if frees {
                    // Ordered after the close, exactly as `emit_resource_cleanup_call`
                    // orders it: a `File`'s mandatory flush-on-close drains the output
                    // buffer this then frees. The helper skips a `moved` record (a
                    // thread transfer owns it now), nulls each pointer word as it goes
                    // and nulls the slot after the record free, so it is idempotent.
                    self.emit_resource_block_reclaim(
                        payload_slot,
                        state_type.as_ref(),
                        has_io_buffers,
                        frees_record,
                    )?;
                }
                if let OwnedListPass::Take(reclaim_slot) = pass {
                    self.emit_owned_list_take(payload_slot, node_slot, 8, reclaim_slot);
                    self.emit(abi::branch(&loop_label));
                }
                self.emit(abi::label(&node_done));
            }
            OwnedListDrop::Union {
                variants,
                state_type,
                io_buffer_tags,
                record_free_tags,
            } => {
                let variants = variants.clone();
                let state_type = state_type.clone();
                let io_buffer_tags = io_buffer_tags.clone();
                let record_free_tags = record_free_tags.clone();
                // bug-1012: the node's first two words ARE the box (`{tag@0,
                // record@8}`, copied when the node was pushed), so the node's own
                // address is the union pointer the dispatch reads.
                let union_ptr = self.allocate_register();
                self.emit(abi::move_register(&union_ptr, &scratch9));
                self.emit(abi::load_u64(
                    &scratch10,
                    &scratch9,
                    OWNED_LIST_UNION_NODE_NEXT_OFFSET,
                ));
                self.emit(abi::store_u64(
                    &scratch10,
                    abi::stack_pointer(),
                    cursor_slot,
                ));
                let node_done = self.label("owned_list_union_node_done");
                let dropped = self.label("owned_list_union_drop_box");
                // bug-755: as the concrete arm — the active variant's record must be
                // owned by this list's scope.
                let record = self.allocate_register();
                self.emit(abi::load_u64(&record, &union_ptr, 8));
                if matches!(pass, OwnedListPass::Take(_)) {
                    self.emit(abi::store_u64(record, abi::stack_pointer(), payload_slot));
                }
                self.emit(abi::compare_immediate(&record, "0"));
                self.emit(abi::branch_eq(&node_done));
                if !matches!(pass, OwnedListPass::Reclaim(_)) {
                    self.emit_skip_unless_owned(
                        &abi::Reg::from(record),
                        cleanup.scope_depth,
                        &node_done,
                    )?;
                }
                // The reclaim's dispatch closes again, which a closed record answers
                // with the defined no-op, and then frees what the first walk left.
                self.emit_union_tag_dispatch_drop(
                    &union_ptr,
                    &variants,
                    state_type.as_ref(),
                    // bug-1001: the buffers go with the same proof as the record.
                    if frees { &io_buffer_tags } else { &[] },
                    if frees { &record_free_tags } else { &[] },
                    &dropped,
                )?;
                // Both a matched variant (which branches here) and an unrecognised tag
                // (which falls out of the dispatch) converge here. No box is freed:
                // the list never held one (bug-1012), only the copy in its node,
                // which goes with the node below.
                self.emit(abi::label(&dropped));
                if let OwnedListPass::Take(reclaim_slot) = pass {
                    self.emit_owned_list_take(
                        payload_slot,
                        node_slot,
                        OWNED_LIST_UNION_NODE_NEXT_OFFSET,
                        reclaim_slot,
                    );
                    self.emit(abi::branch(&loop_label));
                }
                self.emit(abi::label(&node_done));
            }
        }
        // The node block itself: allocated by `emit_owned_list_push`, reachable only
        // through the chain this loop has already advanced past, so this free is
        // unaliased and once-only regardless of what the element turned out to own.
        let node_size = match &cleanup.drop {
            OwnedListDrop::Union { .. } => OWNED_LIST_UNION_NODE_SIZE,
            OwnedListDrop::Concrete { .. } | OwnedListDrop::Dynamic => 16,
        };
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            node_slot,
        ));
        self.emit(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            &node_size.to_string(),
        ));
        self.emit_arena_free_call();
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    /// bug-1053: the record in `payload_slot` is closed and this list's scope owns
    /// it. Mark it taken (`resource::owner::RESOURCE_OWNER_TAKEN`, which every
    /// other cleanup that still names the record reads and skips on) and move its
    /// node (in `node_slot`, link at `next_offset`) to the reclaim list. The word
    /// stays readable until the reclaim frees the record.
    fn emit_owned_list_take(
        &mut self,
        payload_slot: usize,
        node_slot: usize,
        next_offset: usize,
        reclaim_slot: usize,
    ) {
        let record = self.allocate_register();
        self.emit(abi::load_u64(record, abi::stack_pointer(), payload_slot));
        let taken = self.allocate_register();
        // Built as `0 - 2`: an immediate cannot be negative.
        const _: () =
            assert!(crate::codegen::resource::owner::RESOURCE_OWNER_TAKEN == 0u64.wrapping_sub(2));
        self.emit(abi::move_immediate(taken, "Integer", "0"));
        self.emit(abi::subtract_immediate(taken, taken, 2));
        self.emit(abi::store_u64(taken, record, RESOURCE_OFFSET_OWNER));
        let node = self.allocate_register();
        self.emit(abi::load_u64(node, abi::stack_pointer(), node_slot));
        let previous = self.allocate_register();
        self.emit(abi::load_u64(previous, abi::stack_pointer(), reclaim_slot));
        self.emit(abi::store_u64(previous, node, next_offset));
        self.emit(abi::store_u64(node, abi::stack_pointer(), reclaim_slot));
    }

    /// Free an owned, non-escaping flat value at scope-drop (plan-01 Phase 5 /
    /// plan-02 Phase 8): recompute the block's byte size from its static type and
    /// `arena_free(ptr, size)`. `arena_free` scrubs the bytes (zero fill),
    /// so a later use-after-free reads garbage and traps loudly. Clobbers
    /// caller-saved scratch; the caller reloads anything it needs afterward.
    pub(crate) fn emit_owned_value_drop(
        &mut self,
        cleanup: &OwnedValueCleanup,
    ) -> Result<(), String> {
        // bug-571: a `FOR EACH` loop item, whose free is guarded against the one
        // pointer the loop provably does not own.
        if let Some(alias_slot) = cleanup.loop_alias_slot {
            return self.emit_loop_item_drop(cleanup, alias_slot);
        }
        // The slot is null when the binding's initializer trapped before it was
        // stored (the slot is zero-initialized at bind, see `lower_ops`), or for
        // a moved-out value; a null free would fault scrubbing address 0, so skip.
        // The guard is only sound if the slot genuinely reads 0 on every path
        // that reaches this drop without storing a value — so register the slot
        // for the prologue zero-init (idempotent; the splice dedups). The
        // bind-site registration in `lower_ops` covers `LET` bindings but not
        // owned temporaries like a record's flat-copy: a trap route that jumps
        // past the copy leaves the slot unwritten, and the drop then frees
        // whatever the stack held (benignly 0 on AArch64 in practice; stack
        // garbage — a wild free — on x86-64).
        self.owned_value_slots.push(cleanup.stack_offset);
        // plan-134-G: a recursive value is a graph of blocks, which `_mfb_rt_graph_drop` frees
        // whole and non-recursively (plan-134-F).
        if cleanup.result_wrapper.is_none() && self.owns_graph(&cleanup.type_) {
            return self.emit_graph_value_drop(&cleanup.type_, cleanup.stack_offset);
        }
        // plan-118-E: the `String` drop is a pure function of the slot address —
        // null-test, header read, `+9`, free, null the slot — so all eleven
        // instructions live in `_mfb_rt_drop_owned_string` and the site is the
        // slot address plus a `bl`. `String` is by far the commonest owned
        // cleanup, and this shape is emitted at every exit of every scope that
        // holds one.
        // The flat collection drop shares the same treatment; the size formula
        // varies only on the entry stride and whether the block carries a
        // hash-bucket region, both compile-time constants here.
        if typed_is_collection_type(&cleanup.type_) {
            let element = typed_list_element_type(&cleanup.type_)
                .cloned()
                .unwrap_or_else(|| ParameterType::named(""));
            let stride = list_entry_stride(&element);
            let buckets = matches!(
                cleanup.type_,
                ParameterType::MapOf(..) | ParameterType::SetOf(_)
            );
            self.emit(abi::add_immediate(
                abi::c_arg(0),
                abi::stack_pointer(),
                cleanup.stack_offset,
            ));
            self.emit(abi::move_immediate(
                abi::c_arg(1),
                "Integer",
                &stride.to_string(),
            ));
            self.emit(abi::move_immediate(
                abi::c_arg(2),
                "Integer",
                if buckets { "1" } else { "0" },
            ));
            self.emit(abi::branch_link(DROP_OWNED_COLLECTION_SYMBOL));
            self.push_internal_call_relocation(DROP_OWNED_COLLECTION_SYMBOL);
            return Ok(());
        }
        if cleanup.type_ == ParameterType::String {
            // bug-560: a `String` this function self-appends to in place carries
            // geometric capacity headroom that its `byteLength` header does not
            // record, so the tight `byteLength + 9` free orphaned `spare` bytes on
            // every drop — 31 B per iteration on `out = "" / out = out & "a"`, and
            // a third of the string on a decoder that builds one and returns it.
            // The capacity-aware helper adds the shadow's spare bytes, which is
            // the size `arena_alloc` was actually given. It ADDS no free and
            // removes none: only the size argument changes, and only upward by an
            // amount this frame itself recorded.
            if let Some(capacity_slot) = cleanup.capacity_slot {
                self.emit(abi::add_immediate(
                    abi::c_arg(0),
                    abi::stack_pointer(),
                    cleanup.stack_offset,
                ));
                self.emit(abi::load_u64(
                    abi::c_arg(1),
                    abi::stack_pointer(),
                    capacity_slot,
                ));
                self.emit(abi::branch_link(DROP_OWNED_STRING_CAP_SYMBOL));
                self.push_internal_call_relocation(DROP_OWNED_STRING_CAP_SYMBOL);
                return Ok(());
            }
            self.emit(abi::add_immediate(
                abi::c_arg(0),
                abi::stack_pointer(),
                cleanup.stack_offset,
            ));
            self.emit(abi::branch_link(DROP_OWNED_STRING_SYMBOL));
            self.push_internal_call_relocation(DROP_OWNED_STRING_SYMBOL);
            return Ok(());
        }
        let skip = self.label("owned_value_free_skip");
        // This pointer is consumed as arg 0 of the arena-free call below, so emit it
        // into the C-argument token (`c_arg(0)`) — the aligned call bank — rather
        // than a result token. On AArch64/RISC-V arg and result coincide (`x0`/`a0`);
        // on x86 the aligned convention puts the C arg 0 in `rdi` directly.
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            cleanup.stack_offset,
        ));
        self.emit(abi::compare_immediate(abi::c_arg(0), "0"));
        self.emit(abi::branch_eq(&skip));
        // bug-593: an Ok wrapper whose inlined block payload a binding still
        // aliases is left alone; only the error-tagged wrapper is released.
        if cleanup.result_wrapper == Some(ResultWrapperDrop::ErrorOnly) {
            let tag = self.temporary_vreg();
            self.emit(abi::load_u64(&tag, abi::c_arg(0), 0));
            self.emit(abi::compare_immediate(&tag, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&skip));
        }
        if cleanup.result_wrapper == Some(ResultWrapperDrop::OkGraphPayload) {
            self.emit_ok_result_payload_edges_drop(&cleanup.type_, cleanup.stack_offset)?;
        }
        let size_slot = self.allocate_stack_object("owned_value_free_size", 8);
        // The slot already holds the block pointer; size it from the type.
        self.emit_inlined_block_size_from_ptr_slot(
            &cleanup.type_,
            cleanup.stack_offset,
            size_slot,
        )?;
        // plan-71-C Family-1a: reload the pointer directly into arg 0 (was
        // `return_register()`); the arena-free call consumes it as arg 0. Byte-identical;
        // clears this site's entry in the plan-71-C divergence work-list
        // (`builder_owned_cleanup.rs:201` as of `3b29873c5`).
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            cleanup.stack_offset,
        ));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit_arena_free_call();
        // Free-and-null: zero the slot after freeing so a drop that is re-reached
        // without an intervening store — a loop body whose owned temp came from a
        // conditionally/short-circuit-evaluated initializer, so this iteration
        // never re-stored the slot — sees 0 and skips instead of freeing the stale
        // (already-freed) pointer again. Without this the once-only prologue
        // zero-init is insufficient across iterations and the second free is a
        // double free that corrupts the arena (bug-440). Only the freed path needs
        // it; the null path already read 0.
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            cleanup.stack_offset,
        ));
        self.emit(abi::label(&skip));
        Ok(())
    }

    /// bug-571: drop a `FOR EACH` loop item — the `String` block the per-iteration
    /// payload load materialised, which nothing ever freed.
    ///
    /// `FOR EACH e IN xs` binds `e` to a fresh `arena_alloc` block on every
    /// iteration (`emit_load_payload_with_stride`'s `String` arm, via
    /// `emit_materialize_string_from_bytes`), because a packed `String` has no
    /// standalone header to point at. The HOF loops release the equivalent block
    /// with `free_collection_loop_item` (bug-307); the `FOR EACH` statement had no
    /// equivalent, so an 8-element `List OF String` grew arena RSS by 8 blocks per
    /// pass — 25 MB at 50 000 passes, 50 MB at 100 000.
    ///
    /// **The guard.** Adding a free is the double-free direction, and the hazard
    /// specific to a loop item is that a `FOR EACH` element is immutable and, for
    /// every payload type but `String`, IS a pointer into the container's own
    /// block. So the free does not rest on this file's copy of that enumeration
    /// staying in step with the emitter's: `alias_slot` holds the alias pointer
    /// **that same emitter** computed this iteration (`dataBase + offset`, the
    /// value its non-materialising arms return verbatim), and the free is skipped
    /// when the item IS it. That is `collections::reduce`'s model — compare the
    /// produced pointer against the value you do not own, then free only on
    /// difference — and it makes the soundness argument local to one compare
    /// instead of a whole-program claim about which element types allocate.
    ///
    /// Escape is handled before this point rather than here: an owning consumer of
    /// `e` deep-copies it (`value_needs_owning_copy` classes a `Local`/
    /// `MemberAccess` as an aliasing source), and `RETURN e` finds this very
    /// cleanup by stack offset in `plan_returned_move` and removes it for that
    /// path, moving the block to the caller instead. Registering as an ordinary
    /// [`ActiveCleanup::OwnedValue`] is what makes both of those answer correctly:
    /// they are keyed on the SLOT, and a distinct cleanup variant would have made
    /// `lower_returned_value`'s param-borrow gate read "owns no block" for a loop
    /// variable that shadows a parameter name.
    ///
    /// The free itself is the shared `String` drop, which null-tests the slot,
    /// sizes the block from its own header and NULLS the slot afterward — so an
    /// iteration reached without a re-store (there is none; the load precedes the
    /// body) frees nothing.
    fn emit_loop_item_drop(
        &mut self,
        cleanup: &OwnedValueCleanup,
        alias_slot: usize,
    ) -> Result<(), String> {
        debug_assert_eq!(
            cleanup.type_,
            ParameterType::String,
            "only the String payload arm materialises, so only it registers a loop-item drop"
        );
        self.owned_value_slots.push(cleanup.stack_offset);
        let kept = self.label("for_each_item_kept");
        let item = self.temporary_vreg();
        let alias = self.temporary_vreg();
        self.emit(abi::load_u64(
            &item,
            abi::stack_pointer(),
            cleanup.stack_offset,
        ));
        self.emit(abi::load_u64(&alias, abi::stack_pointer(), alias_slot));
        self.emit(abi::compare_registers(&item, &alias));
        self.emit(abi::branch_eq(&kept));
        self.emit(abi::add_immediate(
            abi::c_arg(0),
            abi::stack_pointer(),
            cleanup.stack_offset,
        ));
        self.emit(abi::branch_link(DROP_OWNED_STRING_SYMBOL));
        self.push_internal_call_relocation(DROP_OWNED_STRING_SYMBOL);
        self.emit(abi::label(&kept));
        Ok(())
    }
}
