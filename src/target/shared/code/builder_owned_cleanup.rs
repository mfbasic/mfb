use super::*;

impl CodeBuilder<'_> {
    /// Allocate and register a runtime owned-list for an owner collection binding
    /// (§15.6): a head-pointer stack slot (initialized empty) plus an
    /// [`ActiveCleanup::OwnedList`] obligation drained on every exit path.
    pub(super) fn setup_owned_list(&mut self, name: &str, type_: &str) -> Result<(), String> {
        let close_symbol = self.collection_resource_close_symbol(type_)?;
        let head_slot = self.allocate_stack_object(&format!("owned_list_{name}"), 8);
        let scratch9 = self.temporary_vreg();
        self.emit(abi::move_immediate(&scratch9, "Integer", "0"));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), head_slot));
        self.owned_list_heads.insert(name.to_string(), head_slot);
        self.active_cleanups
            .push(ActiveCleanup::OwnedList(OwnedListCleanup {
                name: name.to_string(),
                head_slot,
                close_symbol,
            }));
        Ok(())
    }

    /// Transfer a returned resource collection's owned-list to the caller: drop
    /// its drain obligation from this scope so the resources are not closed here
    /// (the caller's scope adopts and closes them). Other scopes' owned-lists are
    /// untouched (§15.6).
    pub(super) fn deactivate_owned_list(&mut self, name: &str) {
        if let Some(index) = self
            .active_cleanups
            .iter()
            .rposition(|cleanup| matches!(cleanup, ActiveCleanup::OwnedList(o) if o.name == name))
        {
            self.active_cleanups.remove(index);
        }
    }

    /// Whether a NIR type string is a `RES`-marked resource collection
    /// (`List OF RES File`, `Map OF K TO RES File`): its scope-ownership transfers
    /// across a function boundary (§15.6).
    pub(super) fn is_res_marked_resource_collection(type_: &str) -> bool {
        type_
            .strip_prefix("List OF ")
            .is_some_and(|e| e.starts_with("RES "))
            || type_
                .strip_prefix("Map OF ")
                .and_then(|rest| rest.split_once(" TO "))
                .is_some_and(|(_, value)| value.starts_with("RES "))
    }

    /// Push the resource record at `resource_slot` onto `collection`'s owned-list
    /// as a fresh `{record, next}` node (§15.6).
    pub(super) fn emit_owned_list_push(
        &mut self,
        collection: &str,
        resource_slot: usize,
    ) -> Result<(), String> {
        let head_slot = *self
            .owned_list_heads
            .get(collection)
            .ok_or_else(|| format!("resource floats to '{collection}', which has no owned-list"))?;
        // Allocate a 16-byte node (record ptr at 0, next at 8).
        let alloc_ok = self.label("owned_list_alloc_ok");
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::move_immediate(abi::return_register(), "Integer", "16"));
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        // x1 = node pointer.
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            resource_slot,
        ));
        self.emit(abi::store_u64(&scratch9, abi::RET[1], 0));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), head_slot));
        self.emit(abi::store_u64(&scratch10, abi::RET[1], 8));
        self.emit(abi::store_u64(abi::RET[1], abi::stack_pointer(), head_slot));
        Ok(())
    }

    /// Adopt the resources of a `List OF RES File` value transferred in from a
    /// call: walk the collection and push each element record onto this scope's
    /// owned-list, so the scope closes each once at exit (§15.6).
    pub(super) fn emit_owned_list_seed_from_collection(
        &mut self,
        collection: &str,
        collection_slot: usize,
        element_type: &str,
    ) -> Result<(), String> {
        let cursor_slot = self.allocate_stack_object("adopt_cursor", 8);
        let remaining_slot = self.allocate_stack_object("adopt_remaining", 8);
        let elem_slot = self.allocate_stack_object("adopt_elem", 8);
        self.initialize_collection_loop_slots(
            collection_slot,
            cursor_slot,
            remaining_slot,
            element_type,
        );
        let loop_label = self.label("owned_list_seed_loop");
        let done_label = self.label("owned_list_seed_done");
        let scratch9 = self.temporary_vreg();
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&done_label));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), elem_slot));
        self.emit_owned_list_push(collection, elem_slot)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, element_type);
        self.emit(abi::label(&done_label));
        Ok(())
    }

    /// Drain an owned-list: walk it head-first, closing each record once. The
    /// close is closed-flag idempotent, so a record reachable through more than
    /// one path closes exactly once (§15.6).
    pub(super) fn emit_owned_list_drain(
        &mut self,
        cleanup: &OwnedListCleanup,
    ) -> Result<(), String> {
        let loop_label = self.label("owned_list_drain_loop");
        let done_label = self.label("owned_list_drain_done");
        let close_ok = self.label("owned_list_close_ok");
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            cleanup.head_slot,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&done_label));
        // Advance the head past this node before the call, which clobbers
        // caller-saved registers; the loop reloads the head from memory.
        self.emit(abi::load_u64(abi::return_register(), &scratch9, 0));
        self.emit(abi::load_u64(&scratch10, &scratch9, 8));
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            cleanup.head_slot,
        ));
        self.emit_symbol_call(&cleanup.close_symbol);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&close_ok));
        self.record_secondary_cleanup_failure();
        self.emit(abi::label(&close_ok));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    /// Free an owned, non-escaping flat value at scope-drop (plan-01 Phase 5 /
    /// plan-02 Phase 8): recompute the block's byte size from its static type and
    /// `arena_free(ptr, size)`. `arena_free` scrubs the bytes (entropy poison),
    /// so a later use-after-free reads garbage and traps loudly. Clobbers
    /// caller-saved scratch; the caller reloads anything it needs afterward.
    pub(super) fn emit_owned_value_drop(
        &mut self,
        cleanup: &OwnedValueCleanup,
    ) -> Result<(), String> {
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
        let skip = self.label("owned_value_free_skip");
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            cleanup.stack_offset,
        ));
        self.emit(abi::compare_immediate(abi::return_register(), "0"));
        self.emit(abi::branch_eq(&skip));
        let size_slot = self.allocate_stack_object("owned_value_free_size", 8);
        // The slot already holds the block pointer; size it from the type.
        self.emit_inlined_block_size_from_ptr_slot(
            &cleanup.type_,
            cleanup.stack_offset,
            size_slot,
        )?;
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            cleanup.stack_offset,
        ));
        self.emit(abi::load_u64(abi::ARG[1], abi::stack_pointer(), size_slot));
        self.emit_arena_free_call();
        self.emit(abi::label(&skip));
        Ok(())
    }
}
