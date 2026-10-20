//! plan-176-O: building a scope's owned list — setting it up, retiring a returned owner block, and
//! pushing nodes. The pass state and the container floats live in
//! `cleanup/owned/builder_owned_cleanup/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Allocate and register the runtime owned-lists for an owner container binding
    /// (§15.6): one head-pointer stack slot (initialized empty) plus one
    /// [`ActiveCleanup::OwnedList`] obligation per resource leaf kind the
    /// container's values can reach, each drained on every exit path.
    ///
    /// bug-805: the container is any value that reaches a resource — a
    /// `List OF RES File`, a record with a `RES` field, but equally a
    /// `List OF List OF Holder` or a record holding a `Map OF K TO Holder`. A
    /// pushed node is always a resource leaf (a record, or a copy of a
    /// resource-union box's tag and record), never an aggregate, so the drop is
    /// chosen by the leaf, not by the
    /// container's immediate payload. A container reaching two kinds (a record with
    /// a `File` field and a `Stream` field) gets two lists, one drop each.
    pub(crate) fn setup_owned_list(
        &mut self,
        name: &str,
        type_: &ParameterType,
    ) -> Result<(), String> {
        self.setup_owned_list_at(name, type_, None)
    }

    /// [`Self::setup_owned_list`] with the list heads at `first_head`,
    /// `first_head + 8`, … (one per leaf kind, in `resource_leaf_types` order)
    /// instead of fresh stack objects: the heads of a by-reference capture cell
    /// (bug-1075), which the capturing `LAMBDA`'s frame pushes onto.
    pub(super) fn setup_owned_list_at(
        &mut self,
        name: &str,
        type_: &ParameterType,
        first_head: Option<usize>,
    ) -> Result<(), String> {
        let leaves = self.resource_leaf_types(type_);
        // bug-843: a holder of function values can come to own whatever kinds the
        // module's closures carry, which its type does not name.
        let dynamic = self.type_reaches_closure_leaf(type_);
        if leaves.is_empty() && !dynamic {
            return Err(format!(
                "owned-list owner '{name}' of type '{type_}' reaches no resource"
            ));
        }
        let drops = leaves
            .iter()
            .map(|leaf| Ok((leaf.clone(), self.resource_leaf_drop(leaf)?)))
            .collect::<Result<Vec<_>, String>>()?;
        // The scope that closes a record frees it: the drain takes the records
        // its scope owns and the exit frees them after every other cleanup has
        // run (bug-1053). No holder proof is asked.
        let owns_elements = false;
        let reclaims = true;
        for (index, (leaf, drop)) in drops.into_iter().enumerate() {
            let head_slot = match first_head {
                Some(first) => {
                    self.owned_value_slots.push(first + 8 * index);
                    first + 8 * index
                }
                None => self.allocate_stack_object(&format!("owned_list_{name}"), 8),
            };
            let scratch9 = self.temporary_vreg();
            self.emit(abi::move_immediate(&scratch9, "Integer", "0"));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), head_slot));
            self.owned_list_heads
                .insert((name.to_string(), leaf.clone()), head_slot);
            // A leaf whose record stays as the closed tombstone and that has no
            // buffer or `STATE` of its own (a `canvas` image or font) leaves the
            // reclaim nothing to free: its drain stays as it was.
            let reclaimable = match &drop {
                OwnedListDrop::Concrete {
                    state_type,
                    has_io_buffers,
                    frees_record,
                    ..
                } => *frees_record || *has_io_buffers || state_type.is_some(),
                // The first walk's dispatch already frees a union leaf's `STATE`.
                OwnedListDrop::Union {
                    io_buffer_tags,
                    record_free_tags,
                    ..
                } => !io_buffer_tags.is_empty() || !record_free_tags.is_empty(),
                OwnedListDrop::Dynamic => false,
            };
            let reclaim_slot = (reclaims && reclaimable).then(|| {
                // Zeroed with the frame, never here: a walk that an exit path
                // emitted before this point reads it empty, and a reclaim always
                // leaves it empty.
                let slot = self.allocate_stack_object(&format!("owned_list_reclaim_{name}"), 8);
                self.owned_value_slots.push(slot);
                slot
            });
            let cleanup = OwnedListCleanup {
                name: name.to_string(),
                head_slot,
                drop,
                owns_elements,
                reclaim_slot,
                scope_depth: self.current_scope_depth(),
                leaf,
            };
            if reclaim_slot.is_some() {
                self.reclaiming_owned_lists.push(cleanup.clone());
            }
            self.active_cleanups.push(ActiveCleanup::OwnedList(cleanup));
        }
        if dynamic && first_head.is_none() {
            self.setup_dynamic_owned_list(name);
        }
        Ok(())
    }

    /// A consuming call took over the resources container `name` owns: drop its
    /// owned-list drains so this scope does not close them too (§15.6). Other scopes'
    /// owned-lists are untouched.
    ///
    /// The container's BLOCK stays this scope's to free. A consuming callee
    /// (`canvas::setGroup`) copies the items and adopts the resources they name, never
    /// the caller's block — so the `OwnedValue` free at `name`'s slot must survive.
    /// Removing it here was right only while this also served `RETURN` (bug-645), whose
    /// block really is the caller's; `RETURN` now goes through
    /// [`Self::retire_returned_owner_block`]. Once bug-805 widened
    /// [`Self::is_resource_owning_container`] to every aggregate reaching a resource,
    /// `List OF canvas::DrawItem` qualified, and removing the free leaked the whole
    /// items block on every `setGroup` (33,328 B per call for 40 rectangles).
    pub(crate) fn deactivate_owned_list(&mut self, name: &str) {
        // bug-805: a container reaching several resource kinds registered one list
        // per kind, all under its name.
        self.active_cleanups
            .retain(|cleanup| !matches!(cleanup, ActiveCleanup::OwnedList(o) if o.name == name));
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

    /// Push the resource record (or the tag and record of the resource-union box)
    /// at `resource_slot` onto `collection`'s owned-list for `leaf` as a fresh node
    /// (§15.6).
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
        self.emit_owned_list_push_as(collection, resource_slot, leaf, false)
    }

    /// [`Self::emit_owned_list_push`], told whether the push ADOPTS a record out of
    /// a value a call handed over (`adopting`) or floats a binding's own.
    ///
    /// bug-1000: an adopted value can reach a record that already has an owner at
    /// the container's scope or further out — a handle the caller passed in and got
    /// back (`LET xs = wrap(a)`), or one this container already lists
    /// (`xs = addTo(xs)`). Its owner closes and frees it. Listing it again made the
    /// drain a second closer of a record at its own depth, and, once the drain frees
    /// what it closes, the first to free a record the other still reads. So an
    /// adopting push lists a record only when it takes it: unowned (the callee
    /// released it), or owned by a scope inside the container's (which floats up).
    pub(super) fn emit_owned_list_push_as(
        &mut self,
        collection: &str,
        resource_slot: usize,
        leaf: &ParameterType,
        adopting: bool,
    ) -> Result<(), String> {
        let leaf = typed_strip_res_marker(leaf).clone();
        // bug-843: a container holding function values lists what each closure
        // carries on its dynamic owned-list. A closure leaf (an adopted call result's
        // function value) is asked through its environment routine; a resource that
        // no static list of the container names (one a captured value carried in) is
        // pushed there directly.
        let dynamic = self.dynamic_owned_list(collection);
        if self.is_closure_leaf(&leaf) {
            let Some(list) = dynamic else {
                return Err(format!(
                    "a function value floats to '{collection}', which has no dynamic \
                     owned-list (bug-843)"
                ));
            };
            let (address_slot, token_slot) = self.emit_dynamic_list_arguments(list)?;
            return self.emit_closure_resource_op(
                resource_slot,
                CLOSURE_ENV_OP_ADOPT,
                &[address_slot, token_slot],
            );
        }
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
            .or_else(|| (lists.len() == 1 && dynamic.is_none()).then(|| &lists[0]))
            .cloned();
        if chosen.is_none() {
            if let Some(list) = dynamic {
                let (address_slot, token_slot) = self.emit_dynamic_list_arguments(list)?;
                return self.emit_dynamic_owned_push(
                    resource_slot,
                    &leaf,
                    address_slot,
                    token_slot,
                    false,
                );
            }
        }
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
        let union_leaf = self.resource_union_cleanup(&kind).is_some();
        // bug-1046: a concrete resource floating into a container of its resource
        // UNION (`RES f AS fs::File`, then `hs = append(hs, f)` on a `List OF RES
        // Handle`). The float is decided at the binding, before the store wraps
        // the value, so the slot holds the member's RECORD, not a `{tag, record}`
        // box. The list's nodes are union nodes: the record is the value and the
        // tag is the member's, both known here (as `module_leaf_kind` does for a
        // module-level container, bug-1027). Reading the record as a box took its
        // handle word for the record pointer.
        let member_tag = if union_leaf && self.resource_union_cleanup(&leaf).is_none() {
            let member = leaf.without_state();
            let is_member = self
                .type_model
                .variants_for_union(&kind.without_state())
                .any(|variant| *variant == member);
            match (is_member, self.type_model.union_variant_tags.get(&member)) {
                (true, Some(tag)) => Some(*tag),
                _ => {
                    return Err(format!(
                        "resource of type '{leaf}' floats to '{collection}', whose owned-list \
                         holds '{kind}', which '{leaf}' is not a member of"
                    ));
                }
            }
        } else {
            None
        };
        self.emit_owned_list_push_node(
            head_slot,
            list,
            union_leaf,
            resource_slot,
            adopting,
            member_tag,
        )
    }

    /// The push itself: float (or, `adopting`, take) the record for the list's
    /// scope and link a fresh node at `head_slot`. `list` is the list's
    /// `(scope depth, holds resource-union leaves)` while its drain is registered.
    ///
    /// A concrete leaf's node is `{record@0, next@8}`. A resource-union leaf's node
    /// is `{tag@0, record@8, next@16}` ([`OWNED_LIST_UNION_NODE_SIZE`]): it COPIES
    /// the two words of the box in `resource_slot` and never keeps the box's
    /// address (bug-1012). The box is a binding's, a statement's, or a span of the
    /// collection that holds the element inline — each with its own lifetime, none
    /// of them the list's: a node that pointed at a binding's box was read after
    /// that binding freed it, and one that pointed into an adopted collection's
    /// block had the drain free 16 bytes out of the middle of that block. The first
    /// two words of the node are laid out as a box, so the drain hands the node's
    /// own address to the tag-dispatched close.
    ///
    /// `member_tag` is `Some` when a union leaf's `resource_slot` holds a MEMBER's
    /// concrete record instead of a box (bug-1046): the node takes that record and
    /// this tag, so it is the node a box of the member would have produced.
    pub(crate) fn emit_owned_list_push_node(
        &mut self,
        head_slot: usize,
        list: Option<(usize, bool)>,
        union_leaf: bool,
        resource_slot: usize,
        adopting: bool,
        member_tag: Option<usize>,
    ) -> Result<(), String> {
        // Only an adopting push can skip, so only it gets the label to skip to.
        let listed = adopting.then(|| self.label("owned_list_adopt_listed"));
        if let Some((depth, union)) = list {
            let value = self.allocate_register();
            self.emit(abi::load_u64(&value, abi::stack_pointer(), resource_slot));
            let record = if union && member_tag.is_none() {
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
            if let Some(listed) = &listed {
                let take = self.label("owned_list_adopt_take");
                self.emit(abi::compare_immediate(record, "0"));
                self.emit(abi::branch_eq(&take));
                let owner = self.allocate_register();
                self.emit(abi::load_u64(owner, record, RESOURCE_OFFSET_OWNER));
                let token = self.emit_scope_token(depth)?;
                // Unsigned, as `emit_resource_float`: an unowned record (`0`) and
                // one owned deeper are below the token and are taken.
                self.emit_branch_if_takeable(owner, token, &take);
                self.emit(abi::branch(listed));
                self.emit(abi::label(&take));
            }
            self.emit_resource_float(&abi::Reg::from(record), depth)?;
        }
        if union_leaf {
            // A `{tag@0, record@8, next@16}` node holding a copy of the box's words.
            let alloc_ok = self.label("owned_list_alloc_ok");
            self.emit(abi::move_immediate(
                abi::return_register(),
                "Integer",
                &OWNED_LIST_UNION_NODE_SIZE.to_string(),
            ));
            self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
            self.emit_arena_alloc_call();
            self.emit(abi::branch_eq(&alloc_ok));
            self.raise_error_bare("ErrOutOfMemory")?;
            self.emit(abi::label(&alloc_ok));
            let node = self.allocate_register();
            self.emit(abi::move_register(node, abi::mfb_return(1)));
            // A null box lists a null record, which the drain skips.
            let copied = self.label("owned_list_union_node_copied");
            let tag = self.allocate_register();
            let record = self.allocate_register();
            self.emit(abi::move_immediate(tag, "Integer", "0"));
            self.emit(abi::move_immediate(record, "Integer", "0"));
            let source = self.allocate_register();
            self.emit(abi::load_u64(source, abi::stack_pointer(), resource_slot));
            self.emit(abi::compare_immediate(source, "0"));
            self.emit(abi::branch_eq(&copied));
            match member_tag {
                // The slot holds the member's record: the node is its box.
                Some(member_tag) => {
                    self.emit(abi::move_immediate(tag, "Integer", &member_tag.to_string()));
                    self.emit(abi::move_register(record, source));
                }
                None => {
                    self.emit(abi::load_u64(tag, source, 0));
                    self.emit(abi::load_u64(record, source, 8));
                }
            }
            self.emit(abi::label(&copied));
            self.emit(abi::store_u64(tag, node, 0));
            self.emit(abi::store_u64(record, node, 8));
            let head = self.allocate_register();
            self.emit(abi::load_u64(head, abi::stack_pointer(), head_slot));
            self.emit(abi::store_u64(
                head,
                node,
                OWNED_LIST_UNION_NODE_NEXT_OFFSET,
            ));
            self.emit(abi::store_u64(node, abi::stack_pointer(), head_slot));
        } else {
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
        }
        if let Some(listed) = &listed {
            self.emit(abi::label(listed));
        }
        Ok(())
    }
}
