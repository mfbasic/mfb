// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::builder_collection_layout::list_entry_stride;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::{
    typed_is_collection_type, typed_list_element_type, typed_strip_res_marker,
};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::visit::{walk_value, NirVisitor};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    /// Allocate and register the runtime owned-lists for an owner container binding
    /// (§15.6): one head-pointer stack slot (initialized empty) plus one
    /// [`ActiveCleanup::OwnedList`] obligation per resource leaf kind the
    /// container's values can reach, each drained on every exit path.
    ///
    /// bug-805: the container is any value that reaches a resource — a
    /// `List OF RES File`, a record with a `RES` field, but equally a
    /// `List OF List OF Holder` or a record holding a `Map OF K TO Holder`. A
    /// pushed node is always a resource leaf (a record, or a resource-union box),
    /// never an aggregate, so the drop is chosen by the leaf, not by the
    /// container's immediate payload. A container reaching two kinds (a record with
    /// a `File` field and a `Stream` field) gets two lists, one drop each.
    pub(crate) fn setup_owned_list(
        &mut self,
        name: &str,
        type_: &ParameterType,
    ) -> Result<(), String> {
        let leaves = self.resource_leaf_types(type_);
        if leaves.is_empty() {
            return Err(format!(
                "owned-list owner '{name}' of type '{type_}' reaches no resource"
            ));
        }
        let drops = leaves
            .iter()
            .map(|leaf| Ok((leaf.clone(), self.resource_leaf_drop(leaf)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let owns_elements = self.owned_list_owning_collections.contains(name);
        // bug-645: the collection BLOCK itself. A `List OF RES X` is not
        // `is_freeable_flat_value` — `collection_payload_types` strips the `RES`
        // marker, so the walk sees a bare resource nominal and answers "not flat"
        // (`a_res_collection_does_not_diverge` pins that, and it is what keeps such a
        // list out of `copy_flat_block`). The consequence nobody had measured is that
        // NO scope-drop frees the block either: an owner collection rebuilt in a loop
        // leaked one whole list block per iteration (48 B empty, 240 B and up once
        // appended to) on top of its elements.
        //
        // The free is registered as an ordinary [`ActiveCleanup::OwnedValue`], pushed
        // BEFORE the drain so it runs after it, because every rule that governs it is
        // already written there and keyed on the slot: `plan_returned_move` retires it
        // when the list is RETURNed, `trap_route_cleanups` keeps a function-level one
        // live for the handler to read, and `_mfb_rt_drop_owned_collection` null-guards
        // and nulls the slot, so a re-reached drop frees nothing. Gated on the same
        // sole-ownership proof as the elements: copy-insertion does NOT deep-copy such
        // a list (it is not flat), so an aliasing store really can leave two holders,
        // and only `owning_collections` rules that out.
        //
        // Restricted to a collection that is not freeable-flat: a record with a `RES`
        // field (plan-114-C), or a collection of such records, IS freeable-flat (its
        // fields walk unstripped to the `Res(_)` arm) and so already has its own
        // `OwnedValue` from the bind — adding a second here would be the double free
        // this whole file guards against.
        if owns_elements && typed_is_collection_type(type_) && !self.is_freeable_flat_value(type_) {
            if let Some(stack_offset) = self.locals.get(name).map(|local| local.stack_offset) {
                self.register_res_collection_block_free(name, type_, stack_offset);
            }
        }
        for (leaf, drop) in drops {
            let head_slot = self.allocate_stack_object(&format!("owned_list_{name}"), 8);
            let scratch9 = self.temporary_vreg();
            self.emit(abi::move_immediate(&scratch9, "Integer", "0"));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), head_slot));
            self.owned_list_heads
                .insert((name.to_string(), leaf.clone()), head_slot);
            self.active_cleanups
                .push(ActiveCleanup::OwnedList(OwnedListCleanup {
                    name: name.to_string(),
                    head_slot,
                    drop,
                    owns_elements,
                    scope_depth: self.current_scope_depth(),
                    leaf,
                }));
        }
        Ok(())
    }

    /// bug-645/bug-651: register the free of a `RES` collection's own block as an
    /// ordinary [`ActiveCleanup::OwnedValue`]. Shared by the floated container (where it
    /// is pushed BEFORE the drain so it runs after it) and the unfloated one (which has
    /// no drain at all).
    ///
    /// Every rule that governs it is already written for `OwnedValue` and keyed on the
    /// slot: `plan_returned_move` retires it when the collection is RETURNed,
    /// `trap_route_cleanups` keeps a function-level one live for the handler to read, and
    /// `_mfb_rt_drop_owned_collection` null-guards and nulls the slot, so a re-reached
    /// drop frees nothing. It frees the container block only — the elements are resources
    /// with their own close obligations.
    ///
    /// Sole ownership is NOT assumed here: both callers gate on
    /// `owned_list_owning_collections`, which is `record_ownership`'s verdict that the
    /// block is fresh, non-escaping and never read through a borrowed-element call.
    /// Copy-insertion does not deep-copy such a list (it is not flat), so an aliasing
    /// store really can leave two holders and only that pass rules it out.
    pub(crate) fn register_res_collection_block_free(
        &mut self,
        name: &str,
        type_: &ParameterType,
        stack_offset: usize,
    ) {
        if self.inner_owning_collections.contains(name) {
            self.inner_block_slots.insert(stack_offset);
        }
        self.active_cleanups
            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                type_: type_.clone(),
                stack_offset,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            }));
        self.owned_value_slots.push(stack_offset);
    }

    /// Transfer a returned resource collection's owned-list to the caller: drop
    /// its drain obligation from this scope so the resources are not closed here
    /// (the caller's scope adopts and closes them). Other scopes' owned-lists are
    /// untouched (§15.6).
    ///
    /// bug-645: the block free `setup_owned_list` paired with the drain goes with it.
    /// `plan_returned_move` usually gets there first (it retires the same cleanup by
    /// slot), but it declines for several shapes — an address-taken local, a `FOR EACH`
    /// iterable — and the container is still transferred on every one of them. Left
    /// registered, this scope would free the block the caller is about to adopt.
    pub(crate) fn deactivate_owned_list(&mut self, name: &str) {
        // bug-805: a container reaching several resource kinds registered one list
        // per kind, all under its name.
        self.active_cleanups
            .retain(|cleanup| !matches!(cleanup, ActiveCleanup::OwnedList(o) if o.name == name));
        let Some(stack_offset) = self.locals.get(name).map(|local| local.stack_offset) else {
            return;
        };
        if let Some(index) = self.active_cleanups.iter().rposition(|cleanup| {
            matches!(cleanup, ActiveCleanup::OwnedValue(o) if o.stack_offset == stack_offset)
        }) {
            self.active_cleanups.remove(index);
        }
    }

    /// Whether a type is a `RES`-marked resource collection
    /// (`List OF RES File`, `Map OF K TO RES File`): its scope-ownership transfers
    /// across a function boundary (§15.6).
    ///
    /// plan-106-E: the `RES` marker is a [`ParameterType::Res`] variant, so this is
    /// the element/value shape, not a prefix on its spelling.
    /// bug-753: a collection whose BLOCK a sole-holder binding may free without
    /// closing anything: a `RES`-marked collection, or a list of such collections
    /// (`collections::chunks`/`window` over a `List OF RES T`). A nested list holds
    /// its inner lists inline in its own data region, so freeing the outer block
    /// releases them all; their elements are references with their own owners.
    pub(crate) fn is_res_collection_block(type_: &ParameterType) -> bool {
        Self::is_res_marked_resource_collection(type_)
            || matches!(type_, ParameterType::ListOf(element) if Self::is_res_collection_block(element))
    }

    pub(crate) fn is_res_marked_resource_collection(type_: &ParameterType) -> bool {
        let marked = |type_: &ParameterType| matches!(type_, ParameterType::Res(_));
        match type_ {
            ParameterType::ListOf(element) => marked(element),
            ParameterType::MapOf(_, value) => marked(value),
            _ => false,
        }
    }

    /// `RETURN` of owner container `name`: the returned block is the caller's now,
    /// so retire the block free `setup_owned_list` registered — but keep its
    /// owned-list drains. bug-805: removing them (as [`Self::deactivate_owned_list`]
    /// does for a consuming call, whose callee takes the records over) leaked the
    /// 16-byte node of every resource floated into a returned container, and left a
    /// record the container no longer held owned by a dead frame, never closed. The
    /// `RETURN` releases every record the returned value reaches before the
    /// cleanups run, so the drains skip exactly those.
    pub(crate) fn retire_returned_owner_block(&mut self, name: &str) {
        let Some(stack_offset) = self.locals.get(name).map(|local| local.stack_offset) else {
            return;
        };
        if let Some(index) = self.active_cleanups.iter().rposition(|cleanup| {
            matches!(cleanup, ActiveCleanup::OwnedValue(o) if o.stack_offset == stack_offset)
        }) {
            self.active_cleanups.remove(index);
        }
    }

    /// Whether a type is a container whose scope-ownership of resources
    /// transfers across a function boundary (§15.6): any aggregate that reaches a
    /// resource (bug-805) — a `RES`-marked collection, a record with a `RES` field
    /// (plan-114-C), or any list, map, record or data union nesting one.
    ///
    /// This is what the `RETURN` transfer keys on. Getting it wrong for records
    /// is not a missing optimisation: a returned float-target record whose
    /// owned-list is *drained* instead of transferred closes the handle the
    /// caller is about to adopt, and the caller then closes it again.
    pub(crate) fn is_resource_owning_container(&self, type_: &ParameterType) -> bool {
        !self.is_resource_leaf(type_) && self.type_carries_resources(type_)
    }

    /// Push the resource record (or resource-union box) at `resource_slot` onto
    /// `collection`'s owned-list for `leaf` as a fresh `{value, next}` node (§15.6).
    ///
    /// `leaf` is the pushed value's own resource type. A container with one list
    /// takes every push on it, as before bug-805; one with several lists picks the
    /// list whose leaf is `leaf`, or the one sharing its base resource when the
    /// slot's `STATE` spelling differs from the pushed binding's.
    pub(crate) fn emit_owned_list_push(
        &mut self,
        collection: &str,
        resource_slot: usize,
        leaf: &ParameterType,
    ) -> Result<(), String> {
        let leaf = typed_strip_res_marker(leaf).clone();
        let lists: Vec<(ParameterType, usize)> = self
            .owned_list_heads
            .iter()
            .filter(|((name, _), _)| name == collection)
            .map(|((_, kind), slot)| (kind.clone(), *slot))
            .collect();
        let chosen = lists
            .iter()
            .find(|(kind, _)| *kind == leaf)
            .or_else(|| {
                let mut same_base = lists
                    .iter()
                    .filter(|(kind, _)| kind.without_state() == leaf.without_state());
                match (same_base.next(), same_base.next()) {
                    (Some(only), None) => Some(only),
                    _ => None,
                }
            })
            .or_else(|| (lists.len() == 1).then(|| &lists[0]))
            .cloned();
        let Some((kind, head_slot)) = chosen else {
            return Err(format!(
                "resource of type '{leaf}' floats to '{collection}', which has no owned-list \
                 for it"
            ));
        };
        // bug-755: the record floats up to the collection's scope — unless it is
        // already owned there or further out, which a float never lowers.
        let list = self
            .active_cleanups
            .iter()
            .rev()
            .find_map(|cleanup| match cleanup {
                ActiveCleanup::OwnedList(list) if list.name == collection && list.leaf == kind => {
                    Some((
                        list.scope_depth,
                        matches!(list.drop, OwnedListDrop::Union { .. }),
                    ))
                }
                _ => None,
            });
        if let Some((depth, union)) = list {
            let value = self.allocate_register();
            self.emit(abi::load_u64(&value, abi::stack_pointer(), resource_slot));
            let record = if union {
                let skip = self.label("owned_list_float_union_null");
                let record = self.allocate_register();
                self.emit(abi::move_immediate(&record, "Integer", "0"));
                self.emit(abi::compare_immediate(&value, "0"));
                self.emit(abi::branch_eq(&skip));
                self.emit(abi::load_u64(&record, &value, 8));
                self.emit(abi::label(&skip));
                record
            } else {
                value
            };
            self.emit_resource_float(&abi::Reg::from(record), depth)?;
        }
        // Allocate a 16-byte node (record ptr at 0, next at 8).
        let alloc_ok = self.label("owned_list_alloc_ok");
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::move_immediate(abi::return_register(), "Integer", "16"));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        // x1 = node pointer.
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            resource_slot,
        ));
        self.emit(abi::store_u64(&scratch9, abi::mfb_return(1), 0));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), head_slot));
        self.emit(abi::store_u64(&scratch10, abi::mfb_return(1), 8));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            head_slot,
        ));
        Ok(())
    }

    /// bug-805: the container locals this body ASSIGNS a call result that can carry
    /// resources the callee hands out (`xs = makeFiles()`). A `Bind` of a call result
    /// adopts at the bind; an assignment has nowhere to register a drain except the
    /// binding's own scope, so these locals get their owned-lists at their bind, like
    /// a float target, and the assignment seeds them. Before this the resources such
    /// an assignment received were owned by nobody and never closed — a buffered
    /// write through them was lost.
    pub(crate) fn call_assigned_owner_containers(
        &self,
        body: &[crate::target::shared::nir::NirOp],
    ) -> std::collections::HashSet<String> {
        use crate::target::shared::nir::visit::walk_op;
        use crate::target::shared::nir::NirOp;
        struct Scan {
            types: std::collections::HashMap<String, ParameterType>,
            assigned: Vec<String>,
        }
        impl NirVisitor for Scan {
            fn visit_op(&mut self, op: &NirOp) {
                match op {
                    NirOp::Bind { name, type_, .. } => {
                        self.types.insert(name.clone(), type_.clone());
                    }
                    NirOp::Assign { name, value } if adopts_from_call(value) => {
                        self.assigned.push(name.clone());
                    }
                    _ => {}
                }
                walk_op(self, op);
            }
        }
        let mut scan = Scan {
            types: std::collections::HashMap::new(),
            assigned: Vec::new(),
        };
        scan.visit_ops(body);
        scan.assigned
            .into_iter()
            .filter(|name| {
                scan.types
                    .get(name)
                    .is_some_and(|type_| self.is_resource_owning_container(type_))
            })
            .collect()
    }

    /// Adopt the resources a value transferred in from a call carries: walk every
    /// resource the value of `collection_type` in `collection_slot` reaches (at any
    /// depth, bug-805) and push each onto this scope's owned-list for its kind, so
    /// the scope closes each once at exit (§15.6).
    pub(crate) fn emit_owned_list_seed_from_collection(
        &mut self,
        collection: &str,
        collection_slot: usize,
        collection_type: &ParameterType,
    ) -> Result<(), String> {
        let collection = collection.to_string();
        // bug-755: the push floats each resource to this scope. One the callee
        // released (unowned) is taken; one still owned further out — a handle the
        // caller passed in and got back — keeps its owner, and the drain skips it.
        self.emit_for_each_resource(
            collection_slot,
            collection_type,
            &mut |builder, leaf_slot, leaf| {
                builder.emit_owned_list_push(&collection, leaf_slot, leaf)
            },
        )
    }

    /// Drain an owned-list: walk it head-first, closing each record once. The
    /// close is closed-flag idempotent, so a record reachable through more than
    /// one path closes exactly once (§15.6). A concrete-resource element closes
    /// via its single registered close op; a resource-union element (bug-429) is
    /// tag-dispatched to the active variant's close op and its uniform STATE
    /// block freed — the same drop a lone `RES u AS Union STATE S` binding runs,
    /// via the shared `emit_union_tag_dispatch_drop`.
    ///
    /// **bug-645: the drain reclaims, it does not only close.** Every node it visits
    /// is a 16-byte `{record, next}` block `emit_owned_list_push` allocated and this
    /// loop is the only reader of — nothing else can name it — so that block is freed
    /// unconditionally. The blocks the node POINTS at are a different question, and
    /// answered by `cleanup.owns_elements` (`record_ownership::owning_collections`):
    /// with the collection proved the one owner, the concrete arm runs the same
    /// `emit_resource_block_reclaim` an `ActiveCleanup::Resource` drop runs, and the
    /// union arm passes its real `record_free_tags` and frees the `{tag, record}` box
    /// the wrap allocated. Unproved, both fall back to close-only — the leak this fix
    /// narrows, never a double free.
    pub(crate) fn emit_owned_list_drain(
        &mut self,
        cleanup: &OwnedListCleanup,
    ) -> Result<(), String> {
        let loop_label = self.label("owned_list_drain_loop");
        let done_label = self.label("owned_list_drain_done");
        let close_ok = self.label("owned_list_close_ok");
        // The node pointer must survive the close and the frees below, all of which
        // clobber every caller-saved register (`.ai/compiler.md`), so it lives in a
        // slot from here on.
        let node_slot = self.allocate_stack_object("owned_list_drain_node", 8);
        let payload_slot = self.allocate_stack_object("owned_list_drain_payload", 8);
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
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), node_slot));
        // Advance the head past this node before the call(s) below, which clobber
        // caller-saved registers; the loop reloads the head from memory. Node
        // field 0 is the value to close — the record pointer for a concrete
        // element, the union block pointer (`{tag@0, record-ptr@8}`) for a union.
        match &cleanup.drop {
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
                    cleanup.head_slot,
                ));
                let node_done = self.label("owned_list_node_done");
                // A node's record is never null in a well-formed list, but the close
                // helper dereferences the closed flag, so guard it like every other
                // resource drop (bug-246).
                self.emit(abi::compare_immediate(abi::return_register(), "0"));
                self.emit(abi::branch_eq(&node_done));
                // bug-755: close only a record this list's scope still owns. A record
                // that was a reference here (owned further out) or that a `RETURN`
                // released to the caller is someone else's to close.
                let record = self.allocate_register();
                self.emit(abi::move_register(&record, abi::return_register()));
                self.emit_skip_unless_owned(
                    &abi::Reg::from(record.clone()),
                    cleanup.scope_depth,
                    &node_done,
                )?;
                self.emit(abi::move_register(abi::return_register(), &record));
                self.emit_symbol_call(&close);
                self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
                self.emit(abi::branch_eq(&close_ok));
                self.record_secondary_cleanup_failure();
                self.emit(abi::label(&close_ok));
                if cleanup.owns_elements {
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
                self.emit(abi::label(&node_done));
            }
            OwnedListDrop::Union {
                variants,
                state_type,
                record_free_tags,
            } => {
                let variants = variants.clone();
                let state_type = state_type.clone();
                let record_free_tags = record_free_tags.clone();
                let union_ptr = self.allocate_register();
                self.emit(abi::load_u64(&union_ptr, &scratch9, 0));
                self.emit(abi::store_u64(
                    &union_ptr,
                    abi::stack_pointer(),
                    payload_slot,
                ));
                self.emit(abi::load_u64(&scratch10, &scratch9, 8));
                self.emit(abi::store_u64(
                    &scratch10,
                    abi::stack_pointer(),
                    cleanup.head_slot,
                ));
                // Defensive null-skip: a well-formed floated union node is
                // non-null, but guard like the single-binding path so a null can
                // never fault the tag load at `union_ptr+0` (bug-246).
                let node_done = self.label("owned_list_union_node_done");
                let dropped = self.label("owned_list_union_drop_box");
                self.emit(abi::compare_immediate(&union_ptr, "0"));
                self.emit(abi::branch_eq(&node_done));
                // bug-755: as the concrete arm — the active variant's record must be
                // owned by this list's scope.
                let record = self.allocate_register();
                self.emit(abi::load_u64(&record, &union_ptr, 8));
                self.emit(abi::compare_immediate(&record, "0"));
                self.emit(abi::branch_eq(&node_done));
                self.emit_skip_unless_owned(
                    &abi::Reg::from(record),
                    cleanup.scope_depth,
                    &node_done,
                )?;
                self.emit_union_tag_dispatch_drop(
                    &union_ptr,
                    &variants,
                    state_type.as_ref(),
                    if cleanup.owns_elements {
                        &record_free_tags
                    } else {
                        &[]
                    },
                    &dropped,
                )?;
                // Both a matched variant (which branches here) and an unrecognised tag
                // (which falls out of the dispatch) converge on the box free, mirroring
                // `emit_resource_union_cleanup_call`'s `dropped` label.
                self.emit(abi::label(&dropped));
                if cleanup.owns_elements {
                    let box_ptr = self.allocate_register();
                    self.emit(abi::load_u64(&box_ptr, abi::stack_pointer(), payload_slot));
                    self.emit(abi::move_register(abi::c_arg(0), &box_ptr));
                    self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "16"));
                    self.emit_arena_free_call();
                }
                self.emit(abi::label(&node_done));
            }
        }
        // The node block itself: allocated by `emit_owned_list_push`, reachable only
        // through the chain this loop has already advanced past, so this free is
        // unaliased and once-only regardless of what the element turned out to own.
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            node_slot,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "16"));
        self.emit_arena_free_call();
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    /// Free an owned, non-escaping flat value at scope-drop (plan-01 Phase 5 /
    /// plan-02 Phase 8): recompute the block's byte size from its static type and
    /// `arena_free(ptr, size)`. `arena_free` scrubs the bytes (entropy poison),
    /// so a later use-after-free reads garbage and traps loudly. Clobbers
    /// caller-saved scratch; the caller reloads anything it needs afterward.
    /// bug-753: free each inner block of the nested collection in `slot` (an
    /// inner-owning binding, `record_ownership::block_collections`), before the
    /// caller frees the outer block. The inner lists are held by pointer, so each is
    /// its own block; their elements are references with their own owners.
    fn emit_inner_block_frees(
        &mut self,
        slot: usize,
        inner_type: &ParameterType,
    ) -> Result<(), String> {
        let done = self.label("inner_block_free_done");
        let null_check = self.temporary_vreg();
        self.emit(abi::load_u64(&null_check, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&null_check, "0"));
        self.emit(abi::branch_eq(&done));
        let cursor_slot = self.allocate_stack_object("inner_free_cursor", 8);
        let remaining_slot = self.allocate_stack_object("inner_free_remaining", 8);
        let inner_slot = self.allocate_stack_object("inner_free_block", 8);
        self.initialize_collection_loop_slots(slot, cursor_slot, remaining_slot, inner_type);
        let top = self.label("inner_block_free_loop");
        self.emit(abi::label(&top));
        let remaining = self.temporary_vreg();
        self.emit(abi::load_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&done));
        let inner = self.load_collection_loop_item(slot, cursor_slot, inner_type)?;
        self.emit(abi::store_u64(&inner, abi::stack_pointer(), inner_slot));
        let leaf = typed_list_element_type(inner_type)
            .cloned()
            .unwrap_or_else(|| ParameterType::named(""));
        self.emit(abi::add_immediate(
            abi::c_arg(0),
            abi::stack_pointer(),
            inner_slot,
        ));
        self.emit(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            &list_entry_stride(&leaf).to_string(),
        ));
        self.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
        self.emit(abi::branch_link(DROP_OWNED_COLLECTION_SYMBOL));
        self.push_internal_call_relocation(DROP_OWNED_COLLECTION_SYMBOL);
        self.advance_collection_loop(cursor_slot, remaining_slot, &top, inner_type);
        self.emit(abi::label(&done));
        Ok(())
    }

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
            if self.inner_block_slots.contains(&cleanup.stack_offset) {
                self.emit_inner_block_frees(cleanup.stack_offset, &element)?;
            }
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
        // clears `builder_owned_cleanup.rs:201`.
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

/// bug-643: whether every resource in `value`'s result is a pointer one of the
/// call's arguments already held, so the binding it is assigned to must not adopt
/// the handles as fresh close obligations.
///
/// A `List OF RES X` bound from a call adopts every handle in the list (§15.6's
/// transfer on `RETURN`). That is right for a callee that PRODUCED them, and wrong
/// for a `collections` member that only re-packages the caller's own pointers. Those
/// pointers already have an owner, which encloses the binding: an argument is read at
/// the call, so its owning scope is live there, and a parameter's owner is in a
/// caller's frame. Adopting them made the binding's scope close the caller's handles
/// when it exited — before their owner did, for a binding in an inner scope. The
/// `chunks`/`window` bodies bind each block in their `WHILE` body, so every handle
/// copied into a block was closed after that iteration.
///
/// True when `value` calls a `collections` member that cannot produce a resource
/// ([`collections_member_produces_elements`]) and no argument can produce one either:
/// each is free of calls, or is itself such a call. An argument with any other call
/// in it (`take(makeFiles(), 1)`) keeps the adoption, because that call's handles may
/// be fresh.
pub(crate) fn call_result_holds_only_argument_resources(value: &NirValue) -> bool {
    let (NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. }) = value
    else {
        return false;
    };
    let Some(member) = collections_call_member(target) else {
        return false;
    };
    !collections_member_produces_elements(member) && args.iter().all(value_produces_no_resource)
}

/// Whether a binding or assignment of `value` adopts the resources it carries: a
/// call, unless it is a `collections` member that only re-packages its arguments'
/// handles (bug-643).
pub(crate) fn adopts_from_call(value: &NirValue) -> bool {
    matches!(value, NirValue::Call { .. } | NirValue::CallResult { .. })
        && !call_result_holds_only_argument_resources(value)
}

/// An argument subtree whose resources are all pointers already bound to a name:
/// no call at all, or only calls [`call_result_holds_only_argument_resources`]
/// accepts. A closure's body is a separate function, so its captures are what is
/// visited here, never its code.
fn value_produces_no_resource(value: &NirValue) -> bool {
    struct Calls {
        clean: bool,
    }
    impl NirVisitor for Calls {
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Call { .. } | NirValue::CallResult { .. } => {
                    if !call_result_holds_only_argument_resources(value) {
                        self.clean = false;
                    }
                }
                NirValue::RuntimeCall { .. } => self.clean = false,
                _ => walk_value(self, value),
            }
        }
    }
    let mut calls = Calls { clean: true };
    calls.visit_value(value);
    calls.clean
}

/// The `collections` member a call target names: `collections.X` for a native
/// member, `#collections_X$<types>` for an instantiated source-generic body — which
/// includes the internal `slice` helper the `take`/`drop`/`chunks`/`window` bodies
/// call. `None` for any other target.
fn collections_call_member(target: &str) -> Option<&str> {
    match target.strip_prefix("#collections_") {
        Some(rest) => rest.split('$').next(),
        None => target.strip_prefix("collections."),
    }
}

/// Whether a `collections` member's result can hold a value none of its arguments
/// held: some callback parameter returns a type that also appears in the member's
/// return type (`transform`'s `FUNC(T) AS U` into `List OF U`, `reduce`'s
/// accumulator, `mapValues`). Such a callback may open a resource and hand it back,
/// and that handle is fresh. Every other member only re-packages its arguments'
/// elements. A name the registry does not declare is an internal source helper
/// (`slice`), which takes no callback.
fn collections_member_produces_elements(member: &str) -> bool {
    let Some(function) = crate::codegen::registry::registry()
        .resolve_package("collections")
        .and_then(|package| package.function(member))
    else {
        return false;
    };
    function.implementations.iter().any(|implementation| {
        // `reduce` declares its return as "the type of argument 1" (the accumulator).
        let return_type = match &implementation.return_type {
            ParameterType::Arg(index) => implementation
                .params
                .get(*index)
                .map_or(&implementation.return_type, |parameter| &parameter.ty),
            other => other,
        };
        let mut returned = Vec::new();
        type_variables(return_type, &mut returned);
        implementation
            .params
            .iter()
            .any(|parameter| match &parameter.ty {
                ParameterType::Func(_, callback_return, _) => {
                    let mut produced = Vec::new();
                    type_variables(callback_return, &mut produced);
                    produced.iter().any(|var| returned.contains(var))
                }
                _ => false,
            })
    })
}

/// The type variables `type_` mentions, at any depth of the shapes a `collections`
/// signature uses.
fn type_variables(type_: &ParameterType, out: &mut Vec<ParameterType>) {
    match type_ {
        ParameterType::Var(_) => out.push(type_.clone()),
        ParameterType::ListOf(inner)
        | ParameterType::SetOf(inner)
        | ParameterType::ResultOf(inner)
        | ParameterType::Res(inner) => type_variables(inner, out),
        ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
            type_variables(key, out);
            type_variables(value, out);
        }
        ParameterType::UserOf(_, args) => args.iter().for_each(|arg| type_variables(arg, out)),
        ParameterType::Func(params, ret, _) => {
            params.iter().for_each(|param| type_variables(param, out));
            type_variables(ret, out);
        }
        _ => {}
    }
}

#[cfg(test)]
mod argument_resource_tests {
    use super::*;
    use crate::target::shared::nir::NirSourceLoc;

    fn call(target: &str, args: Vec<NirValue>) -> NirValue {
        NirValue::Call {
            target: target.to_string(),
            args,
            loc: NirSourceLoc::default(),
        }
    }

    fn local(name: &str) -> NirValue {
        NirValue::Local(name.to_string())
    }

    /// bug-643: the members that re-package their arguments' elements. `slice` is the
    /// internal helper the `chunks`/`window`/`take`/`drop` bodies call; it is not a
    /// registry function at all. `sortBy` has a callback, but its key never becomes
    /// an element.
    #[test]
    fn a_repackaging_member_produces_no_elements() {
        for member in [
            "chunks", "window", "take", "drop", "mid", "slice", "filter", "sortBy", "flatten",
            "values", "append", "prepend", "insert", "set", "removeAt", "get", "getOr",
        ] {
            assert!(
                !collections_member_produces_elements(member),
                "{member} only re-packages its arguments"
            );
        }
    }

    /// The members whose callback result becomes (part of) the result: a handle it
    /// opens is fresh, so a binding of the result must still adopt it.
    #[test]
    fn a_member_whose_callback_builds_the_result_produces_elements() {
        for member in ["transform", "mapValues", "reduce", "reduceRight"] {
            assert!(
                collections_member_produces_elements(member),
                "{member}'s callback result reaches its return value"
            );
        }
    }

    /// The `chunks` body's own binding: every argument is a name or a constant.
    #[test]
    fn a_slice_of_named_arguments_holds_only_argument_resources() {
        let value = call(
            "#collections_slice$RES$fs$File",
            vec![local("value"), local("i"), local("stop")],
        );
        assert!(call_result_holds_only_argument_resources(&value));
    }

    /// Nesting re-packagers keeps the answer: `take(filter(xs, pred), 1)`.
    #[test]
    fn a_nested_repackaging_call_holds_only_argument_resources() {
        let predicate = NirValue::Closure {
            name: "lambda_0".to_string(),
            type_: ParameterType::Unknown,
            captures: Vec::new(),
        };
        let inner = call(
            "#collections_filter$RES$fs$File",
            vec![local("xs"), predicate],
        );
        let value = call("#collections_take$RES$fs$File", vec![inner, local("n")]);
        assert!(call_result_holds_only_argument_resources(&value));
    }

    /// Any other call among the arguments may hand back fresh handles, so the
    /// binding keeps adopting what it receives.
    #[test]
    fn an_argument_from_another_call_keeps_the_adoption() {
        let value = call(
            "#collections_take$RES$fs$File",
            vec![call("makeFiles", Vec::new()), local("n")],
        );
        assert!(!call_result_holds_only_argument_resources(&value));
    }

    /// A user function or a producing member is never answered for: its result
    /// may be fresh.
    #[test]
    fn a_user_call_or_a_producing_member_keeps_the_adoption() {
        assert!(!call_result_holds_only_argument_resources(&call(
            "openAll",
            vec![local("paths")]
        )));
        assert!(!call_result_holds_only_argument_resources(&call(
            "#collections_transform$String$RES$fs$File",
            vec![local("paths"), local("opener")]
        )));
        assert!(!call_result_holds_only_argument_resources(&local("xs")));
    }
}
