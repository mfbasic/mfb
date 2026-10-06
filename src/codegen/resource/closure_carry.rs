//! bug-843: a closure carries resource references, exactly as a `List` does.
//!
//! **The rule (owner decision).** Capturing a resource — directly, or through a value
//! that reaches one — copies a reference into the closure's environment, the same way
//! `collections::append` copies one into a `List` (§15.6). The closure never owns the
//! resource. When the closure is held by a longer-lived scope the resource's cleanup
//! scope rises to it, and the resource is closed once at the end of its own (possibly
//! raised) scope. Every movement of a closure — a copy, a `RETURN`, a discarded call
//! result, a store into a module-level binding, an element of a `List OF FUNC` —
//! follows what the `List` rule already does for the same movement.
//!
//! **Inside one function** the routing is static, as it is for a `List`: the escape
//! analysis treats a `LAMBDA` as a container whose elements are its captures
//! (`ir::resource_escape`), so a resource captured by a closure stored in an outer
//! binding floats to that binding's scope and is listed on its owned-list.
//!
//! **Across a function boundary** a `List` is walked by its TYPE
//! (`resource::walk`): a `RETURN` releases what the frame owns, the caller's binding
//! adopts, a discarded result is closed, a store into a global raises to the module.
//! A `FUNC` type says nothing about what its closure captured, so a function value is
//! a *closure leaf* of that walk and the walk asks the closure itself: every capturing
//! `LAMBDA`'s environment routine (`closure_ops`, bug-781) gains four resource
//! operations — `RELEASE`, `CLOSE_UNOWNED`, `MODULE_FLOAT`, `ADOPT` — each running the
//! same per-resource step the type walk runs, over the captures it knows the types of.
//!
//! **The dynamic owned-list.** A binding that holds function values (a `FUNC`, a
//! `List OF FUNC`, a record with a `FUNC` field) can come to own resources of any kind
//! a closure in the module carries, which its type cannot name. Such a binding gets
//! one owned-list whose nodes record their own kind: `{record@0, next@8, kind@16,
//! tag@24}`, `kind` indexing [`CodeBuilder::closure_resource_kinds`] and `tag` the
//! active variant of a resource-union leaf (a node holds the variant's RECORD, never
//! the union box, which its wrapping binding frees — the module list's rule). The
//! drain closes each node's record when the scope still owns it and reclaims its
//! buffers and `STATE`; it never frees the record itself (the tombstone every view
//! reads), so a record listed twice is closed and reclaimed idempotently.
//!
//! **Cost.** All of this keys on [`CodeBuilder::closures_carry_resources`]: a module
//! none of whose `LAMBDA`s captures a resource-reaching value lowers exactly as it did
//! before, environment routines included.
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::typed_strip_res_marker;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{NirModule, NirOp, NirValue};
use crate::types::ParameterType;
use std::collections::HashMap;

/// Bytes in one dynamic owned-list node: `{record@0, next@8, kind@16, tag@24}`.
pub(crate) const DYNAMIC_NODE_SIZE: usize = 32;

/// The type of every by-value `LAMBDA` capture in `module` that can reach a resource,
/// in first-seen order. Read from both ends of a capture — the creating function's
/// `Closure` (a captured local, typed by its binding) and the lifted body's `Capture`
/// reads (which carry the type) — so a binding whose type the creating side cannot
/// name is still seen through the body that reads it.
pub(crate) fn closure_capture_types(model: &TypeModel, module: &NirModule) -> Vec<ParameterType> {
    struct Scan<'m> {
        model: &'m TypeModel,
        types: HashMap<String, ParameterType>,
        out: Vec<ParameterType>,
    }
    impl Scan<'_> {
        fn note(&mut self, type_: &ParameterType) {
            if crate::codegen::resource::module_owner::model_type_reaches_resource(
                self.model, type_,
            ) && !self.out.contains(type_)
            {
                self.out.push(type_.clone());
            }
        }
    }
    impl NirVisitor for Scan<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { name, type_, .. }
                | NirOp::For { name, type_, .. }
                | NirOp::ForEach { name, type_, .. } => {
                    self.types.insert(name.clone(), type_.clone());
                }
                _ => {}
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Closure { captures, .. } => {
                    for capture in captures {
                        if let NirValue::Local(name) = capture {
                            if let Some(type_) = self.types.get(name).cloned() {
                                self.note(&type_);
                            }
                        }
                    }
                }
                NirValue::Capture {
                    type_,
                    by_ref: false,
                    ..
                } => self.note(type_),
                _ => {}
            }
            walk_value(self, value);
        }
    }
    let mut scan = Scan {
        model,
        types: HashMap::new(),
        out: Vec::new(),
    };
    for function in &module.functions {
        scan.types = function
            .params
            .iter()
            .map(|param| (param.name.clone(), param.type_.clone()))
            .collect();
        scan.visit_ops(&function.body);
    }
    scan.out
}

/// A dynamic owned-list a binding registered: its head slot and its scope depth.
#[derive(Clone, Copy)]
pub(crate) struct DynamicList {
    pub(crate) head_slot: usize,
    pub(crate) scope_depth: usize,
}

impl CodeBuilder<'_> {
    /// The resource leaf kinds some closure in this module can carry, in first-seen
    /// order: the `kind` numbering of a dynamic owned-list node.
    pub(crate) fn closure_resource_kinds(&self) -> Vec<ParameterType> {
        let mut kinds: Vec<ParameterType> = Vec::new();
        for type_ in &self.type_model.closure_capture_types {
            for leaf in self.resource_leaf_types(type_) {
                if !kinds.contains(&leaf) {
                    kinds.push(leaf);
                }
            }
        }
        kinds
    }

    /// Whether a function value in this module can carry a resource reference: some
    /// `LAMBDA` captures a value that reaches a resource.
    pub(crate) fn closures_carry_resources(&self) -> bool {
        self.type_model
            .closure_capture_types
            .iter()
            .any(|type_| !self.resource_leaf_types(type_).is_empty())
    }

    /// Whether `type_` is a closure leaf of the resource walk: a function value, in a
    /// module whose closures can carry resources.
    pub(crate) fn is_closure_leaf(&self, type_: &ParameterType) -> bool {
        matches!(typed_strip_res_marker(type_), ParameterType::Func(..))
            && self.closures_carry_resources()
    }

    /// The `kind` a resource of type `leaf` is listed under on a dynamic owned-list:
    /// its own kind, else the one kind sharing its base resource.
    fn closure_kind_index(&self, leaf: &ParameterType) -> Result<usize, String> {
        let leaf = typed_strip_res_marker(leaf);
        let kinds = self.closure_resource_kinds();
        if let Some(kind) = kinds.iter().position(|kind| kind == leaf) {
            return Ok(kind);
        }
        let mut same_base = kinds
            .iter()
            .enumerate()
            .filter(|(_, kind)| kind.without_state() == leaf.without_state());
        match (same_base.next(), same_base.next()) {
            (Some((kind, _)), None) => Ok(kind),
            _ => Err(format!(
                "a resource of type '{leaf}' floats to a holder of function values, but no \
                 closure in this module carries that resource (bug-843)"
            )),
        }
    }

    /// The dynamic owned-list `collection` registered, innermost first.
    pub(crate) fn dynamic_owned_list(&self, collection: &str) -> Option<DynamicList> {
        self.active_cleanups
            .iter()
            .rev()
            .find_map(|cleanup| match cleanup {
                ActiveCleanup::OwnedList(list)
                    if list.name == collection && matches!(list.drop, OwnedListDrop::Dynamic) =>
                {
                    Some(DynamicList {
                        head_slot: list.head_slot,
                        scope_depth: list.scope_depth,
                    })
                }
                _ => None,
            })
    }

    /// Register `name`'s dynamic owned-list (see the module doc): an empty head, and
    /// an [`ActiveCleanup::OwnedList`] drained on every exit path of this scope.
    pub(crate) fn setup_dynamic_owned_list(&mut self, name: &str) {
        let head_slot = self.allocate_stack_object(&format!("owned_dynamic_{name}"), 8);
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), head_slot));
        self.active_cleanups
            .push(ActiveCleanup::OwnedList(OwnedListCleanup {
                name: name.to_string(),
                head_slot,
                drop: OwnedListDrop::Dynamic,
                owns_elements: false,
                scope_depth: self.current_scope_depth(),
                leaf: ParameterType::Unknown,
            }));
    }

    /// Call the environment routine of the function value in `slot` with resource
    /// operation `op` (a `CLOSURE_ENV_OP_*` past copy and drop), passing the words in
    /// `args` (stack slots) as arguments 2 and 3. A null value and a static descriptor
    /// (`env = 0`, which captured nothing) are skipped.
    pub(crate) fn emit_closure_resource_op(
        &mut self,
        slot: usize,
        op: usize,
        args: &[usize],
    ) -> Result<(), String> {
        if args.len() > 2 {
            return Err(format!(
                "closure resource operation {op} takes at most two arguments, got {}",
                args.len()
            ));
        }
        let sp = abi::stack_pointer();
        let done = self.label("closure_resource_op_done");
        let object = self.allocate_register();
        self.emit(abi::load_u64(&object, sp, slot));
        self.emit(abi::compare_immediate(&object, "0"));
        self.emit(abi::branch_eq(&done));
        let env = self.allocate_register();
        self.emit(abi::load_u64(&env, &object, CLOSURE_OFFSET_ENV));
        self.emit(abi::compare_immediate(&env, "0"));
        self.emit(abi::branch_eq(&done));
        let env_slot = self.spill_to_slot("closure_resource_op_env", &env);
        self.emit(abi::move_immediate(
            abi::c_arg(0),
            "Integer",
            &op.to_string(),
        ));
        self.emit(abi::load_u64(abi::c_arg(1), sp, env_slot));
        for (index, arg_slot) in args.iter().enumerate() {
            self.emit(abi::load_u64(abi::c_arg(2 + index), sp, *arg_slot));
        }
        let routine = self.allocate_register();
        self.emit(abi::load_u64(&routine, sp, env_slot));
        self.emit(abi::load_u64(&routine, &routine, CLOSURE_ENV_OFFSET_OPS));
        self.emit(abi::branch_link_register(&routine));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// A fresh copy of the 16-byte resource-union box in `slot` (`{tag@0,
    /// record@8}`), or `0` for a null box: the closure environment's own copy
    /// (`ClosureEnvWord::ResourceUnionBox`).
    pub(crate) fn emit_copy_resource_union_box(
        &mut self,
        slot: usize,
    ) -> Result<crate::codegen::engine::operand::VirtualRegister, String> {
        let sp = abi::stack_pointer();
        let result_slot = self.allocate_stack_object("union_box_copy", 8);
        let absent = self.label("union_box_copy_absent");
        let alloc_ok = self.label("union_box_copy_alloc_ok");
        self.emit(abi::store_u64(abi::ZERO, sp, result_slot));
        let source = self.allocate_register();
        self.emit(abi::load_u64(&source, sp, slot));
        self.emit(abi::compare_immediate(&source, "0"));
        self.emit(abi::branch_eq(&absent));
        self.emit(abi::move_immediate(abi::return_register(), "Integer", "16"));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        let copy = self.allocate_register();
        self.emit(abi::move_register(&copy, abi::mfb_return(1)));
        self.emit(abi::store_u64(&copy, sp, result_slot));
        let source = self.allocate_register();
        self.emit(abi::load_u64(&source, sp, slot));
        for offset in [0, 8] {
            let word = self.allocate_register();
            self.emit(abi::load_u64(&word, &source, offset));
            self.emit(abi::store_u64(&word, &copy, offset));
        }
        self.emit(abi::label(&absent));
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, sp, result_slot));
        Ok(result)
    }

    /// The record of the leaf value in `slot` (the value itself for a concrete
    /// resource, the active variant's record for a resource-union box) and that
    /// variant's tag (`0` for a concrete resource), stored into fresh slots. Branches
    /// to `skip` for a null value or variant record.
    fn emit_leaf_record_and_tag(
        &mut self,
        slot: usize,
        leaf: &ParameterType,
        skip: &str,
    ) -> (usize, usize) {
        let sp = abi::stack_pointer();
        let record_slot = self.allocate_stack_object("closure_carry_record", 8);
        let tag_slot = self.allocate_stack_object("closure_carry_tag", 8);
        let value = self.allocate_register();
        self.emit(abi::load_u64(&value, sp, slot));
        self.emit(abi::compare_immediate(&value, "0"));
        self.emit(abi::branch_eq(skip));
        if self.resource_union_cleanup(leaf).is_some() {
            let tag = self.allocate_register();
            self.emit(abi::load_u64(&tag, &value, 0));
            self.emit(abi::store_u64(&tag, sp, tag_slot));
            let record = self.allocate_register();
            self.emit(abi::load_u64(&record, &value, 8));
            self.emit(abi::compare_immediate(&record, "0"));
            self.emit(abi::branch_eq(skip));
            self.emit(abi::store_u64(&record, sp, record_slot));
        } else {
            self.emit(abi::store_u64(abi::ZERO, sp, tag_slot));
            self.emit(abi::store_u64(&value, sp, record_slot));
        }
        (record_slot, tag_slot)
    }

    /// List the resource leaf in `slot` on the dynamic owned-list whose head word is
    /// at the address in `head_address_slot`, after floating it to the scope whose
    /// token is in `token_slot`.
    ///
    /// With `take_only` (an adoption, `ADOPT`) the node is pushed only when that scope
    /// TAKES the record — it was unowned or owned by a scope inside it — so one
    /// record is listed once however often a closure carrying it is adopted. Without
    /// it (a static float, which `ir::resource_escape` decided) the node is always
    /// pushed, exactly as a `List`'s owned-list push is.
    pub(crate) fn emit_dynamic_owned_push(
        &mut self,
        slot: usize,
        leaf: &ParameterType,
        head_address_slot: usize,
        token_slot: usize,
        take_only: bool,
    ) -> Result<(), String> {
        let leaf = typed_strip_res_marker(leaf).clone();
        let kind = self.closure_kind_index(&leaf)?;
        let sp = abi::stack_pointer();
        let done = self.label("dynamic_owned_push_done");
        let take = self.label("dynamic_owned_push_take");
        let push = self.label("dynamic_owned_push_node");
        let (record_slot, tag_slot) = self.emit_leaf_record_and_tag(slot, &leaf, &done);
        let record = self.allocate_register();
        self.emit(abi::load_u64(&record, sp, record_slot));
        let owner = self.allocate_register();
        self.emit(abi::load_u64(&owner, &record, RESOURCE_OFFSET_OWNER));
        let token = self.allocate_register();
        self.emit(abi::load_u64(&token, sp, token_slot));
        // Unsigned: an unowned record (`0`) is below every token, so it is taken too;
        // one owned here or further out keeps its owner (ownership only floats up).
        self.emit(abi::compare_registers(&owner, &token));
        self.emit(abi::branch_lo(&take));
        self.emit(abi::branch(if take_only { &done } else { &push }));
        self.emit(abi::label(&take));
        self.emit(abi::store_u64(&token, &record, RESOURCE_OFFSET_OWNER));
        self.emit(abi::label(&push));
        let alloc_ok = self.label("dynamic_owned_push_alloc_ok");
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &DYNAMIC_NODE_SIZE.to_string(),
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        let node = self.allocate_register();
        self.emit(abi::move_register(&node, abi::mfb_return(1)));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, sp, record_slot));
        self.emit(abi::store_u64(&field, &node, 0));
        let kind_reg = self.allocate_register();
        self.emit(abi::move_immediate(&kind_reg, "Integer", &kind.to_string()));
        self.emit(abi::store_u64(&kind_reg, &node, 16));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, sp, tag_slot));
        self.emit(abi::store_u64(&field, &node, 24));
        let head_address = self.allocate_register();
        self.emit(abi::load_u64(&head_address, sp, head_address_slot));
        let head = self.allocate_register();
        self.emit(abi::load_u64(&head, &head_address, 0));
        self.emit(abi::store_u64(&head, &node, 8));
        self.emit(abi::store_u64(&node, &head_address, 0));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// The address of this frame's `head_slot` and the token of the scope at `depth`,
    /// each in a fresh slot: the two arguments a dynamic push and an `ADOPT` take.
    pub(crate) fn emit_dynamic_list_arguments(
        &mut self,
        list: DynamicList,
    ) -> Result<(usize, usize), String> {
        let address = self.allocate_register();
        self.emit(abi::add_immediate(
            &address,
            abi::stack_pointer(),
            list.head_slot,
        ));
        let address_slot = self.spill_to_slot("dynamic_owned_head_address", &address);
        let token = self.emit_scope_token(list.scope_depth)?;
        let token_slot = self.spill_to_slot("dynamic_owned_token", &token);
        Ok((address_slot, token_slot))
    }

    /// Drain a dynamic owned-list (see the module doc): pop and free each node, and
    /// close its record by its kind when this list's scope still owns it, reclaiming
    /// the record's buffers and `STATE` behind the close but never the record.
    pub(crate) fn emit_dynamic_owned_list_drain(
        &mut self,
        cleanup: &OwnedListCleanup,
    ) -> Result<(), String> {
        let sp = abi::stack_pointer();
        let kinds = self.closure_resource_kinds();
        let record_slot = self.allocate_stack_object("dynamic_drain_record", 8);
        let kind_slot = self.allocate_stack_object("dynamic_drain_kind", 8);
        let tag_slot = self.allocate_stack_object("dynamic_drain_tag", 8);
        let top = self.label("dynamic_drain_loop");
        let done = self.label("dynamic_drain_done");
        self.emit(abi::label(&top));
        let node = self.allocate_register();
        self.emit(abi::load_u64(&node, sp, cleanup.head_slot));
        self.emit(abi::compare_immediate(&node, "0"));
        self.emit(abi::branch_eq(&done));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, &node, 8));
        self.emit(abi::store_u64(&field, sp, cleanup.head_slot));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, &node, 0));
        self.emit(abi::store_u64(&field, sp, record_slot));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, &node, 16));
        self.emit(abi::store_u64(&field, sp, kind_slot));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, &node, 24));
        self.emit(abi::store_u64(&field, sp, tag_slot));
        self.emit(abi::move_register(abi::c_arg(0), &node));
        self.emit(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            &DYNAMIC_NODE_SIZE.to_string(),
        ));
        self.emit_arena_free_call();
        for (kind, leaf) in kinds.iter().enumerate() {
            let next_kind = self.label("dynamic_drain_next_kind");
            let current = self.allocate_register();
            self.emit(abi::load_u64(&current, sp, kind_slot));
            self.emit(abi::compare_immediate(&current, &kind.to_string()));
            self.emit(abi::branch_ne(&next_kind));
            // bug-755: close only a record this list's scope still owns.
            let record = self.allocate_register();
            self.emit(abi::load_u64(&record, sp, record_slot));
            self.emit(abi::compare_immediate(&record, "0"));
            self.emit(abi::branch_eq(&top));
            self.emit_skip_unless_owned(&Reg::from(record), cleanup.scope_depth, &top)?;
            self.emit_listed_record_close(record_slot, tag_slot, leaf, true, &top)?;
            self.emit(abi::branch(&top));
            self.emit(abi::label(&next_kind));
        }
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Close the record in `record_slot` by `leaf`'s close op — for a resource-union
    /// leaf, the close op of the variant whose tag is in `tag_slot` — then branch to
    /// `next`. A close of an already-closed or moved handle is the defined no-op;
    /// any other failure is recorded like every drop-close failure. With `reclaim`, the
    /// record's buffers and `STATE` are reclaimed behind the close, never the record
    /// itself. The caller has already decided this record is to be closed.
    pub(crate) fn emit_listed_record_close(
        &mut self,
        record_slot: usize,
        tag_slot: usize,
        leaf: &ParameterType,
        reclaim: bool,
        next: &str,
    ) -> Result<(), String> {
        let closes: Vec<(Option<usize>, String)> = match self.resource_union_cleanup(leaf) {
            Some(variants) => variants
                .into_iter()
                .map(|(tag, close)| (Some(tag), close))
                .collect(),
            None => vec![(
                None,
                self.resource_cleanup_symbol(leaf).ok_or_else(|| {
                    format!("listed resource type '{leaf}' has no registered close op")
                })?,
            )],
        };
        let union = self.resource_union_cleanup(leaf).is_some();
        for (tag, close) in closes {
            let other = self.label("listed_close_other_variant");
            let closed = self.label("listed_close_closed");
            if let Some(tag) = tag {
                let active = self.allocate_register();
                self.emit(abi::load_u64(&active, abi::stack_pointer(), tag_slot));
                let expected = self.allocate_register();
                self.emit(abi::move_immediate(&expected, "Integer", &tag.to_string()));
                self.emit(abi::compare_registers(&active, &expected));
                self.emit(abi::branch_ne(&other));
            }
            let record = self.allocate_register();
            self.emit(abi::load_u64(&record, abi::stack_pointer(), record_slot));
            self.emit(abi::move_register(abi::return_register(), &record));
            self.emit_symbol_call(&close);
            self.emit_drop_close_outcome(&closed);
            self.emit(abi::label(&closed));
            if reclaim {
                if union {
                    if let Some(state_type) = leaf.state() {
                        self.emit_free_resource_state_block(record_slot, &state_type)?;
                    }
                } else {
                    self.emit_resource_block_reclaim(
                        record_slot,
                        leaf.state().as_ref(),
                        Self::resource_uses_io_buffers(leaf),
                        false,
                    )?;
                }
            }
            self.emit(abi::branch(next));
            self.emit(abi::label(&other));
        }
        Ok(())
    }

    /// Emit, inside an environment routine, the body of resource operation `op` over
    /// the `resource_words` of its layout: the environment pointer is in `env_slot`
    /// and the operation's arguments in `arg_slots`.
    pub(crate) fn emit_closure_env_resource_op(
        &mut self,
        op: usize,
        env_slot: usize,
        arg_slots: &[usize],
        resource_words: &[(usize, ParameterType)],
    ) -> Result<(), String> {
        let sp = abi::stack_pointer();
        for (index, type_) in resource_words {
            let capture_slot = self.allocate_stack_object("closure_env_resource", 8);
            let source = self.allocate_register();
            self.emit(abi::load_u64(&source, sp, env_slot));
            self.emit(abi::load_u64(
                &source,
                &source,
                closure_env_word_offset(*index),
            ));
            self.emit(abi::store_u64(&source, sp, capture_slot));
            let arg_slots = arg_slots.to_vec();
            self.emit_for_each_resource(capture_slot, type_, &mut |builder, leaf_slot, leaf| {
                builder.emit_resource_op_leaf(op, leaf_slot, leaf, &arg_slots)
            })?;
        }
        Ok(())
    }

    /// One resource leaf's step of operation `op`, inside an environment routine. A
    /// closure leaf (a captured function value) forwards the same operation to its
    /// own routine.
    fn emit_resource_op_leaf(
        &mut self,
        op: usize,
        leaf_slot: usize,
        leaf: &ParameterType,
        arg_slots: &[usize],
    ) -> Result<(), String> {
        if self.is_closure_leaf(leaf) {
            return self.emit_closure_resource_op(leaf_slot, op, arg_slots);
        }
        match op {
            CLOSURE_ENV_OP_RELEASE => self.emit_leaf_release_against(leaf_slot, leaf, arg_slots[0]),
            CLOSURE_ENV_OP_CLOSE_UNOWNED => self.emit_close_unowned_at(leaf_slot, leaf),
            CLOSURE_ENV_OP_MODULE_FLOAT => self.emit_module_float(leaf_slot, leaf),
            CLOSURE_ENV_OP_ADOPT => {
                self.emit_dynamic_owned_push(leaf_slot, leaf, arg_slots[0], arg_slots[1], true)
            }
            CLOSURE_ENV_OP_CLAIM => self.emit_leaf_claim_against(leaf_slot, leaf, arg_slots[0]),
            other => Err(format!("unknown closure resource operation {other}")),
        }
    }
}
