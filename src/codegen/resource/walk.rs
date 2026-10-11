//! bug-805/bug-806: one type-directed walk over every resource a value reaches.
//!
//! A resource can sit at any depth of an ordinary value: a `List OF RES File`
//! element, a record's `RES` field, a `List OF List OF Holder`'s inner record, a
//! data-union variant's field, a map value. Every consumer that has to touch "the
//! resources this value carries" — a `RETURN` releasing them to the caller, a
//! caller's binding adopting what a call handed back, a discarded call result
//! closing what nobody took, a module-level store raising them to the module
//! instance — walks the same set, so the walk is written once, here, and each
//! consumer only says what to do with one resource.
//!
//! A **leaf** is a concrete resource (its value IS the resource record) or a
//! resource union (its value is the address of a `{tag@0, record@8}` box — a
//! binding's or a statement's block, or the two words a record field or a
//! collection element holds inline; a visitor reads them and never keeps the
//! address, bug-1012). Everything else the
//! walk descends: `List`/`Map` elements through the ordinary collection loop (a
//! map skips its tombstones, bug-760 B1), record fields through the same
//! `8 * index` slot read `lower_field_access` makes (an inlined field adds the
//! record base), and a data union's active variant — the record it inlines at
//! `+16` — behind a tag test. A field or
//! element whose type reaches no resource is never visited.
//!
//! **Recursive types.** A record that reaches itself (`TYPE Node { h AS RES File,
//! kids AS List OF Node }`) cannot be expanded inline — the emitted code would be
//! infinite, and a value-depth recursion on the native stack overflows on deep
//! data (`.ai/codegen-invariants.md`, "A recursive value's deep copy must never
//! recurse on the native stack"). The walk expands each composite inline until it
//! meets a type it is already expanding; that value is pushed onto a run-time
//! worklist of `{value, kind, next}` arena nodes instead, and a dispatch loop after
//! the inline walk pops and expands each deferred value by its kind. Code size is
//! one expansion per deferred kind; run-time depth is a heap list, not the stack.
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::typed_strip_res_marker;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;
use std::collections::HashSet;

/// What one resource visit is handed: the stack slot holding the leaf value (a
/// record pointer, a resource-union box pointer, or — bug-843 — a function value whose
/// closure may carry resources), and the leaf's type with the `RES` marker stripped
/// and any `STATE` kept.
pub(crate) type ResourceVisit<'a> =
    dyn FnMut(&mut CodeBuilder<'_>, usize, &ParameterType) -> Result<(), String> + 'a;

/// The run-time worklist a walk over a recursive type uses (see the module doc).
struct Deferred {
    /// Stack slot holding the worklist head (`0` = empty).
    head_slot: usize,
    /// The composite type of each deferred kind, by kind index.
    kinds: Vec<ParameterType>,
}

/// Where a data union inlines its active variant's record (plan-02 §4.3).
const DATA_UNION_VARIANT_OFFSET: usize = 16;

/// Bytes in one worklist node: `{value@0, kind@8, next@16}`.
const WORKLIST_NODE_SIZE: usize = 24;

impl CodeBuilder<'_> {
    /// Whether `type_` is itself a resource leaf: a concrete resource or a
    /// resource union (the `RES` marker is ignored).
    pub(crate) fn is_resource_leaf(&self, type_: &ParameterType) -> bool {
        let bare = typed_strip_res_marker(type_);
        self.resource_cleanup_symbol(bare).is_some() || self.resource_union_cleanup(bare).is_some()
    }

    /// Whether a value of `type_` can reach a resource at any depth: it is a
    /// resource leaf, or a list/map/record/data union one of whose parts is.
    pub(crate) fn type_carries_resources(&self, type_: &ParameterType) -> bool {
        self.carries_resources_inner(type_, &mut HashSet::new())
    }

    fn carries_resources_inner(
        &self,
        type_: &ParameterType,
        seen: &mut HashSet<ParameterType>,
    ) -> bool {
        let bare = typed_strip_res_marker(type_);
        if self.is_resource_leaf(bare) || self.is_closure_leaf(bare) {
            return true;
        }
        match bare {
            ParameterType::ListOf(element) => self.carries_resources_inner(element, seen),
            ParameterType::MapOf(_, value) => self.carries_resources_inner(value, seen),
            _ => {
                // A nominal type already on the path contributes nothing new: a
                // cycle reaches a resource only through some other part of it.
                if !seen.insert(bare.clone()) {
                    return false;
                }
                let carries = self
                    .composite_parts(bare)
                    .into_iter()
                    .any(|(_, _, part)| self.carries_resources_inner(&part, seen));
                seen.remove(bare);
                carries
            }
        }
    }

    /// bug-843: whether a value of `type_` can hold a function value that carries
    /// resources — a closure leaf at any depth. Such a holder can come to own
    /// resources of kinds its type does not name, so it needs a dynamic owned-list
    /// (`closure_carry`).
    pub(crate) fn type_reaches_closure_leaf(&self, type_: &ParameterType) -> bool {
        self.closures_carry_resources() && self.reaches_func_inner(type_, &mut HashSet::new())
    }

    fn reaches_func_inner(&self, type_: &ParameterType, seen: &mut HashSet<ParameterType>) -> bool {
        let bare = typed_strip_res_marker(type_);
        match bare {
            ParameterType::Func(..) => true,
            ParameterType::ListOf(element) => self.reaches_func_inner(element, seen),
            ParameterType::MapOf(_, value) => self.reaches_func_inner(value, seen),
            _ => {
                if self.is_resource_leaf(bare) || !seen.insert(bare.clone()) {
                    return false;
                }
                let reaches = self
                    .composite_parts(bare)
                    .into_iter()
                    .any(|(_, _, part)| self.reaches_func_inner(&part, seen));
                seen.remove(bare);
                reaches
            }
        }
    }

    /// The distinct resource leaves a value of `type_` can reach, in first-seen
    /// order: the drop descriptors an owned-list for such a value needs.
    pub(crate) fn resource_leaf_types(&self, type_: &ParameterType) -> Vec<ParameterType> {
        let mut out = Vec::new();
        self.collect_leaves(type_, &mut HashSet::new(), &mut out);
        out
    }

    fn collect_leaves(
        &self,
        type_: &ParameterType,
        seen: &mut HashSet<ParameterType>,
        out: &mut Vec<ParameterType>,
    ) {
        let bare = typed_strip_res_marker(type_);
        if self.is_resource_leaf(bare) {
            if !out.contains(bare) {
                out.push(bare.clone());
            }
            return;
        }
        match bare {
            ParameterType::ListOf(element) => self.collect_leaves(element, seen, out),
            ParameterType::MapOf(_, value) => self.collect_leaves(value, seen, out),
            _ => {
                if !seen.insert(bare.clone()) {
                    return;
                }
                for (_, _, part) in self.composite_parts(bare) {
                    self.collect_leaves(&part, seen, out);
                }
            }
        }
    }

    /// The parts of a nominal composite, as `(variant tag, offset, part type)`: a
    /// record's fields (tag `None`, the field's slot at `8 * index`), or a data
    /// union's variants (the variant's tag, offset 16 — a data union is
    /// `{tag@0, size@8, variant record inlined @16}`, plan-02 §4.3, so the part is
    /// the ADDRESS `base + 16`, a record walked like any other). Empty for
    /// anything else, a resource union included (that is a leaf).
    fn composite_parts(&self, type_: &ParameterType) -> Vec<(Option<usize>, usize, ParameterType)> {
        if let Some(fields) = self.type_model.record_fields.get(type_) {
            return fields
                .iter()
                .enumerate()
                .map(|(index, (_, field))| (None, 8 * index, field.clone()))
                .collect();
        }
        if self.type_model.union_names.contains(type_)
            && self.resource_union_cleanup(type_).is_none()
        {
            return self
                .type_model
                .variants_for_union(type_)
                .filter(|variant| self.type_model.record_fields.contains_key(*variant))
                .filter_map(|variant| {
                    let tag = self.type_model.union_variant_tags.get(variant).copied()?;
                    Some((Some(tag), DATA_UNION_VARIANT_OFFSET, variant.clone()))
                })
                .collect();
        }
        Vec::new()
    }

    /// Emit a walk over every resource the value of `type_` in `slot` reaches,
    /// calling `visit` once per leaf with a slot holding the leaf value. A null
    /// container, record or union along the way is skipped; a null leaf is handed
    /// to `visit`, whose own null guard skips it (every resource consumer has one).
    ///
    /// `visit` may call runtime helpers: every live value of the walk is kept in a
    /// stack slot and reloaded after it.
    pub(crate) fn emit_for_each_resource(
        &mut self,
        slot: usize,
        type_: &ParameterType,
        visit: &mut ResourceVisit<'_>,
    ) -> Result<(), String> {
        if !self.type_carries_resources(type_) {
            return Ok(());
        }
        let mut deferred = None;
        let mut expanding = Vec::new();
        self.emit_walk_value(slot, type_, visit, &mut expanding, &mut deferred)?;
        if let Some(deferred) = deferred {
            self.emit_deferred_walks(deferred, visit)?;
        }
        Ok(())
    }

    fn emit_walk_value(
        &mut self,
        slot: usize,
        type_: &ParameterType,
        visit: &mut ResourceVisit<'_>,
        expanding: &mut Vec<ParameterType>,
        deferred: &mut Option<Deferred>,
    ) -> Result<(), String> {
        let bare = typed_strip_res_marker(type_).clone();
        // bug-843: a function value is a leaf too — its closure is asked, through its
        // environment routine, for the resources it carries (`closure_carry`).
        if self.is_resource_leaf(&bare) || self.is_closure_leaf(&bare) {
            return visit(self, slot, &bare);
        }
        if !self.type_carries_resources(&bare) {
            return Ok(());
        }
        match &bare {
            ParameterType::ListOf(element) => {
                let element = (**element).clone();
                self.emit_walk_list(slot, &element, visit, expanding, deferred)
            }
            ParameterType::MapOf(_, value) => {
                let value = (**value).clone();
                self.emit_walk_map(slot, &value, visit, expanding, deferred)
            }
            _ => {
                if expanding.contains(&bare) {
                    return self.emit_defer(slot, &bare, deferred);
                }
                expanding.push(bare.clone());
                let result = self.emit_walk_composite(slot, &bare, visit, expanding, deferred);
                expanding.pop();
                result
            }
        }
    }

    fn emit_walk_list(
        &mut self,
        slot: usize,
        element: &ParameterType,
        visit: &mut ResourceVisit<'_>,
        expanding: &mut Vec<ParameterType>,
        deferred: &mut Option<Deferred>,
    ) -> Result<(), String> {
        let element_type = typed_strip_res_marker(element).clone();
        let done = self.label("resource_walk_list_done");
        let list = self.allocate_register();
        self.emit(abi::load_u64(&list, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&list, "0"));
        self.emit(abi::branch_eq(&done));
        let cursor_slot = self.allocate_stack_object("resource_walk_cursor", 8);
        let remaining_slot = self.allocate_stack_object("resource_walk_remaining", 8);
        let item_slot = self.allocate_stack_object("resource_walk_item", 8);
        self.initialize_collection_loop_slots(slot, cursor_slot, remaining_slot, &element_type);
        let top = self.label("resource_walk_list_loop");
        self.emit(abi::label(&top));
        let remaining = self.allocate_register();
        self.emit(abi::load_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(slot, cursor_slot, &element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        self.emit_walk_value(item_slot, element, visit, expanding, deferred)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &top, &element_type);
        self.emit(abi::label(&done));
        Ok(())
    }

    fn emit_walk_map(
        &mut self,
        slot: usize,
        value: &ParameterType,
        visit: &mut ResourceVisit<'_>,
        expanding: &mut Vec<ParameterType>,
        deferred: &mut Option<Deferred>,
    ) -> Result<(), String> {
        let value_type = typed_strip_res_marker(value).clone();
        let done = self.label("resource_walk_map_done");
        let map = self.allocate_register();
        self.emit(abi::load_u64(&map, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&map, "0"));
        self.emit(abi::branch_eq(&done));
        let index_slot = self.allocate_stack_object("resource_walk_index", 8);
        let item_slot = self.allocate_stack_object("resource_walk_item", 8);
        let zero = self.allocate_register();
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::store_u64(&zero, abi::stack_pointer(), index_slot));
        let top = self.label("resource_walk_map_loop");
        let next = self.label("resource_walk_map_next");
        self.emit(abi::label(&top));
        let map = self.allocate_register();
        let index = self.allocate_register();
        let count = self.allocate_register();
        self.emit(abi::load_u64(&map, abi::stack_pointer(), slot));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(&count, &map, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&done));
        let entry = self.allocate_register();
        let stride = self.allocate_register();
        self.emit(abi::move_immediate(
            &stride,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&entry, &index, &stride));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&entry, &map, &entry));
        // bug-760 B1: a map held by a binding may carry tombstones; a removed
        // entry's value is no longer part of the map.
        let flags = self.allocate_register();
        self.emit(abi::load_u8(&flags, &entry, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::compare_immediate(
            &flags,
            &COLLECTION_ENTRY_FLAG_REMOVED.to_string(),
        ));
        self.emit(abi::branch_eq(&next));
        let offset = self.allocate_register();
        let length = self.allocate_register();
        self.emit(abi::load_u64(
            &offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &length,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        let item = self.emit_load_map_payload(&value_type, &map, &offset, &length)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        self.emit_walk_value(item_slot, value, visit, expanding, deferred)?;
        self.emit(abi::label(&next));
        let index = self.allocate_register();
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// A record's or data union's resource-carrying parts, read exactly as
    /// `lower_field_access` reads them.
    fn emit_walk_composite(
        &mut self,
        slot: usize,
        type_: &ParameterType,
        visit: &mut ResourceVisit<'_>,
        expanding: &mut Vec<ParameterType>,
        deferred: &mut Option<Deferred>,
    ) -> Result<(), String> {
        let is_record = self.type_model.record_fields.contains_key(type_);
        let done = self.label("resource_walk_composite_done");
        let base = self.allocate_register();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&base, "0"));
        self.emit(abi::branch_eq(&done));
        let parts: Vec<_> = self
            .composite_parts(type_)
            .into_iter()
            .filter(|(_, _, part)| self.type_carries_resources(part))
            .collect();
        for (tag, offset, part) in parts {
            let skip = self.label("resource_walk_part_skip");
            let base = self.allocate_register();
            self.emit(abi::load_u64(&base, abi::stack_pointer(), slot));
            if let Some(tag) = tag {
                let active = self.allocate_register();
                self.emit(abi::load_u64(&active, &base, 0));
                let expected = self.allocate_register();
                self.emit(abi::move_immediate(&expected, "Integer", &tag.to_string()));
                self.emit(abi::compare_registers(&active, &expected));
                self.emit(abi::branch_ne(&skip));
            }
            let value = self.allocate_register();
            if tag.is_some() {
                // The active variant's record is inlined in the union block; the
                // value is its address (`UnionExtract`'s data arm).
                self.emit(abi::add_immediate(&value, &base, offset));
            } else {
                self.emit(abi::load_u64(&value, &base, offset));
                // An inlined record field holds a block-relative offset; the value
                // is the record base plus it (plan-02 §4.2).
                // Asked of the declared field type, `RES` marker included, exactly
                // as `lower_field_access` asks it: a concrete `RES` field holds the
                // record pointer in its slot, and a `RES <resource union>` field
                // holds the union's box inline (bug-1012), so its value is the
                // address of that box.
                if is_record && self.record_field_is_inlined(&part) {
                    self.emit(abi::add_registers(&value, &base, &value));
                }
            }
            let part_slot = self.allocate_stack_object("resource_walk_part", 8);
            self.emit(abi::store_u64(&value, abi::stack_pointer(), part_slot));
            self.emit_walk_value(part_slot, &part, visit, expanding, deferred)?;
            self.emit(abi::label(&skip));
        }
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Push the composite value in `slot` onto the run-time worklist.
    fn emit_defer(
        &mut self,
        slot: usize,
        type_: &ParameterType,
        deferred: &mut Option<Deferred>,
    ) -> Result<(), String> {
        if deferred.is_none() {
            let head_slot = self.allocate_stack_object("resource_walk_worklist", 8);
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), head_slot));
            *deferred = Some(Deferred {
                head_slot,
                kinds: Vec::new(),
            });
        }
        let state = deferred.as_mut().expect("worklist initialized above");
        let kind = match state.kinds.iter().position(|kind| kind == type_) {
            Some(kind) => kind,
            None => {
                state.kinds.push(type_.clone());
                state.kinds.len() - 1
            }
        };
        let head_slot = state.head_slot;
        let skip = self.label("resource_walk_defer_null");
        let value = self.allocate_register();
        self.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&value, "0"));
        self.emit(abi::branch_eq(&skip));
        let alloc_ok = self.label("resource_walk_defer_alloc_ok");
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &WORKLIST_NODE_SIZE.to_string(),
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        let node = self.allocate_register();
        self.emit(abi::move_register(&node, abi::mfb_return(1)));
        let value = self.allocate_register();
        self.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
        self.emit(abi::store_u64(&value, &node, 0));
        let kind_reg = self.allocate_register();
        self.emit(abi::move_immediate(&kind_reg, "Integer", &kind.to_string()));
        self.emit(abi::store_u64(&kind_reg, &node, 8));
        let head = self.allocate_register();
        self.emit(abi::load_u64(&head, abi::stack_pointer(), head_slot));
        self.emit(abi::store_u64(&head, &node, 16));
        self.emit(abi::store_u64(&node, abi::stack_pointer(), head_slot));
        self.emit(abi::label(&skip));
        Ok(())
    }

    /// Pop and expand every deferred value until the worklist is empty. A kind
    /// discovered while expanding another gets its own expansion.
    fn emit_deferred_walks(
        &mut self,
        mut deferred: Deferred,
        visit: &mut ResourceVisit<'_>,
    ) -> Result<(), String> {
        let head_slot = deferred.head_slot;
        let value_slot = self.allocate_stack_object("resource_walk_deferred_value", 8);
        let kind_slot = self.allocate_stack_object("resource_walk_deferred_kind", 8);
        let top = self.label("resource_walk_deferred_loop");
        let dispatch = self.label("resource_walk_deferred_dispatch");
        let done = self.label("resource_walk_deferred_done");
        self.emit(abi::label(&top));
        let node = self.allocate_register();
        self.emit(abi::load_u64(&node, abi::stack_pointer(), head_slot));
        self.emit(abi::compare_immediate(&node, "0"));
        self.emit(abi::branch_eq(&done));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, &node, 0));
        self.emit(abi::store_u64(&field, abi::stack_pointer(), value_slot));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, &node, 8));
        self.emit(abi::store_u64(&field, abi::stack_pointer(), kind_slot));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, &node, 16));
        self.emit(abi::store_u64(&field, abi::stack_pointer(), head_slot));
        self.emit(abi::move_register(abi::c_arg(0), &node));
        self.emit(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            &WORKLIST_NODE_SIZE.to_string(),
        ));
        self.emit_arena_free_call();
        self.emit(abi::branch(&dispatch));
        let mut bodies = Vec::new();
        let mut index = 0;
        while index < deferred.kinds.len() {
            let kind = deferred.kinds[index].clone();
            let body = self.label("resource_walk_deferred_kind");
            self.emit(abi::label(&body));
            let mut expanding = vec![kind.clone()];
            let mut nested = Some(deferred);
            self.emit_walk_composite(value_slot, &kind, visit, &mut expanding, &mut nested)?;
            deferred = nested.expect("the worklist stays initialized");
            self.emit(abi::branch(&top));
            bodies.push(body);
            index += 1;
        }
        self.emit(abi::label(&dispatch));
        for (kind, body) in bodies.iter().enumerate() {
            let current = self.allocate_register();
            self.emit(abi::load_u64(&current, abi::stack_pointer(), kind_slot));
            let expected = self.allocate_register();
            self.emit(abi::move_immediate(&expected, "Integer", &kind.to_string()));
            self.emit(abi::compare_registers(&current, &expected));
            self.emit(abi::branch_eq(body));
        }
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
        Ok(())
    }
}
