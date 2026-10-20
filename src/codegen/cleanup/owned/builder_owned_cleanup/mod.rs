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

mod drain;
mod owned_list;
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
            builder.emit_branch_if_takeable(owner, token, &take);
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
