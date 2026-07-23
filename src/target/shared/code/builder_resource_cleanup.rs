use super::*;

impl CodeBuilder<'_> {
    /// Whether a resource kind uses the per-`File` output/read buffer words
    /// (`BUF_PTR` @24 … `READ_AT_EOF` @72) — i.e. whether it is a `File`.
    ///
    /// Every resource kind shares the 80-byte record, but only `File`'s open
    /// helpers zero those words after the PRNG-poisoned arena alloc; `net`'s
    /// `emit_make_handle` writes offsets 0/8/16 and leaves the rest poisoned. So
    /// the words are readable-as-pointers only for a `File`, and the drop-path
    /// reclaim must ask before it frees them (plan-52-B Phase 2).
    pub(super) fn resource_uses_io_buffers(type_: &str) -> bool {
        crate::builtins::resource::base_resource_name(type_) == "File"
    }

    pub(super) fn resource_cleanup_symbol(&self, type_: &str) -> Option<String> {
        let Some(close) = crate::builtins::resource_close_function(type_) else {
            // bug-374: not one of the language's own resources, so fall back to
            // the user-declared `RESOURCE T CLOSE BY op` table. The close op is
            // an ordinary `LINK` call target, so it resolves through
            // `function_symbols` exactly as an explicit `sql::close(db)` does —
            // NIR registers one import per link function (and a second for each
            // re-export alias), both keyed to the same thunk symbol.
            let close = self
                .type_model
                .resource_closers
                .get(crate::builtins::resource::base_resource_name(type_))?;
            return crate::target::shared::code::resolve_closer_symbol(
                close,
                self.function_symbols,
            );
        };
        let symbol = self
            .function_symbols
            .get(close)
            .cloned()
            .or_else(|| {
                runtime::helper_for_call(close)
                    .map(|helper| runtime::symbol_for_call(helper, close))
            })
            .unwrap_or_else(|| close.to_string());
        Some(symbol)
    }

    /// If `type_` is a resource union (every variant is a resource), the
    /// `(tag, close_symbol)` pairs for tag-dispatched drop; otherwise `None`.
    pub(super) fn resource_union_cleanup(&self, type_: &str) -> Option<Vec<(usize, String)>> {
        if !self.type_model.union_names.contains(type_) {
            return None;
        }
        let variants: Vec<String> = self.type_model.variants_for_union(type_).cloned().collect();
        if variants.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for variant in variants {
            if !crate::builtins::is_resource_type(&variant) {
                return None;
            }
            let tag = *self.type_model.union_variant_tags.get(&variant)?;
            let symbol = self.resource_cleanup_symbol(&variant)?;
            out.push((tag, symbol));
        }
        Some(out)
    }

    pub(super) fn deactivate_resource_cleanup(&mut self, name: &str) {
        if let Some(index) = self.active_cleanups.iter().rposition(|cleanup| {
            matches!(cleanup, ActiveCleanup::Resource(resource) if resource.name == name)
                || matches!(cleanup, ActiveCleanup::ResourceUnion(u) if u.name == name)
        }) {
            self.active_cleanups.remove(index);
        }
    }

    /// Tag-dispatched drop of a resource union: read the union tag and call the
    /// active variant's registered close op on its resource pointer (offset 8).
    pub(super) fn emit_resource_union_cleanup_call(
        &mut self,
        cleanup: &ResourceUnionCleanup,
    ) -> Result<(), String> {
        let stack_offset = match self.locals.get(&cleanup.name) {
            Some(local) => local.stack_offset,
            None => return Ok(()),
        };
        let union_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(
            &union_ptr,
            abi::stack_pointer(),
            stack_offset,
        ));
        let done = self.label("resource_union_drop_done");
        // Skip the whole dispatch when the slot is null: a `RES x = <fallible>`
        // resource-union binding whose initializer trapped before storing (or a
        // bind jumped past) leaves the slot at its entry-zeroed 0, and the tag
        // load at `union_ptr+0` would SIGSEGV on null (bug-246).
        self.emit(abi::compare_immediate(&union_ptr, "0"));
        self.emit(abi::branch_eq(&done));

        // plan-59-D Phase 3: the same identity skip as the plain-resource case.
        // A returned resource union escapes to the caller, so this scope must not
        // run its tag-dispatched drop. Backstops the static deactivation at
        // `emit_return_exit` (bug-141's arm) for a returned union that is not
        // syntactically the local owning the cleanup.
        if let Some(escaping) = self.escaping_value_slot {
            let escaping_ptr = self.allocate_register()?;
            self.emit(abi::load_u64(&escaping_ptr, abi::stack_pointer(), escaping));
            self.emit(abi::compare_registers(&union_ptr, &escaping_ptr));
            self.emit(abi::branch_eq(&done));
        }

        let union_slot = self.allocate_stack_object("resource_union_drop_ptr", 8);
        self.emit(abi::store_u64(&union_ptr, abi::stack_pointer(), union_slot));
        let tag_register = self.allocate_register()?;
        self.emit(abi::load_u64(&tag_register, &union_ptr, 0));
        let tag_slot = self.allocate_stack_object("resource_union_drop_tag", 8);
        self.emit(abi::store_u64(
            &tag_register,
            abi::stack_pointer(),
            tag_slot,
        ));
        let payload_slot = self.allocate_stack_object("resource_union_drop_payload", 8);
        for (tag, symbol) in cleanup.variants.clone() {
            let next = self.label("resource_union_drop_next");
            let tag_reg = self.allocate_register()?;
            self.emit(abi::load_u64(&tag_reg, abi::stack_pointer(), tag_slot));
            self.emit(abi::compare_immediate(&tag_reg, &tag.to_string()));
            self.emit(abi::branch_ne(&next));
            // Load the variant's resource pointer (payload at offset 8) and close it.
            let base = self.allocate_register()?;
            self.emit(abi::load_u64(&base, abi::stack_pointer(), union_slot));
            let payload = self.allocate_register()?;
            self.emit(abi::load_u64(&payload, &base, 8));
            self.emit(abi::store_u64(&payload, abi::stack_pointer(), payload_slot));
            let arg = NirValue::Local(format!("__resource_union_payload@{payload_slot}"));
            self.locals.insert(
                format!("__resource_union_payload@{payload_slot}"),
                LocalValue {
                    type_: "File".to_string(),
                    stack_offset: payload_slot,
                    constant: None,
                    by_ref: false,
                },
            );
            self.emit_raw_call(
                &symbol,
                std::slice::from_ref(&arg),
                "resource_union_drop_arg",
            )?;
            let after = self.label("resource_union_drop_check");
            self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&after));
            self.record_secondary_cleanup_failure();
            self.emit(abi::label(&after));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&next));
        }
        self.emit(abi::label(&done));
        Ok(())
    }

    pub(super) fn deactivate_moved_resource_arguments(&mut self, target: &str, args: &[NirValue]) {
        for (index, arg) in args.iter().enumerate() {
            let NirValue::Local(name) = arg else {
                continue;
            };
            let Some(local) = self.locals.get(name) else {
                continue;
            };
            let Some(close) = crate::builtins::resource_close_function(&local.type_) else {
                continue;
            };
            let consumed = if target == close {
                index == 0
            } else if matches!(
                target,
                "thread.start"
                    | "thread.send"
                    | "thread.emit"
                    | "thread.transferResource"
                    | "thread.emitResource"
            ) {
                // A thread-sendable resource is moved into the thread on a
                // successful transfer. Deactivation runs only on the success
                // path (after the result-tag branch), so the sender keeps
                // ownership and cleanup when the transfer fails with `Err`.
                index == 1 && crate::builtins::is_thread_sendable_resource_type(&local.type_)
            } else if crate::builtins::is_builtin_call(target) {
                false
            } else {
                // Ordinary user calls do not move the resource's ownership: the caller retains
                // ownership and its scope-drop cleanup. Only the fixed
                // invalidation events (registered close, thread transfer,
                // `RETURN`) hand off ownership.
                false
            };
            if consumed {
                self.deactivate_resource_cleanup(name);
            }
        }
    }

    pub(super) fn emit_resource_cleanup_call(
        &mut self,
        cleanup: &ResourceCleanup,
    ) -> Result<(), String> {
        let done = self.label("resource_cleanup_done");
        // Every path below that finishes the close converges here, where the
        // blocks the record points at are reclaimed (plan-52-B Phase 2). The
        // null-slot guard branches past it to `done` instead — there is no record
        // to read pointers out of.
        let reclaim = self.label("resource_cleanup_reclaim");
        // Skip the close entirely when the slot is null: a `RES x = <fallible>`
        // whose initializer trapped before storing a handle (or a bind the error
        // path jumped past) leaves the slot at its entry-zeroed 0, and the close
        // helper dereferences the closed-flag at `ptr+8` — a null read would
        // SIGSEGV (bug-246). Prologue zero-init guarantees such a slot reads 0
        // rather than stack garbage.
        let resource_slot = self
            .locals
            .get(&cleanup.name)
            .map(|local| local.stack_offset);
        if let Some(offset) = resource_slot {
            let ptr = self.allocate_register()?;
            self.emit(abi::load_u64(&ptr, abi::stack_pointer(), offset));
            self.emit(abi::compare_immediate(&ptr, "0"));
            self.emit(abi::branch_eq(&done));

            // plan-59-D: the identity skip. This resource's record pointer equals
            // the value escaping the scope, so it is being RETURNed — its close
            // obligation moves to the caller and this scope must neither close nor
            // reclaim it. Branch past both, to `done` rather than `reclaim`.
            //
            // Same control-flow shape as `emit_resource_block_reclaim`'s
            // moved-bit skip; only the predicate differs — a pointer compare
            // instead of a flag test.
            //
            // `escaping_value_slot` is `Some` only while emitting a `RETURN`'s
            // cleanups, so this is inert on every other exit path. That is what
            // keeps §15.6's rule intact: on an error exit *before* the return the
            // resource has not escaped and is still closed here.
            if let Some(escaping) = self.escaping_value_slot {
                let escaping_ptr = self.allocate_register()?;
                self.emit(abi::load_u64(&escaping_ptr, abi::stack_pointer(), escaping));
                self.emit(abi::compare_registers(&ptr, &escaping_ptr));
                self.emit(abi::branch_eq(&done));
            }
        }
        let arg = NirValue::Local(cleanup.name.clone());
        self.emit_raw_call(
            &cleanup.symbol,
            std::slice::from_ref(&arg),
            "resource_drop_arg",
        )?;
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&reclaim));
        // A close on an already-closed resource returns `ERR_RESOURCE_CLOSED`
        // (File/net's deliberate bug-63 re-close error). On the *drop* path this
        // is a benign no-op — the handle is already closed (e.g. the offset-8
        // closed-default record materialized for a `RES x = <fallible> TRAP`
        // error binding, or a program that already called `close`) — not a real
        // cleanup failure, so it must not be logged in the arena
        // cleanup-failure ledger (plan-38 F2). Every other non-OK close *is* a
        // genuine failure and still records. Compare via a register (the code is
        // an 8-digit value beyond the immediate range some backends allow).
        let closed_code = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &closed_code,
            "Integer",
            ERR_RESOURCE_CLOSED_CODE,
        ));
        self.emit(abi::compare_registers(RESULT_VALUE_REGISTER, &closed_code));
        self.emit(abi::branch_eq(&reclaim));
        self.record_secondary_cleanup_failure();
        self.emit(abi::label(&reclaim));
        if let Some(offset) = resource_slot {
            self.emit_resource_block_reclaim(
                offset,
                cleanup.state_type.as_deref(),
                cleanup.has_io_buffers,
            )?;
        }
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Reclaim the blocks a resource record points at — its output buffer, its
    /// read buffer, and its `STATE` payload — and null each pointer word as it
    /// goes (plan-52-B Phase 2). The 80-byte record itself is deliberately NOT
    /// freed: it is the tombstone holding the closed flag that makes a re-close
    /// idempotent and that every alias reads (res.md §3.1).
    ///
    /// **Runs at drop, never at close.** `fs::close(f)` must stay memory-neutral:
    /// the `.state` read path has no closed-guard, so `x.state` after an explicit
    /// close is legal today and would become a null dereference if close freed the
    /// payload. At a drop the binding is gone and nothing can name the resource.
    /// Close releases the OS handle; drop reclaims memory (plan-52-B §4).
    ///
    /// **Ordering is load-bearing**: the caller emits this only AFTER the close
    /// call, because the mandatory flush-on-close drains `BUF_PTR[0..BUF_FILLED]`
    /// to the fd. Freeing the buffer first would strand buffered data on the floor.
    ///
    /// Once-only comes free: these blocks are reachable ONLY through the record's
    /// pointer words, so nulling as we free makes a second drop a no-op — the same
    /// trick the closed flag plays for close. No aliasing analysis is needed, unlike
    /// `ActiveCleanup::OwnedValue`, whose `arena_free` is sound only because
    /// copy-insertion guarantees its block is unaliased.
    pub(super) fn emit_resource_block_reclaim(
        &mut self,
        resource_slot: usize,
        state_type: Option<&str>,
        has_io_buffers: bool,
    ) -> Result<(), String> {
        // A moved record's blocks belong to the receiver now: `thread::transfer`
        // copied the STATE pointer into the receiver's record, so freeing it here
        // would hand another thread a dangling payload. The transfer also
        // deactivates the sender's cleanup, so this path is not normally reached
        // for a moved resource — this guard makes that a property of the code
        // rather than of the caller (plan-52-B Open Decisions, the sharpest edge).
        let skip = self.label("resource_reclaim_skip");
        let ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), resource_slot));
        let flags = self.allocate_register()?;
        self.emit(abi::load_u64(&flags, &ptr, FILE_OFFSET_CLOSED));
        let moved_mask = self.allocate_register()?;
        self.emit(abi::move_immediate(
            &moved_mask,
            "Integer",
            &(1u64 << RESOURCE_MOVED_BIT).to_string(),
        ));
        self.emit(abi::and_registers(&moved_mask, &flags, &moved_mask));
        self.emit(abi::compare_immediate(&moved_mask, "0"));
        self.emit(abi::branch_ne(&skip));

        // The two per-`File` buffers are fixed-capacity blocks (plan-14-B/14-C).
        // Only a `File` may be asked for them: every resource kind shares the
        // 80-byte record, but only `File`'s open helpers zero these words after
        // the PRNG-poisoned arena alloc — a socket's record leaves 24..72 as
        // poison, so a null-guard is not enough to skip them and freeing them
        // handed `arena_free` a poison value (SIGSEGV in every `net::` program's
        // cleanup, caught by acceptance).
        if has_io_buffers {
            self.emit_free_resource_block(
                resource_slot,
                FILE_OFFSET_BUF_PTR,
                &FILE_BUFFER_CAPACITY.to_string(),
            )?;
            self.emit_free_resource_block(
                resource_slot,
                FILE_OFFSET_READ_PTR,
                &FILE_READ_BUFFER_CAPACITY.to_string(),
            )?;
        }
        if let Some(state_type) = state_type {
            self.emit_free_resource_state_block(resource_slot, state_type)?;
        }
        self.emit(abi::label(&skip));
        Ok(())
    }

    /// `arena_free` the fixed-size block at `offset` in the resource record, then
    /// null the pointer word. A null word is skipped: an unbuffered `File` never
    /// allocated an output buffer, a `File` that only wrote never allocated a read
    /// buffer, and no other resource kind uses either word.
    ///
    /// The record pointer is reloaded from `resource_slot` after the `arena_free`
    /// call rather than kept in a register: `_mfb_*` helpers clobber every
    /// caller-saved register, with no survivor set (`.ai/compiler.md`).
    pub(super) fn emit_free_resource_block(
        &mut self,
        resource_slot: usize,
        offset: usize,
        size: &str,
    ) -> Result<(), String> {
        let skip = self.label("resource_block_free_skip");
        let ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), resource_slot));
        let block = self.allocate_register()?;
        self.emit(abi::load_u64(&block, &ptr, offset));
        self.emit(abi::compare_immediate(&block, "0"));
        self.emit(abi::branch_eq(&skip));
        self.emit(abi::move_register(abi::return_register(), &block));
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", size));
        self.emit_arena_free_call();
        // Reload: the call above destroyed every caller-saved register.
        let ptr_after = self.allocate_register()?;
        self.emit(abi::load_u64(
            &ptr_after,
            abi::stack_pointer(),
            resource_slot,
        ));
        self.emit(abi::store_u64(abi::ZERO, &ptr_after, offset));
        self.emit(abi::label(&skip));
        Ok(())
    }

    /// `arena_free` the `STATE` payload and null its pointer word. Unlike the two
    /// buffers this block has no fixed size — a `STATE` record inlines its `String`
    /// fields — so it is sized from the type at `FILE_OFFSET_STATE`.
    pub(super) fn emit_free_resource_state_block(
        &mut self,
        resource_slot: usize,
        state_type: &str,
    ) -> Result<(), String> {
        let skip = self.label("resource_state_free_skip");
        let state_slot = self.allocate_stack_object("resource_state_free_ptr", 8);
        let size_slot = self.allocate_stack_object("resource_state_free_size", 8);
        let ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), resource_slot));
        let block = self.allocate_register()?;
        self.emit(abi::load_u64(&block, &ptr, FILE_OFFSET_STATE));
        self.emit(abi::store_u64(&block, abi::stack_pointer(), state_slot));
        self.emit(abi::compare_immediate(&block, "0"));
        self.emit(abi::branch_eq(&skip));
        self.emit_inlined_block_size_from_ptr_slot(state_type, state_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            state_slot,
        ));
        self.emit(abi::load_u64(abi::ARG[1], abi::stack_pointer(), size_slot));
        self.emit_arena_free_call();
        let ptr_after = self.allocate_register()?;
        self.emit(abi::load_u64(
            &ptr_after,
            abi::stack_pointer(),
            resource_slot,
        ));
        self.emit(abi::store_u64(abi::ZERO, &ptr_after, FILE_OFFSET_STATE));
        self.emit(abi::label(&skip));
        Ok(())
    }

    pub(super) fn record_secondary_cleanup_failure(&mut self) {
        let scratch9 = self.temporary_vreg();
        self.emit(abi::load_u64(
            &scratch9,
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
        ));
        self.emit(abi::add_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(
            &scratch9,
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
        ));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_CODE_OFFSET,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
        ));
    }

    pub(super) fn emit_thread_cleanup_call(
        &mut self,
        cleanup: &ThreadCleanup,
    ) -> Result<(), String> {
        let arg = NirValue::Local(cleanup.name.clone());
        self.emit_raw_call(
            &cleanup.symbol,
            std::slice::from_ref(&arg),
            "thread_drop_arg",
        )?;
        Ok(())
    }

    pub(super) fn emit_thread_cleanup_for_name(&mut self, name: &str) -> Result<(), String> {
        let cleanup = ThreadCleanup {
            name: name.to_string(),
            symbol: Self::thread_drop_symbol(),
        };
        self.emit_thread_cleanup_call(&cleanup)
    }

    /// The close op symbol for a resource collection's element/value type, or an
    /// error if `type_` is not a collection whose element is a single resource.
    pub(super) fn collection_resource_close_symbol(&self, type_: &str) -> Result<String, String> {
        let element = list_element_type(type_)
            .or_else(|| map_type_parts(type_).map(|(_, value)| value))
            .ok_or_else(|| format!("owned-list owner '{type_}' is not a collection"))?;
        self.resource_cleanup_symbol(&element).ok_or_else(|| {
            format!("owned-list element type '{element}' has no registered close op")
        })
    }
}
