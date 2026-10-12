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
/// bug-1012: bytes in an owned-list node of a resource-union leaf,
/// `{tag@0, record@8, next@16}` — a copy of the leaf's box, then the link.
const OWNED_LIST_UNION_NODE_SIZE: usize = 24;
/// Where such a node keeps its link.
const OWNED_LIST_UNION_NODE_NEXT_OFFSET: usize = 16;

/// bug-1053: what one walk over an owned-list does with each node.
#[derive(Clone, Copy)]
enum OwnedListPass {
    /// Close each record the list's scope owns, free what it points at when the
    /// list is proved its one holder, and free the node.
    Drain,
    /// Close each record the list's scope owns, mark it taken and move its node to
    /// the reclaim list in this slot; free the node of any other record.
    Take(usize),
    /// Free what each record on the reclaim list in this slot points at, the
    /// record, and its node.
    Reclaim(usize),
}

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
    fn setup_owned_list_at(
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
    fn emit_owned_list_push_as(
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
                self.emit(abi::compare_registers(owner, token));
                self.emit(abi::branch_lo(&take));
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
        struct Scan<'t> {
            types: std::collections::HashMap<String, ParameterType>,
            assigned: Vec<String>,
            trapped: &'t std::collections::HashSet<String>,
        }
        impl NirVisitor for Scan<'_> {
            fn visit_op(&mut self, op: &NirOp) {
                match op {
                    NirOp::Bind { name, type_, .. } => {
                        self.types.insert(name.clone(), type_.clone());
                    }
                    NirOp::Assign { name, value }
                        if trapped_call_source(value)
                            .is_some_and(|source| self.trapped.contains(source)) =>
                    {
                        self.assigned.push(name.clone());
                    }
                    _ => {}
                }
                walk_op(self, op);
            }
        }
        let trapped = trap_call_result_locals(body);
        let mut scan = Scan {
            types: std::collections::HashMap::new(),
            assigned: Vec::new(),
            trapped: &trapped,
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

    /// Rule 4, for an assignment: when `value` can carry a resource its statement
    /// owns, raise every resource container `name` reaches after the store to
    /// the container's scope and list the ones it takes. A record the container
    /// already listed, or one owned further out, is skipped by the push.
    pub(crate) fn emit_assigned_container_float(
        &mut self,
        name: &str,
        value: &NirValue,
    ) -> Result<(), String> {
        // The store already floated the one item it added (the in-place
        // lowerings, `emit_stored_item_float`): nothing else was stored.
        if std::mem::take(&mut self.statement_temps.item_floated) {
            return Ok(());
        }
        // A store through a by-reference capture: the whole stored value, by the
        // owner word (bug-1075).
        if let Some(local) = self.locals.get(name).filter(|local| local.by_ref) {
            let (reference, type_) = (local.stack_offset, local.type_.clone());
            if !self.type_carries_resources(&type_) || self.is_resource_leaf(&type_) {
                return Ok(());
            }
            let cell = self.allocate_register();
            self.emit(abi::load_u64(cell, abi::stack_pointer(), reference));
            let block = self.allocate_register();
            self.emit(abi::load_u64(block, cell, 0));
            let slot = self.spill_to_slot("captured_store_value", block);
            return self.emit_captured_store_float(name, slot, &type_);
        }
        if !self
            .store_floats
            .stores
            .contains(&(value as *const NirValue as usize))
        {
            return Ok(());
        }
        let Some(local) = self.locals.get(name).filter(|local| !local.by_ref) else {
            return Ok(());
        };
        let (slot, type_) = (local.stack_offset, local.type_.clone());
        if !self.is_resource_owning_container(&type_)
            || !(self.owned_list_heads.keys().any(|(list, _)| list == name)
                || self.dynamic_owned_list(name).is_some())
        {
            return Ok(());
        }
        self.emit_owned_list_seed_from_collection(name, slot, &type_)
    }

    /// Rule 4 at the store itself: `item` is the one value an in-place store
    /// (an append, a set) adds to container `name`, so it is all the store can
    /// carry in. Every resource it reaches is raised to the container's scope
    /// and listed there when taken; the walk is the item's size, never the
    /// container's.
    pub(crate) fn emit_stored_item_float(
        &mut self,
        name: &str,
        mut item: ValueResult,
    ) -> Result<ValueResult, String> {
        if !self.type_carries_resources(&item.type_) {
            return Ok(item);
        }
        let by_ref = self.locals.get(name).is_some_and(|local| local.by_ref);
        if !by_ref
            && (!self.statement_temps.assign_floats
                || !(self.owned_list_heads.keys().any(|(list, _)| list == name)
                    || self.dynamic_owned_list(name).is_some()))
        {
            return Ok(item);
        }
        let slot = self.spill_to_slot("stored_item_float", &item.location);
        if by_ref {
            // What a captured binding's scope is relative to this value's is not
            // known in this frame: the owner word decides.
            self.emit_captured_store_float(name, slot, &item.type_.clone())?;
        } else {
            let container = name.to_string();
            self.emit_for_each_resource(slot, &item.type_, &mut |builder, leaf_slot, leaf| {
                builder.emit_owned_list_push_as(&container, leaf_slot, leaf, true)
            })?;
        }
        self.statement_temps.item_floated = true;
        // The pushes allocate nodes, and an arena call destroys every
        // caller-saved register: hand the store its item back from the slot.
        let reloaded = self.allocate_register();
        self.emit(abi::load_u64(reloaded, abi::stack_pointer(), slot));
        item.location = crate::codegen::engine::operand::Operand::from(reloaded);
        Ok(item)
    }

    /// bug-1075: the locals of `body` that a `LAMBDA` captures by reference and
    /// that reach a resource.
    pub(crate) fn by_ref_resource_captures(
        &self,
        body: &[crate::target::shared::nir::NirOp],
    ) -> std::collections::HashSet<String> {
        struct Scan<'b, 'c> {
            builder: &'b CodeBuilder<'c>,
            names: std::collections::HashSet<String>,
        }
        impl NirVisitor for Scan<'_, '_> {
            fn visit_value(&mut self, value: &NirValue) {
                if let NirValue::Closure { captures, .. } = value {
                    for capture in captures {
                        if let NirValue::LocalRef { name, type_ } = capture {
                            if self.builder.type_carries_resources(type_) {
                                self.names.insert(name.clone());
                            }
                        }
                    }
                }
                walk_value(self, value);
            }
        }
        let mut scan = Scan {
            builder: self,
            names: std::collections::HashSet::new(),
        };
        scan.visit_ops(body);
        scan.names
    }

    /// bug-1075: the bytes of the cell of a `type_` binding a `LAMBDA` captures
    /// by reference: `{value@0, scope token@8, one owned-list head per resource
    /// leaf kind@16…}`.
    pub(crate) fn ref_cell_size(&self, type_: &ParameterType) -> usize {
        16 + 8 * self.resource_leaf_types(type_).len()
    }

    /// bug-1075: arm the cell of a binding a `LAMBDA` captures by reference
    /// (`StatementTemps::ref_cells`): this scope's token at `+8`, and at `+16…`
    /// the heads of owned-lists that are drained, and whose records are freed,
    /// on every exit of this scope like any other container's.
    pub(crate) fn setup_ref_cell(
        &mut self,
        name: &str,
        type_: &ParameterType,
        stack_offset: usize,
    ) -> Result<(), String> {
        let depth = self.current_scope_depth();
        let token = self.emit_scope_token(depth)?;
        self.emit(abi::store_u64(
            token,
            abi::stack_pointer(),
            stack_offset + 8,
        ));
        self.setup_owned_list_at(&format!("{name}$ref"), type_, Some(stack_offset + 16))
    }

    /// Rule 4 for a store through a by-reference capture (bug-1075): the binding
    /// lives in the frame that created the `LAMBDA`, and its cell carries its
    /// scope's token and its owned-list heads. Every resource the value in
    /// `value_slot` (of `type_`, a value the `container` binding now holds)
    /// reaches is raised to that scope and listed there when taken; one owned
    /// there or further out keeps its owner.
    pub(crate) fn emit_captured_store_float(
        &mut self,
        name: &str,
        value_slot: usize,
        type_: &ParameterType,
    ) -> Result<(), String> {
        let Some(local) = self.locals.get(name).filter(|local| local.by_ref) else {
            return Ok(());
        };
        let (reference, container) = (local.stack_offset, local.type_.clone());
        let kinds = self.resource_leaf_types(&container);
        let sp = abi::stack_pointer();
        let cell = self.allocate_register();
        self.emit(abi::load_u64(cell, sp, reference));
        let cell_slot = self.spill_to_slot("captured_store_cell", cell);
        self.emit_for_each_resource(value_slot, type_, &mut |builder, leaf_slot, leaf| {
            let leaf = typed_strip_res_marker(leaf).clone();
            let Some(index) = kinds.iter().position(|kind| *kind == leaf).or_else(|| {
                kinds
                    .iter()
                    .position(|kind| kind.without_state() == leaf.without_state())
            }) else {
                return Err(format!(
                    "a resource of type '{leaf}' is stored through a captured reference \
                     to a '{container}', which lists no such resource (bug-1075)"
                ));
            };
            let union = builder.resource_union_cleanup(&kinds[index]).is_some();
            let done = builder.label("captured_store_done");
            let take = builder.label("captured_store_take");
            let value = builder.allocate_register();
            builder.emit(abi::load_u64(value, sp, leaf_slot));
            builder.emit(abi::compare_immediate(value, "0"));
            builder.emit(abi::branch_eq(&done));
            let record = if union {
                let record = builder.allocate_register();
                builder.emit(abi::load_u64(record, value, 8));
                builder.emit(abi::compare_immediate(record, "0"));
                builder.emit(abi::branch_eq(&done));
                record
            } else {
                value
            };
            let owner = builder.allocate_register();
            builder.emit(abi::load_u64(owner, record, RESOURCE_OFFSET_OWNER));
            let cell = builder.allocate_register();
            builder.emit(abi::load_u64(cell, sp, cell_slot));
            let token = builder.allocate_register();
            builder.emit(abi::load_u64(token, cell, 8));
            // Unsigned, as `emit_resource_float`: taken only when below the token.
            builder.emit(abi::compare_registers(owner, token));
            builder.emit(abi::branch_lo(&take));
            builder.emit(abi::branch(&done));
            builder.emit(abi::label(&take));
            builder.emit(abi::store_u64(token, record, RESOURCE_OFFSET_OWNER));
            let size = if union {
                OWNED_LIST_UNION_NODE_SIZE
            } else {
                16
            };
            let alloc_ok = builder.label("captured_store_alloc_ok");
            builder.emit(abi::move_immediate(
                abi::return_register(),
                "Integer",
                &size.to_string(),
            ));
            builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
            builder.emit_arena_alloc_call();
            builder.emit(abi::branch_eq(&alloc_ok));
            builder.raise_error_bare("ErrOutOfMemory")?;
            builder.emit(abi::label(&alloc_ok));
            let node = builder.allocate_register();
            builder.emit(abi::move_register(node, abi::mfb_return(1)));
            let value = builder.allocate_register();
            builder.emit(abi::load_u64(value, sp, leaf_slot));
            let cell = builder.allocate_register();
            builder.emit(abi::load_u64(cell, sp, cell_slot));
            let head_offset = 16 + 8 * index;
            let head = builder.allocate_register();
            builder.emit(abi::load_u64(head, cell, head_offset));
            if union {
                let word = builder.allocate_register();
                builder.emit(abi::load_u64(word, value, 0));
                builder.emit(abi::store_u64(word, node, 0));
                builder.emit(abi::load_u64(word, value, 8));
                builder.emit(abi::store_u64(word, node, 8));
                builder.emit(abi::store_u64(
                    head,
                    node,
                    OWNED_LIST_UNION_NODE_NEXT_OFFSET,
                ));
            } else {
                builder.emit(abi::store_u64(value, node, 0));
                builder.emit(abi::store_u64(head, node, 8));
            }
            builder.emit(abi::store_u64(node, cell, head_offset));
            builder.emit(abi::label(&done));
            Ok(())
        })
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
                builder.emit_owned_list_push_as(&collection, leaf_slot, leaf, true)
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
    /// `arena_free(ptr, size)`. `arena_free` scrubs the bytes (entropy poison),
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

/// Rule 4 (`mfb spec language resource-management`, "The owner of a resource"):
/// whether storing `value` into a place that outlives the statement can carry a
/// resource the STATEMENT owns, so the taker must float what the stored value
/// reaches up to its own scope. A value read from named places only reaches
/// resources already owned at those places' scopes or further out; only a call
/// somewhere in the expression can have handed the statement one. Where in the
/// expression the call sits does not matter.
pub(crate) fn value_may_carry_statement_resources(value: &NirValue) -> bool {
    !value_produces_no_resource(value)
}

/// bug-842: the `Result` local a `resultValue(<local>)` reads — the inline-`TRAP`
/// desugar's delivery `$trap_valN = resultValue($trap_resN)`.
fn trapped_call_source(value: &NirValue) -> Option<&str> {
    match value {
        NirValue::ResultValue { value } => match value.as_ref() {
            NirValue::Local(name) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// bug-842: the locals bound to a resource-adopting call's `Result` — the inline
/// `TRAP` desugar binds `$trap_resN = callResult f(…)` and delivers the success
/// value by assignment, so the binding that receives it is assigned no call at all.
/// Without this, `LET xs = mkList() TRAP … END TRAP` adopted nothing and the list's
/// resources were never closed.
pub(crate) fn trap_call_result_locals(
    body: &[crate::target::shared::nir::NirOp],
) -> std::collections::HashSet<String> {
    use crate::target::shared::nir::visit::walk_op;
    use crate::target::shared::nir::NirOp;
    struct Scan {
        names: std::collections::HashSet<String>,
    }
    impl NirVisitor for Scan {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::Bind {
                name,
                value: Some(value @ NirValue::CallResult { .. }),
                ..
            } = op
            {
                if adopts_from_call(value) {
                    self.names.insert(name.clone());
                }
            }
            walk_op(self, op);
        }
    }
    let mut scan = Scan {
        names: std::collections::HashSet::new(),
    };
    scan.visit_ops(body);
    scan.names
}

impl CodeBuilder<'_> {
    /// bug-842: whether an assignment of `value` delivers a resource-adopting call's
    /// success value through the inline-`TRAP` desugar (`trap_call_result_locals`).
    pub(crate) fn adopts_from_trapped_call(&self, value: &NirValue) -> bool {
        trapped_call_source(value).is_some_and(|source| self.trap_call_results.contains(source))
    }
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
pub(crate) fn collections_call_member(target: &str) -> Option<&str> {
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
pub(crate) fn type_variables(type_: &ParameterType, out: &mut Vec<ParameterType>) {
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
