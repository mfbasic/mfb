// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::target::shared::runtime;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    /// Whether a resource kind uses the per-`File` output/read buffer words
    /// (`BUF_PTR` @32 … `READ_AT_EOF` @80) — i.e. whether it is a `File`.
    ///
    /// Every resource kind shares the resource record, but only `File`'s open
    /// helpers zero those words after the PRNG-poisoned arena alloc; `net`'s
    /// `emit_make_handle` writes offsets 0/8/16/24 and leaves the rest poisoned. So
    /// the words are readable-as-pointers only for a `File`, and the drop-path
    /// reclaim must ask before it frees them (plan-52-B Phase 2).
    pub(crate) fn resource_uses_io_buffers(type_: &ParameterType) -> bool {
        type_.without_state().is_named("fs.File")
    }

    /// bug-623: the resource kinds whose resource record the owning binding's scope
    /// drop frees — the `tcp`, `udp` and `tls` handles. Before this, a
    /// `tcp::connect`/`tcp::close` loop leaked one record per connection forever.
    ///
    /// **The drop is the owner of the free, not `close`.** A `close` through a
    /// `RES` parameter leaves the owner holding the record, and a later operation
    /// through it — a second close included — must still read the closed flag and
    /// raise `ErrResourceClosed` (`tests/net/rt_double_close_is_refused.rs`). At the
    /// owner's drop nothing can name the record any more: every non-owning holder
    /// (a `RES` parameter, an aliasing bind, a `collections::get`/`tcp::poll`
    /// borrow, a record `RES` field) lives in the owner's scope or deeper, and a
    /// floated or returned handle has moved its cleanup elsewhere. For the same
    /// reason an explicit `close` on the owning binding keeps this cleanup
    /// registered (`deactivate_moved_resource_arguments`): its drop-time re-close
    /// is the defined, unreported `ErrResourceClosed` no-op, and the record is
    /// freed after it.
    ///
    /// Every producer of these kinds hands the binding a record in the current
    /// thread's arena — the open helpers' `emit_make_handle`/TLS record allocs, the
    /// closed default of `emit_closed_resource_record`, and the thread-transfer
    /// copy — so the free goes to the arena that allocated it. The slot is zeroed
    /// after the free so a drop reached again on the same slot (a loop re-entry, a
    /// `TRAP` handler after a scope exit) skips on the existing null guard.
    ///
    /// Other kinds keep the tombstone: their producers include records this
    /// analysis has not audited (native `LINK` thunks, `process`).
    ///
    /// plan-158-A: `audio` streams are audited and included. Every producer is a
    /// 96 B record in this arena (the three backends' opens and the closed default);
    /// the streams are not sendable; every I/O helper reads `H_CLOSED` before
    /// `H_STATE`; and the close tears down everything the record points at before
    /// the drop frees it — macOS disposes its `AudioQueue` with `inImmediate = 1`,
    /// so its callbacks, which receive the record, cannot run after it.
    ///
    /// bug-647: `fs.File` used to be excluded too, "shares the record with the
    /// drop's buffer reclaim". That reason was stale — and it cost two whole
    /// records per bind. [`Self::emit_resource_block_reclaim`] frees in dependency
    /// order already: the two `File` buffers (reached THROUGH the record's
    /// `FILE_OFFSET_BUF_PTR` / `FILE_OFFSET_READ_PTR` words), then the STATE block,
    /// and only then the record itself. Nothing reads the record after it is freed,
    /// so sharing it with the buffer reclaim is exactly what the ordering handles.
    /// Including it here frees the producer's record AND, through the
    /// `frees_record` flag this predicate feeds, the closed default record an
    /// inline `TRAP`'s `$trap_valN` binding materializes.
    ///
    /// bug-623 B: the kind is necessary, not sufficient. A binding frees the record
    /// only when it also OWNS it (`record_owning_locals`, computed by
    /// `resource::cleanup::record_ownership`): a union wrapping a live binding, or a
    /// call that hands back a parameter's record, holds a record another binding frees.
    ///
    /// plan-156-C: a parent `Thread` view too. `thread::start` allocates it in the
    /// caller's arena, and the drop's close (`thread.drop`) is done with it — the
    /// control block it names lives on its own reference — so a thread started per loop
    /// iteration does not leak a record per iteration.
    pub(crate) fn resource_record_freed_at_drop(type_: &ParameterType) -> bool {
        if matches!(
            type_.without_state(),
            ParameterType::ThreadHandle { worker: false, .. }
        ) {
            return true;
        }
        let base = type_.without_state();
        [
            "tcp.Socket",
            "tcp.Listener",
            "udp.Socket",
            "tls.Socket",
            "tls.Listener",
            "fs.File",
            "audio.AudioInput",
            "audio.AudioOutput",
        ]
        .iter()
        .any(|name| base.is_named(name))
    }

    pub(crate) fn resource_cleanup_symbol(&self, type_: &ParameterType) -> Option<String> {
        let Some(close) = crate::codegen::builtins::resource_close_function(&type_) else {
            // bug-374: not one of the language's own resources, so fall back to
            // the user-declared `RESOURCE T CLOSE BY op` table. The close op is
            // an ordinary `LINK` call target, so it resolves through
            // `function_symbols` exactly as an explicit `sql::close(db)` does —
            // NIR registers one import per link function (and a second for each
            // re-export alias), both keyed to the same thunk symbol.
            let close = self
                .type_model
                .resource_closers
                .get(&ParameterType::declared(&type_.without_state().name()))?;
            return crate::codegen::engine::builder::resolve_closer_symbol(
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
    ///
    /// The STATE suffix is stripped first (plan-74): a stateful resource union
    /// (`Stream STATE PendingState`) names the same union as its bare form, so it
    /// must register the same tag-dispatched close — otherwise a stateful union
    /// binding would register no cleanup at all and leak its handle.
    /// bug-623 B: the union variant tags whose record a drop may free — the
    /// `resource_record_freed_at_drop` variants. Empty for a non-union.
    pub(crate) fn resource_union_record_free_tags(&self, type_: &ParameterType) -> Vec<usize> {
        let base = type_.without_state();
        if !self.type_model.union_names.contains(&base) {
            return Vec::new();
        }
        self.type_model
            .variants_for_union(&base)
            .filter(|variant| Self::resource_record_freed_at_drop(variant))
            .filter_map(|variant| self.type_model.union_variant_tags.get(variant).copied())
            .collect()
    }

    /// bug-623 B: whether a binding of `type_` can ever have a record freed at drop —
    /// the domain `record_ownership` resolves.
    pub(crate) fn record_freeable_type(&self, type_: &ParameterType) -> bool {
        Self::resource_record_freed_at_drop(type_)
            || !self.resource_union_record_free_tags(type_).is_empty()
    }

    /// bug-623 B: whether a returned resource-union value's `{tag, record}` box already
    /// belongs to this function alone, so `RETURN` may hand it to the caller uncopied.
    ///
    /// The copy `lower_returned_value` otherwise makes orphaned the original box: 16 B
    /// per `RETURN s` of an owned union local (whose cleanup the return deactivates, so
    /// nothing frees the original) and per `RETURN f()` (a temp nobody frees) — the
    /// 16 B of `http::read`'s 112 B per call. It stays for a parameter or any other
    /// alias, where the caller's own argument still owns the box.
    ///
    /// Owned here: a union wrap (a box allocated for this value), a direct call to a
    /// user function returning a resource union (whose box that function either moved
    /// out or copied, so it is exclusively this frame's), and a local — or a bare-local
    /// alias chain — rooted at a binding with a live union cleanup (its box moves; the
    /// `RETURN` deactivation or the identity skip retires that cleanup).
    pub(crate) fn returned_resource_union_owns_box(&self, value: &NirValue) -> bool {
        match value {
            NirValue::UnionWrap { union_type, .. } => {
                self.resource_union_cleanup(union_type).is_some()
            }
            NirValue::Call { target, .. } => {
                !self.locals.contains_key(target)
                    && self
                        .functions
                        .get(target.as_str())
                        .is_some_and(|f| self.resource_union_cleanup(&f.returns).is_some())
            }
            NirValue::Local(name) => {
                let mut root = name.as_str();
                let mut hops = 0;
                while let Some(src) = self.resource_alias_sources.get(root) {
                    root = src.as_str();
                    hops += 1;
                    if hops > 64 {
                        return false;
                    }
                }
                let Some(local) = self.locals.get(root) else {
                    return false;
                };
                self.resource_union_cleanup(&local.type_).is_some()
                    && self.active_cleanups.iter().any(|cleanup| {
                        matches!(cleanup, ActiveCleanup::ResourceUnion(u) if u.name == root)
                    })
            }
            _ => false,
        }
    }

    /// Resource-union alias class: the concrete binding `name` aliases, following the
    /// bind-time bare-local alias map (`RES v = u` hops) to its root.
    pub(crate) fn resource_alias_root(&self, name: &str) -> String {
        let mut current = name;
        for _ in 0..64 {
            match self.live_resource_aliases.get(current) {
                Some(next) => current = next.as_str(),
                None => break,
            }
        }
        current.to_string()
    }

    /// Resource-union alias class: when `RETURN value` returns a union that aliases a
    /// live concrete binding (`RES c AS Union = u` then `RETURN c`, possibly through
    /// bare-local aliases on either side), that binding's name — its close obligation
    /// moves to the caller with the returned union, so this frame must not close it.
    /// `None` for any other return, including a union around a parameter (no live
    /// close to move).
    pub(crate) fn returned_union_alias_root(&self, value: &NirValue) -> Option<String> {
        let NirValue::Local(name) = value else {
            return None;
        };
        let mut current = name.as_str();
        let mut src = None;
        for _ in 0..64 {
            if let Some(wrapped) = self.live_union_wraps.get(current) {
                src = Some(wrapped.as_str());
                break;
            }
            current = self.live_resource_aliases.get(current)?.as_str();
        }
        let root = self.resource_alias_root(src?);
        self.active_cleanups
            .iter()
            .any(|c| matches!(c, ActiveCleanup::Resource(r) if r.name == root))
            .then_some(root)
    }

    pub(crate) fn resource_union_cleanup(
        &self,
        type_: &ParameterType,
    ) -> Option<Vec<(usize, String)>> {
        // The STATE clause is stripped structurally; the union set is keyed on
        // the bare base (plan-75 gap 3).
        let type_ = type_.without_state();
        if !self.type_model.union_names.contains(&type_) {
            return None;
        }
        let variants: Vec<ParameterType> = self
            .type_model
            .variants_for_union(&type_)
            .cloned()
            .collect();
        if variants.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for variant in variants {
            if !crate::codegen::builtins::is_resource_type(&variant) {
                return None;
            }
            let tag = *self.type_model.union_variant_tags.get(&variant)?;
            let symbol = self.resource_cleanup_symbol(&variant)?;
            out.push((tag, symbol));
        }
        Some(out)
    }

    /// True when `type_`'s base names a resource union (plan-74). A resource union
    /// *value* is a `{ tag @0, resource-record-ptr @8 }` block — **not** the
    /// resource record itself — so its `STATE` payload lives in the active
    /// variant's record reached via a `+8` indirection, whereas a concrete
    /// resource value already *is* that record. The STATE suffix is stripped.
    pub(crate) fn is_resource_union_type(&self, type_: &ParameterType) -> bool {
        self.resource_union_cleanup(type_).is_some()
    }

    /// plan-116-J: a call to a **consuming parameter** takes over closing every resource
    /// reachable from its argument, so this scope must stop closing them.
    ///
    /// The analysis twin of this is `ir::verify`'s containment relation, which marks the
    /// caller's bindings moved so a *later* use is `2-203-0055`. That is a diagnostic and
    /// nothing more: `moved` is a verification set, and codegen emits its scope-drop
    /// closes from the cleanup list regardless. Without this deactivation the caller
    /// still closes the image at scope exit, the group goes on naming a closed resource,
    /// and the item draws nothing — silently, because a second close is a defined no-op
    /// so nothing crashes and no error is raised.
    ///
    /// Modelled on `RETURN`'s deactivation in `builder_exits.rs`, which is the same
    /// problem: a value whose ownership leaves this scope must not also be closed by it.
    /// Two shapes, because a resource's close obligation can sit in either place:
    ///
    /// * the resource itself is named in the argument → drop its `ActiveCleanup`;
    /// * a **container** is named and the resource floats to it (`ResOwner::Float`) →
    ///   drop the container's owned-list drain. Dropping the whole list is correct
    ///   rather than over-broad: everything the container holds is reachable from the
    ///   argument, so the callee takes over all of it.
    pub(crate) fn deactivate_consumed_cleanups(&mut self, target: &str, args: &[NirValue]) {
        let Some(index) = crate::codegen::resource::builtin_consuming_parameter_index(target)
        else {
            return;
        };
        let Some(argument) = args.get(index) else {
            return;
        };
        let mut named = Vec::new();
        Self::collect_consumed_locals(argument, &mut named);
        // Expand each named local through what it HOLDS. The argument is normally
        // `[tag]` — a list naming an item, not the resource — and the close obligation
        // that has to be dropped belongs to `face`, whose name appears nowhere in the
        // argument. Without this expansion the walk finds only `tag`, whose type is a
        // union and therefore not a resource-owning container, and deactivates nothing:
        // the scope closes the font at exit and the group's text draws no glyphs.
        // Measured exactly that way before the expansion existed.
        let mut frontier = named.clone();
        let mut seen: std::collections::HashSet<String> = named.iter().cloned().collect();
        while let Some(current) = frontier.pop() {
            let Some(held) = self.resource_containment.get(&current) else {
                continue;
            };
            for name in held.clone() {
                if seen.insert(name.clone()) {
                    named.push(name.clone());
                    frontier.push(name);
                }
            }
        }
        for name in named {
            let Some(local) = self.locals.get(&name) else {
                continue;
            };
            let type_ = local.type_.clone();
            if crate::codegen::builtins::resource_close_function(&type_).is_some()
                || self.resource_union_cleanup(&type_).is_some()
            {
                self.deactivate_resource_cleanup(&name);
            }
            if self.is_resource_owning_container(&type_) {
                self.deactivate_owned_list(&name);
            }
        }
    }

    /// Every local named structurally inside a consumed argument.
    ///
    /// Structural only, and for the same reason the verifier's containment walk is
    /// (`ir::verify::link`): a constructor or literal *places* its operands into the
    /// value it builds, and an opaque call does not. Deactivating a cleanup for a local
    /// that merely fed a call would leave a resource this scope still owns unclosed — a
    /// leak, where the opposite error is only a redundant close of something the callee
    /// already closed, which is a defined no-op.
    pub(crate) fn collect_consumed_locals(value: &NirValue, out: &mut Vec<String>) {
        match value {
            NirValue::Local(name) => out.push(name.clone()),
            NirValue::Constructor { args, .. } => {
                for arg in args {
                    Self::collect_consumed_locals(arg, out);
                }
            }
            NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
                for element in values {
                    Self::collect_consumed_locals(element, out);
                }
            }
            NirValue::MapLiteral { entries, .. } => {
                for (key, entry) in entries {
                    Self::collect_consumed_locals(key, out);
                    Self::collect_consumed_locals(entry, out);
                }
            }
            NirValue::UnionWrap { value, .. } | NirValue::Checked { value, .. } => {
                Self::collect_consumed_locals(value, out);
            }
            NirValue::WithUpdate {
                target, updates, ..
            } => {
                Self::collect_consumed_locals(target, out);
                for update in updates {
                    Self::collect_consumed_locals(&update.value, out);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn deactivate_resource_cleanup(&mut self, name: &str) {
        if let Some(index) = self.active_cleanups.iter().rposition(|cleanup| {
            matches!(cleanup, ActiveCleanup::Resource(resource) if resource.name == name)
                || matches!(cleanup, ActiveCleanup::ResourceUnion(u) if u.name == name)
        }) {
            self.active_cleanups.remove(index);
        }
    }

    /// bug-623 D: the sender's side of a successful thread transfer. The handle is
    /// the receiver's now, so the binding's drop must close nothing. But the 96-byte
    /// record it still points at is the sender's `moved|closed` tombstone, carved
    /// by the sender's arena, and nothing else will ever free it. So a cleanup that
    /// frees its record is kept and switched to `moved_record_only`, and its drop
    /// frees just that record. Every other cleanup is removed exactly as before.
    ///
    /// The drop runs at the binding's scope exit, after the last point the sender
    /// can read the tombstone's flags.
    fn retire_moved_resource_cleanup(&mut self, name: &str) {
        let retired = self
            .active_cleanups
            .iter_mut()
            .rev()
            .find_map(|cleanup| match cleanup {
                ActiveCleanup::Resource(resource) if resource.name == name => Some(resource),
                _ => None,
            });
        match retired {
            Some(resource) if resource.frees_record => resource.moved_record_only = true,
            _ => self.deactivate_resource_cleanup(name),
        }
    }

    /// The drop of a [`retire_moved_resource_cleanup`]d binding: free the sender's
    /// tombstone record when, and only when, it carries the moved bit, then null the
    /// slot so a re-reached drop skips. A null slot, a record being `RETURN`ed
    /// (plan-59-D's identity skip), or a record whose transfer did not complete
    /// frees nothing.
    ///
    /// bug-650: the tombstone's STATE block goes with it. `thread::transfer` does not
    /// hand the receiver this block — `copy_resource_to_current_arena` deep-copies the
    /// STATE into the RECEIVER's arena (bug-257, "never an alias into the sender's
    /// arena"), so once the move completes the sender's own STATE block has no owner at
    /// all. It was the last 16 B per transfer still leaking after the queued copy and
    /// its STATE were reclaimed. The same reasoning that licenses freeing the record
    /// here licenses freeing what it points at, and it is freed BEFORE the record so
    /// the pointer is still readable.
    ///
    /// [`retire_moved_resource_cleanup`]: Self::retire_moved_resource_cleanup
    fn emit_moved_resource_record_free(&mut self, cleanup: &ResourceCleanup) -> Result<(), String> {
        let Some(offset) = self
            .locals
            .get(&cleanup.name)
            .map(|local| local.stack_offset)
        else {
            return Ok(());
        };
        let done = self.label("resource_moved_record_done");
        let ptr = self.allocate_register();
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), offset));
        self.emit(abi::compare_immediate(&ptr, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit_owner_flag_skip(cleanup.owner_flag_slot, &done);
        // bug-755: only the owner's scope frees the tombstone; a reference that
        // transferred a resource owned further out leaves it to that owner.
        self.emit_skip_unless_owned(&abi::Reg::from(ptr.clone()), cleanup.scope_depth, &done)?;
        if let Some(escaping) = self.escaping_value_slot {
            let escaping_ptr = self.allocate_register();
            self.emit(abi::load_u64(&escaping_ptr, abi::stack_pointer(), escaping));
            self.emit(abi::compare_registers(&ptr, &escaping_ptr));
            self.emit(abi::branch_eq(&done));
        }
        let flags = self.allocate_register();
        self.emit(abi::load_u64(&flags, &ptr, FILE_OFFSET_CLOSED));
        let moved_mask = self.allocate_register();
        self.emit(abi::move_immediate(
            &moved_mask,
            "Integer",
            &(1u64 << RESOURCE_MOVED_BIT).to_string(),
        ));
        self.emit(abi::and_registers(&moved_mask, &flags, &moved_mask));
        self.emit(abi::compare_immediate(&moved_mask, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit_free_moved_tombstone(offset, &cleanup.name)?;
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Free the moved tombstone record in the slot at `offset` (binding `name`) and
    /// its STATE block, then null the slot. The caller has established that the
    /// record carries the moved bit and that this scope owns it.
    fn emit_free_moved_tombstone(&mut self, offset: usize, name: &str) -> Result<(), String> {
        // bug-650: the STATE block first, while the record still points at it.
        if let Some(state_type) = self.locals.get(name).and_then(|local| local.type_.state()) {
            self.emit_free_resource_state_block(offset, &state_type)?;
        }
        let record = self.allocate_register();
        self.emit(abi::load_u64(&record, abi::stack_pointer(), offset));
        self.emit(abi::move_register(abi::c_arg(0), &record));
        self.emit(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            RESOURCE_RECORD_SIZE,
        ));
        self.emit_arena_free_call();
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), offset));
        Ok(())
    }

    /// bug-648: branch to `skip` when a binding's run-time ownership flag is clear —
    /// the value it holds was lent to it (a borrowed element, an aliasing `RECOVER`),
    /// so its drop must neither close nor reclaim anything. Emits nothing for a
    /// binding that owns unconditionally.
    fn emit_owner_flag_skip(&mut self, owner_flag_slot: Option<usize>, skip: &str) {
        let Some(flag) = owner_flag_slot else {
            return;
        };
        let owned = self.allocate_register();
        self.emit(abi::load_u64(&owned, abi::stack_pointer(), flag));
        self.emit(abi::compare_immediate(&owned, "0"));
        self.emit(abi::branch_eq(skip));
    }

    /// Tag-dispatched drop of a resource union: read the union tag and call the
    /// active variant's registered close op on its resource pointer (offset 8).
    pub(crate) fn emit_resource_union_cleanup_call(
        &mut self,
        cleanup: &ResourceUnionCleanup,
    ) -> Result<(), String> {
        let stack_offset = match self.locals.get(&cleanup.name) {
            Some(local) => local.stack_offset,
            None => return Ok(()),
        };
        let union_ptr = self.allocate_register();
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
        self.emit_owner_flag_skip(cleanup.owner_flag_slot, &done);

        // plan-59-D Phase 3: the same identity skip as the plain-resource case.
        // A returned resource union escapes to the caller, so this scope must not
        // run its tag-dispatched drop. Backstops the static deactivation at
        // `emit_return_exit` (bug-141's arm) for a returned union that is not
        // syntactically the local owning the cleanup.
        if let Some(escaping) = self.escaping_value_slot {
            let escaping_ptr = self.allocate_register();
            self.emit(abi::load_u64(&escaping_ptr, abi::stack_pointer(), escaping));
            self.emit(abi::compare_registers(&union_ptr, &escaping_ptr));
            self.emit(abi::branch_eq(&done));
        }

        // bug-623: every dispatched drop converges on `dropped`, which frees the
        // union's own `{tag@0, record-ptr@8}` box. The box is this binding's alone:
        // a resource-union `UnionWrap` and the closed default both allocate a fresh
        // 16-byte block for the bind, and every other holder of it (an aliasing
        // bind, a record `RES` field, a floated owned-list node, a `RETURN`) either
        // registers no cleanup or skips to `done` above. The variant RECORD is not
        // freed here: `RES c AS Union = u` wraps a record the concrete binding `u`
        // still owns and frees. The slot is zeroed so a re-reached drop skips on
        // the null guard. The null and escaping-value skips branch past it.
        let dropped = self.label("resource_union_drop_box");
        // Resource-union alias class: an alias union (`closes_variant == false`) does not
        // own the record, so it neither closes it nor frees its STATE — only its box.
        if cleanup.closes_variant {
            // bug-755: the variant record closes only when this binding's scope owns
            // it; a union referring to a record owned further out frees just its box.
            let record = self.allocate_register();
            self.emit(abi::load_u64(&record, &union_ptr, 8));
            self.emit(abi::compare_immediate(&record, "0"));
            self.emit(abi::branch_eq(&dropped));
            self.emit_skip_unless_owned(&abi::Reg::from(record), cleanup.scope_depth, &dropped)?;
            self.emit_union_tag_dispatch_drop(
                &union_ptr,
                &cleanup.variants,
                cleanup.state_type.as_ref(),
                &cleanup.record_free_tags,
                &dropped,
            )?;
        }
        self.emit(abi::label(&dropped));
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), stack_offset));
        self.emit(abi::move_register(abi::c_arg(0), &block));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "16"));
        self.emit_arena_free_call();
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            stack_offset,
        ));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Emit the tag-dispatch-close + STATE-free core of a resource-union drop,
    /// given a register `union_ptr` holding the union block pointer
    /// (`{tag @0, record-ptr @8}`) that the caller has already null/identity
    /// checked. Reads the tag, calls the active variant's registered close op on
    /// its record pointer, then frees that record's uniform STATE block
    /// (plan-74); on a matched-and-closed variant control branches to
    /// `done_label`.
    ///
    /// Shared by the single-binding drop (`emit_resource_union_cleanup_call`) and
    /// the owned-list drain (`emit_owned_list_drain`, bug-429) so a
    /// `List OF RES <Union> STATE <S>` closes each floated element exactly as a
    /// lone `RES u AS Union` binding does — the two call sites cannot drift.
    pub(crate) fn emit_union_tag_dispatch_drop(
        &mut self,
        union_ptr: &VirtualRegister,
        variants: &[(usize, String)],
        state_type: Option<&ParameterType>,
        record_free_tags: &[usize],
        done_label: &str,
    ) -> Result<(), String> {
        let union_slot = self.allocate_stack_object("resource_union_drop_ptr", 8);
        self.emit(abi::store_u64(union_ptr, abi::stack_pointer(), union_slot));
        let tag_register = self.allocate_register();
        self.emit(abi::load_u64(&tag_register, union_ptr, 0));
        let tag_slot = self.allocate_stack_object("resource_union_drop_tag", 8);
        self.emit(abi::store_u64(
            &tag_register,
            abi::stack_pointer(),
            tag_slot,
        ));
        let payload_slot = self.allocate_stack_object("resource_union_drop_payload", 8);
        for (tag, symbol) in variants {
            let next = self.label("resource_union_drop_next");
            let tag_reg = self.allocate_register();
            self.emit(abi::load_u64(&tag_reg, abi::stack_pointer(), tag_slot));
            self.emit(abi::compare_immediate(&tag_reg, &tag.to_string()));
            self.emit(abi::branch_ne(&next));
            // Load the variant's resource pointer (payload at offset 8) and close it.
            let base = self.allocate_register();
            self.emit(abi::load_u64(&base, abi::stack_pointer(), union_slot));
            let payload = self.allocate_register();
            self.emit(abi::load_u64(&payload, &base, 8));
            self.emit(abi::store_u64(&payload, abi::stack_pointer(), payload_slot));
            let arg = NirValue::Local(format!("__resource_union_payload@{payload_slot}"));
            self.locals.insert(
                format!("__resource_union_payload@{payload_slot}"),
                LocalValue {
                    type_: ParameterType::named("fs.File"),
                    stack_offset: payload_slot,
                    constant: None,
                    by_ref: false,
                },
            );
            self.emit_raw_call(
                symbol,
                std::slice::from_ref(&arg),
                "resource_union_drop_arg",
            )?;
            let after = self.label("resource_union_drop_check");
            self.emit_drop_close_outcome(&after);
            self.emit(abi::label(&after));
            // plan-74: after the tag-dispatched close, free the active variant
            // record's uniform STATE block. `payload_slot` holds the variant
            // record pointer (loaded at `+8` above), which is exactly the record
            // pointer `emit_free_resource_state_block` expects; it null-checks the
            // STATE pointer and zeroes it, so a re-drop is a no-op (no double-free).
            // Ordered after the close, mirroring the concrete resource path.
            if let Some(state_type) = state_type {
                self.emit_free_resource_state_block(payload_slot, state_type)?;
            }
            if record_free_tags.contains(tag) {
                // bug-623 B: the union owns this variant's record (`record_free_tags`
                // is empty otherwise), so after the close and the STATE free the
                // record itself goes back to the arena — the same last step the
                // concrete drop takes. A moved record belongs to the receiver.
                let skip = self.label("resource_union_record_free_skip");
                let record = self.allocate_register();
                self.emit(abi::load_u64(&record, abi::stack_pointer(), payload_slot));
                let flags = self.allocate_register();
                self.emit(abi::load_u64(&flags, &record, FILE_OFFSET_CLOSED));
                let moved_mask = self.allocate_register();
                self.emit(abi::move_immediate(
                    &moved_mask,
                    "Integer",
                    &(1u64 << RESOURCE_MOVED_BIT).to_string(),
                ));
                self.emit(abi::and_registers(&moved_mask, &flags, &moved_mask));
                self.emit(abi::compare_immediate(&moved_mask, "0"));
                self.emit(abi::branch_ne(&skip));
                let record_arg = self.allocate_register();
                self.emit(abi::load_u64(
                    &record_arg,
                    abi::stack_pointer(),
                    payload_slot,
                ));
                self.emit(abi::move_register(abi::c_arg(0), &record_arg));
                self.emit(abi::move_immediate(
                    abi::c_arg(1),
                    "Integer",
                    RESOURCE_RECORD_SIZE,
                ));
                self.emit_arena_free_call();
                self.emit(abi::label(&skip));
            }
            self.emit(abi::branch(done_label));
            self.emit(abi::label(&next));
        }
        Ok(())
    }

    /// bug-854: what follows every drop-path close call. An `Ok` close, and the two
    /// closes that find nothing to do — `ErrResourceClosed` (the program already
    /// closed it) and `ErrResourceMoved` (`thread::transfer` moved it to another
    /// thread, possibly through a callee the static move check cannot see) — branch
    /// to `ok`; any other failure is recorded in the cleanup-failure ledger, then
    /// falls through. A drop is never where a close of a closed or moved handle is
    /// reported (§15, transfer outcome: "Sender cleanup must not close it").
    pub(crate) fn emit_drop_close_outcome(&mut self, ok: &str) {
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(ok));
        // Compared through a register: the codes are 8-digit values beyond the
        // immediate range some backends allow.
        for benign in ["ErrResourceClosed", "ErrResourceMoved"] {
            let code = self.temporary_vreg();
            self.emit(abi::move_immediate(
                &code,
                "Integer",
                crate::codegen::registry::runtime_error(benign)
                    .expect("errorCode name")
                    .0,
            ));
            self.emit(abi::compare_registers(RESULT_VALUE_REGISTER, &code));
            self.emit(abi::branch_eq(ok));
        }
        self.record_secondary_cleanup_failure();
    }

    pub(crate) fn deactivate_moved_resource_arguments(&mut self, target: &str, args: &[NirValue]) {
        for (index, arg) in args.iter().enumerate() {
            let NirValue::Local(name) = arg else {
                continue;
            };
            let Some(local) = self.locals.get(name) else {
                continue;
            };
            let Some(close) = crate::codegen::builtins::resource_close_function(&local.type_)
            else {
                continue;
            };
            let mut moved_to_thread = false;
            let consumed = if target == close {
                // bug-623: a kind whose record the drop frees keeps its cleanup
                // across an explicit close — the drop's re-close is a no-op and the
                // drop is the record's only free (`resource_record_freed_at_drop`).
                index == 0 && !Self::resource_record_freed_at_drop(&local.type_)
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
                moved_to_thread = index == 1
                    && crate::codegen::builtins::is_thread_sendable_resource_type(&local.type_);
                moved_to_thread
            } else if crate::codegen::builtins::is_builtin_call(target) {
                false
            } else {
                // Ordinary user calls do not move the resource's ownership: the caller retains
                // ownership and its scope-drop cleanup. Only the fixed
                // invalidation events (registered close, thread transfer,
                // `RETURN`) hand off ownership.
                false
            };
            if consumed && moved_to_thread {
                // The cleanup to retire is the OWNER's. A non-owning alias (bug-375:
                // `RES f = …` over a live resource — which is exactly how an inline
                // `TRAP` bind lands, `f` aliasing the desugar's `$trap_val`) has no
                // cleanup of its own, so retiring by the alias's name found nothing and
                // the owner's full drop later skipped the moved record: 96 B leaked per
                // trapped-bind transfer.
                let owner = self
                    .live_resource_aliases
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                self.retire_moved_resource_cleanup(&owner);
            } else if consumed {
                self.deactivate_resource_cleanup(name);
            }
        }
    }

    pub(crate) fn emit_resource_cleanup_call(
        &mut self,
        cleanup: &ResourceCleanup,
    ) -> Result<(), String> {
        if cleanup.moved_record_only {
            return self.emit_moved_resource_record_free(cleanup);
        }
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
            let ptr = self.allocate_register();
            self.emit(abi::load_u64(&ptr, abi::stack_pointer(), offset));
            self.emit(abi::compare_immediate(&ptr, "0"));
            self.emit(abi::branch_eq(&done));
            self.emit_owner_flag_skip(cleanup.owner_flag_slot, &done);
            // bug-755: the resource holds its cleanup scope. This binding closes the
            // record only when its scope is the record's owner; a binding that refers
            // to a resource owned further out — a handle a callee was passed and
            // handed back — closes and reclaims nothing.
            self.emit_skip_unless_owned(&abi::Reg::from(ptr.clone()), cleanup.scope_depth, &done)?;

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
                let escaping_ptr = self.allocate_register();
                self.emit(abi::load_u64(&escaping_ptr, abi::stack_pointer(), escaping));
                self.emit(abi::compare_registers(&ptr, &escaping_ptr));
                self.emit(abi::branch_eq(&done));
            }
            // bug-854: a record `thread::transfer` moved to another thread — through
            // a callee, or under an inline `TRAP` whose success the static retirement
            // cannot see (bug-899) — is the receiver's: no close and no reclaim.
            // What is left here is the sender's tombstone, freed exactly as the
            // retired drop of a statically seen transfer frees it.
            let live = self.label("resource_cleanup_not_moved");
            let flags = self.allocate_register();
            let record = self.allocate_register();
            self.emit(abi::load_u64(&record, abi::stack_pointer(), offset));
            self.emit(abi::load_u64(&flags, &record, FILE_OFFSET_CLOSED));
            self.emit(abi::compare_immediate(&flags, RESOURCE_MOVED_CLOSED_VALUE));
            self.emit(abi::branch_ne(&live));
            if cleanup.frees_record {
                self.emit_free_moved_tombstone(offset, &cleanup.name)?;
            }
            self.emit(abi::branch(&done));
            self.emit(abi::label(&live));
        }
        let arg = NirValue::Local(cleanup.name.clone());
        self.emit_raw_call(
            &cleanup.symbol,
            std::slice::from_ref(&arg),
            "resource_drop_arg",
        )?;
        // A close on an already-closed resource returns `ERR_RESOURCE_CLOSED`
        // (File/net's deliberate bug-63 re-close error). On the *drop* path this
        // is a benign no-op — the handle is already closed (e.g. the offset-16
        // closed-default record materialized for a `RES x = <fallible> TRAP`
        // error binding, or a program that already called `close`) — not a real
        // cleanup failure, so it must not be logged in the arena
        // cleanup-failure ledger (plan-38 F2). Every other non-OK close *is* a
        // genuine failure and still records. Compare via a register (the code is
        // an 8-digit value beyond the immediate range some backends allow).
        // bug-854: so is a moved one (`emit_drop_close_outcome`).
        self.emit_drop_close_outcome(&reclaim);
        self.emit(abi::label(&reclaim));
        if let Some(offset) = resource_slot {
            self.emit_resource_block_reclaim(
                offset,
                cleanup.state_type.as_ref(),
                cleanup.has_io_buffers,
                cleanup.frees_record,
            )?;
        }
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Reclaim the blocks a resource record points at — its output buffer, its
    /// read buffer, and its `STATE` payload — and null each pointer word as it
    /// goes (plan-52-B Phase 2). The resource record itself is deliberately NOT
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
    pub(crate) fn emit_resource_block_reclaim(
        &mut self,
        resource_slot: usize,
        state_type: Option<&ParameterType>,
        has_io_buffers: bool,
        free_record: bool,
    ) -> Result<(), String> {
        // A moved record's blocks belong to the receiver now: `thread::transfer`
        // copied the STATE pointer into the receiver's record, so freeing it here
        // would hand another thread a dangling payload. The transfer also
        // deactivates the sender's cleanup, so this path is not normally reached
        // for a moved resource — this guard makes that a property of the code
        // rather than of the caller (plan-52-B Open Decisions, the sharpest edge).
        let skip = self.label("resource_reclaim_skip");
        let ptr = self.allocate_register();
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), resource_slot));
        let flags = self.allocate_register();
        self.emit(abi::load_u64(&flags, &ptr, FILE_OFFSET_CLOSED));
        let moved_mask = self.allocate_register();
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
        // resource record, but only `File`'s open helpers zero these words after
        // the PRNG-poisoned arena alloc — a socket's record leaves 32..80 as
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
        if free_record {
            // bug-623: last, after the close and the blocks the record points at —
            // the record itself (`resource_record_freed_at_drop`). A moved record
            // took the skip above and is not freed here. The slot is zeroed so a
            // drop re-reached on it reads 0 and skips.
            let record = self.allocate_register();
            self.emit(abi::load_u64(&record, abi::stack_pointer(), resource_slot));
            self.emit(abi::move_register(abi::c_arg(0), &record));
            self.emit(abi::move_immediate(
                abi::c_arg(1),
                "Integer",
                RESOURCE_RECORD_SIZE,
            ));
            self.emit_arena_free_call();
            self.emit(abi::store_u64(
                abi::ZERO,
                abi::stack_pointer(),
                resource_slot,
            ));
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
    pub(crate) fn emit_free_resource_block(
        &mut self,
        resource_slot: usize,
        offset: usize,
        size: &str,
    ) -> Result<(), String> {
        let skip = self.label("resource_block_free_skip");
        let ptr = self.allocate_register();
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), resource_slot));
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, &ptr, offset));
        self.emit(abi::compare_immediate(&block, "0"));
        self.emit(abi::branch_eq(&skip));
        // plan-71-C Family-1a: block ptr is arg 0 of arena-free → `%arg0`.
        self.emit(abi::move_register(abi::c_arg(0), &block));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", size));
        self.emit_arena_free_call();
        // Reload: the call above destroyed every caller-saved register.
        let ptr_after = self.allocate_register();
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
    /// fields — so it is sized from the type at `RESOURCE_OFFSET_STATE`.
    pub(crate) fn emit_free_resource_state_block(
        &mut self,
        resource_slot: usize,
        state_type: &ParameterType,
    ) -> Result<(), String> {
        let skip = self.label("resource_state_free_skip");
        let state_slot = self.allocate_stack_object("resource_state_free_ptr", 8);
        let size_slot = self.allocate_stack_object("resource_state_free_size", 8);
        let ptr = self.allocate_register();
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), resource_slot));
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, &ptr, RESOURCE_OFFSET_STATE));
        self.emit(abi::store_u64(&block, abi::stack_pointer(), state_slot));
        self.emit(abi::compare_immediate(&block, "0"));
        self.emit(abi::branch_eq(&skip));
        self.emit_inlined_block_size_from_ptr_slot(state_type, state_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            state_slot,
        ));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit_arena_free_call();
        let ptr_after = self.allocate_register();
        self.emit(abi::load_u64(
            &ptr_after,
            abi::stack_pointer(),
            resource_slot,
        ));
        self.emit(abi::store_u64(abi::ZERO, &ptr_after, RESOURCE_OFFSET_STATE));
        self.emit(abi::label(&skip));
        Ok(())
    }

    pub(crate) fn record_secondary_cleanup_failure(&mut self) {
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

    /// The drop descriptor for one owned-list's resource leaf (bug-805): a single
    /// close-op symbol for a concrete resource, or a tag-dispatch table + uniform
    /// STATE type for a resource union (bug-429).
    ///
    /// An owned-list node is always a leaf — a resource record, or a resource-union
    /// box — whatever container it floated to, so the descriptor is the leaf's own
    /// and never an aggregate's. A container reaching several leaf kinds gets one
    /// list per kind ([`Self::setup_owned_list`]), so a record with a `File` field
    /// and a `Stream` field is dropped correctly rather than refused.
    pub(crate) fn resource_leaf_drop(&self, leaf: &ParameterType) -> Result<OwnedListDrop, String> {
        // A resource-union leaf has no single close op; each node is dropped by
        // dispatching on its active variant's tag, exactly as a lone
        // `RES u AS Union STATE S` binding is (plan-74). Checked first — a union's
        // variants are individually closable, so the concrete arm would otherwise
        // never see them.
        if let Some(variants) = self.resource_union_cleanup(leaf) {
            return Ok(OwnedListDrop::Union {
                variants,
                state_type: leaf.state(),
                record_free_tags: self.resource_union_record_free_tags(leaf),
            });
        }
        let symbol = self
            .resource_cleanup_symbol(leaf)
            .ok_or_else(|| format!("owned-list leaf type '{leaf}' has no registered close op"))?;
        Ok(OwnedListDrop::Concrete {
            close: symbol,
            state_type: leaf.state(),
            has_io_buffers: Self::resource_uses_io_buffers(leaf),
            frees_record: Self::resource_record_freed_at_drop(leaf),
        })
    }
}
