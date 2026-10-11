//! bug-806: the module instance as a resource owner.
//!
//! A module-level binding (a global) is the outermost scope there is: per
//! instance — the program's main thread, or each worker thread's own fresh
//! instance of the project (§13, bug-369) — it lives until the instance tears
//! down. A resource stored into one therefore floats to that scope, exactly as a
//! resource appended to an outer collection floats to the collection's scope
//! (§15.6), and is closed once at instance teardown.
//!
//! **The owner token.** A frame scope's token is `((sp + slot) << 12) - depth`
//! (`resource::owner`) with `depth` in `1..4096`, so it is below `2^59` on every
//! target (a user stack address is below `2^47`). [`MODULE_OWNER_TOKEN`] is all
//! ones: a frame token equals it only if `(sp + slot) << 12 == depth - 1`, which
//! needs `depth == 1` and a stack address of `0` (mod `2^52`) — so no frame token
//! can equal it. It is above every frame token (so a float — "raise to the larger token" — never
//! lowers a module-owned record into a frame scope), and a `RETURN`'s frame test
//! (`(owner >> 12) + 1 == sp + slot`) reads `2^52` for it, which is no stack
//! address. Every frame-scope close runs behind `emit_skip_unless_owned`, so a
//! function's cleanups skip a module-owned record; a discarded result closes only
//! an unowned one; a binding claims only an unowned one.
//!
//! **The instance's list.** One writable global slot, [`MODULE_RESOURCES_GLOBAL`],
//! heads a list of `{record@0, next@8, kind@16, tag@24}` arena nodes. A node holds
//! the resource RECORD, never a resource-union box: a box belongs to whoever made
//! it — a binding, which frees it at its scope exit, or the statement that built
//! it, which frees it at its end (bug-1012) — or is a span of the record or
//! collection that holds the value inline, whoever owns the record. So a union
//! leaf is listed as its active variant's record plus that variant's tag, read
//! from the box when it is listed. It lives in the
//! globals region, which is per arena — so each instance has its own list, and it
//! is zero (empty) at instance start like every global slot. A node's `kind`
//! indexes [`CodeBuilder::module_resource_leaves`], the resource leaf types every
//! global can reach, in global order, so the push site and the teardown agree on
//! it without sharing state.
//!
//! **Teardown.** [`lower_module_resource_drain`] closes every listed resource
//! still owned by the instance (an explicit close or a transfer already happened
//! leaves the closed/moved flag, and the close is then the defined no-op). It
//! reclaims nothing: the instance's arena is released with the instance. The
//! program entry calls it on its one exit path (normal return, `EXIT PROGRAM`
//! and an untrapped error all converge there); a worker's trampoline calls it
//! after the worker body returns; the canvas graphics thread after its render
//! loop.
//!
//! **Removing a view closes nothing.** The list holds the record, not the
//! global's current value, so replacing or emptying the global leaves the
//! obligation in place (§15.6: ownership only floats outward).
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::{is_unset_type, typed_strip_res_marker};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};

/// The owner word of a record owned by the module instance (see the module doc).
pub(crate) const MODULE_OWNER_TOKEN: u64 = u64::MAX;

/// The writable global slot heading the instance's module-owned resource list.
/// Not an MFBASIC identifier, so it cannot collide with a declared global.
pub(crate) const MODULE_RESOURCES_GLOBAL: &str = "$module_resources";

/// The instance-teardown routine that closes the module-owned resources.
pub(crate) const MODULE_RESOURCE_DRAIN_SYMBOL: &str = "_mfb_module_resource_drain";

/// Bytes in one module list node: `{record@0, next@8, kind@16, tag@24}`.
const MODULE_NODE_SIZE: usize = 32;

thread_local! {
    /// The leaf kinds some store of this module lists, collected while its
    /// functions lower (they lower on one thread, like `value_functions`). The
    /// teardown closes only these kinds: a kind no store lists never has a node,
    /// and its close op need not exist in the program — a module whose global can
    /// hold a `tcp::Socket` but that never opens one links no `tcp` close.
    static REQUESTED_KINDS: RefCell<BTreeSet<usize>> = const { RefCell::new(BTreeSet::new()) };
}

/// Forget the kinds a previous module's lowering listed.
pub(crate) fn reset_requested_kinds() {
    REQUESTED_KINDS.with(|kinds| kinds.borrow_mut().clear());
}

fn request_kind(kind: usize) {
    REQUESTED_KINDS.with(|kinds| {
        kinds.borrow_mut().insert(kind);
    });
}

fn requested_kinds() -> BTreeSet<usize> {
    REQUESTED_KINDS.with(|kinds| kinds.borrow().clone())
}

/// Whether a value of `type_` can reach a resource, from the type model alone —
/// what decides, before any function is lowered, whether the module reserves
/// [`MODULE_RESOURCES_GLOBAL`]. The same structural test the operand-snapshot
/// pass uses (`type_reaches_resource`); it may only over-approximate the
/// lowering's own leaf test, which costs one unused slot.
pub(crate) fn model_type_reaches_resource(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut stack = vec![type_.clone()];
    let mut visited = HashSet::new();
    while let Some(current) = stack.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if matches!(
            current,
            ParameterType::ThreadHandle { .. }
                | ParameterType::Res(_)
                | ParameterType::Stateful { .. }
        ) || crate::codegen::builtins::is_resource_type(&current)
            || model.resource_names.contains(&current.without_state())
        {
            return true;
        }
        // bug-843: a function value reaches whatever some closure in the module
        // captured (`closure_carry::closure_capture_types`, empty while that table
        // is itself being built).
        if matches!(current, ParameterType::Func(..)) && !model.closure_capture_types.is_empty() {
            return true;
        }
        stack.extend(
            crate::codegen::collection::layout::builder_collection_layout::type_components(
                model, &current,
            ),
        );
    }
    false
}

impl CodeBuilder<'_> {
    /// [`MODULE_OWNER_TOKEN`] into a fresh register. Built as `0 - 1`: an immediate
    /// operand is unsigned and a full 64-bit one is not encodable everywhere.
    fn emit_module_owner_token(&mut self) -> crate::codegen::engine::operand::VirtualRegister {
        const _: () = assert!(MODULE_OWNER_TOKEN == 0u64.wrapping_sub(1));
        let token = self.allocate_register();
        self.emit(abi::move_immediate(&token, "Integer", "0"));
        self.emit(abi::subtract_immediate(&token, &token, 1));
        token
    }

    /// Whether this module reserved the module-owned resource list.
    pub(crate) fn has_module_resources(&self) -> bool {
        self.globals.contains_key(MODULE_RESOURCES_GLOBAL)
    }

    /// Whether `name` is a module-level binding rather than a local.
    pub(crate) fn is_module_global(&self, name: &str) -> bool {
        !self.locals.contains_key(name) && self.globals.contains_key(name)
    }

    /// The resource leaf types every global can reach, in global-slot order then
    /// first-seen order: the `kind` numbering of a module list node.
    pub(crate) fn module_resource_leaves(&self) -> Vec<ParameterType> {
        let mut globals: Vec<(&String, &GlobalValue)> = self
            .globals
            .iter()
            .filter(|(name, _)| name.as_str() != MODULE_RESOURCES_GLOBAL)
            .collect();
        globals.sort_by_key(|(name, global)| (global.offset, (*name).clone()));
        let mut leaves: Vec<ParameterType> = Vec::new();
        for (_, global) in globals {
            let mut reached = self.resource_leaf_types(&global.type_);
            // bug-843: a global holding function values can hold whatever the
            // module's closures carry.
            if self.type_reaches_closure_leaf(&global.type_) {
                reached.extend(self.closure_resource_kinds());
            }
            for leaf in reached {
                if !leaves.contains(&leaf) {
                    leaves.push(leaf);
                }
            }
        }
        leaves
    }

    /// The `kind` a resource of type `leaf` is listed under: its own leaf, else
    /// the one leaf sharing its base resource (a `STATE` spelling the slot does
    /// not repeat).
    fn module_leaf_kind(&self, leaf: &ParameterType) -> Result<usize, String> {
        let leaf = typed_strip_res_marker(leaf);
        let leaves = self.module_resource_leaves();
        if let Some(kind) = leaves.iter().position(|kind| kind == leaf) {
            return Ok(kind);
        }
        let mut same_base = leaves
            .iter()
            .enumerate()
            .filter(|(_, kind)| kind.without_state() == leaf.without_state());
        match (same_base.next(), same_base.next()) {
            (Some((kind, _)), None) => Ok(kind),
            _ => Err(format!(
                "a resource of type '{leaf}' is stored into a module-level binding, but no \
                 module-level binding's type reaches that resource (bug-806)"
            )),
        }
    }

    /// Hand the resource leaf in `slot` (a record, or a resource-union box) to the
    /// module instance: unless it is already the instance's, its owner word becomes
    /// [`MODULE_OWNER_TOKEN`] and it is pushed onto the instance's list once. A
    /// null value or variant record is skipped.
    pub(crate) fn emit_module_float(
        &mut self,
        slot: usize,
        leaf: &ParameterType,
    ) -> Result<(), String> {
        let leaf = typed_strip_res_marker(leaf).clone();
        let kind = self.module_leaf_kind(&leaf)?;
        request_kind(kind);
        let union = self.resource_union_cleanup(&leaf).is_some();
        let done = self.label("module_float_done");
        let record_slot = self.allocate_stack_object("module_float_record", 8);
        let tag_slot = self.allocate_stack_object("module_float_tag", 8);
        let value = self.allocate_register();
        self.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&value, "0"));
        self.emit(abi::branch_eq(&done));
        let record = if union {
            let tag = self.allocate_register();
            self.emit(abi::load_u64(&tag, &value, 0));
            self.emit(abi::store_u64(&tag, abi::stack_pointer(), tag_slot));
            let record = self.allocate_register();
            self.emit(abi::load_u64(&record, &value, 8));
            self.emit(abi::compare_immediate(&record, "0"));
            self.emit(abi::branch_eq(&done));
            record
        } else {
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), tag_slot));
            value
        };
        self.emit(abi::store_u64(&record, abi::stack_pointer(), record_slot));
        let owner = self.allocate_register();
        self.emit(abi::load_u64(&owner, &record, RESOURCE_OFFSET_OWNER));
        let module = self.emit_module_owner_token();
        self.emit(abi::compare_registers(&owner, &module));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::store_u64(&module, &record, RESOURCE_OFFSET_OWNER));
        let alloc_ok = self.label("module_float_alloc_ok");
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &MODULE_NODE_SIZE.to_string(),
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        let node = self.allocate_register();
        self.emit(abi::move_register(&node, abi::mfb_return(1)));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, abi::stack_pointer(), record_slot));
        self.emit(abi::store_u64(&field, &node, 0));
        let kind_reg = self.allocate_register();
        self.emit(abi::move_immediate(&kind_reg, "Integer", &kind.to_string()));
        self.emit(abi::store_u64(&kind_reg, &node, 16));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, abi::stack_pointer(), tag_slot));
        self.emit(abi::store_u64(&field, &node, 24));
        let head_address = self.load_global_address(MODULE_RESOURCES_GLOBAL)?;
        let head = self.allocate_register();
        self.emit(abi::load_u64(&head, &head_address, 0));
        self.emit(abi::store_u64(&head, &node, 8));
        self.emit(abi::store_u64(&node, &head_address, 0));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// [`Self::emit_module_float`] for every resource the value of `type_` in
    /// `slot` reaches.
    pub(crate) fn emit_module_float_value(
        &mut self,
        slot: usize,
        type_: &ParameterType,
    ) -> Result<(), String> {
        self.emit_for_each_resource(slot, type_, &mut |builder, leaf_slot, leaf| {
            if builder.is_closure_leaf(leaf) {
                // bug-843: a stored closure raises what it carries, through its
                // environment routine.
                return builder.emit_closure_resource_op(
                    leaf_slot,
                    CLOSURE_ENV_OP_MODULE_FLOAT,
                    &[],
                );
            }
            builder.emit_module_float(leaf_slot, leaf)
        })
    }

    /// bug-843: whether some module-level binding can hold a function value that
    /// carries resources — the only way a closure's resources reach the module
    /// instance, and so the only case an environment routine's `MODULE_FLOAT` runs.
    pub(crate) fn module_globals_reach_closures(&self) -> bool {
        self.has_module_resources()
            && self
                .globals
                .iter()
                .filter(|(name, _)| name.as_str() != MODULE_RESOURCES_GLOBAL)
                .any(|(_, global)| self.type_reaches_closure_leaf(&global.type_))
    }

    /// Raise to the module instance every resource a store into global `name`
    /// adds, BEFORE the store, when the stored operands are named values; answers
    /// `true` when they are not (an operand is a call whose resources have no
    /// name), and the caller walks the whole stored global AFTER the store
    /// instead ([`Self::emit_module_float_global`]).
    ///
    /// The operand walk is what keeps a self-update `g = collections::append(g,
    /// item)` O(item) rather than O(len(g)): the global's own operand is skipped —
    /// whatever it holds was raised when it was stored.
    pub(crate) fn emit_module_store_float(
        &mut self,
        name: &str,
        type_: &ParameterType,
        value: Option<&NirValue>,
    ) -> Result<bool, String> {
        if !self.has_module_resources() {
            return Ok(false);
        }
        let global_type = if is_unset_type(type_) {
            self.global_value(name)?.type_
        } else {
            type_.clone()
        };
        if !self.type_carries_resources(&global_type) {
            return Ok(false);
        }
        // A store with no value writes the type's default, which holds no resource.
        let Some(value) = value else {
            return Ok(false);
        };
        let mut operands = Vec::new();
        if !self.named_store_operands(value, &mut operands) {
            return Ok(true);
        }
        for operand in operands {
            let result = self.lower_value(operand)?;
            if !self.type_carries_resources(&result.type_) {
                continue;
            }
            let location: Operand = result.location;
            let slot = self.spill_to_slot("module_store_operand", &location);
            self.emit_module_float_value(slot, &result.type_)?;
        }
        Ok(false)
    }

    /// Walk the whole current value of global `name` and raise every resource it
    /// reaches to the module instance (a record already the instance's is skipped).
    pub(crate) fn emit_module_float_global(&mut self, name: &str) -> Result<(), String> {
        let global = self.global_value(name)?;
        let address = self.load_global_address(name)?;
        let value = self.allocate_register();
        self.emit(abi::load_u64(&value, &address, 0));
        let slot = self.spill_to_slot("module_store_global", &value);
        self.emit_module_float_value(slot, &global.type_)
    }

    /// Collect the named, side-effect-free operands of a stored value that can
    /// carry a resource into it — a local, a field chain over a local, a capture,
    /// a union extract of one — through the constructors, literals, `WITH`, and
    /// `collections` insertions that place them. Answers `false` when some other
    /// operand may carry one (a call result): its resources have no name to walk
    /// before the store.
    fn named_store_operands<'v>(&self, value: &'v NirValue, out: &mut Vec<&'v NirValue>) -> bool {
        match value {
            NirValue::Const { .. } | NirValue::Global { .. } | NirValue::FunctionRef { .. } => true,
            NirValue::Local(name) => {
                if self
                    .locals
                    .get(name)
                    .is_none_or(|local| self.type_carries_resources(&local.type_))
                {
                    out.push(value);
                }
                true
            }
            NirValue::Capture { type_, .. } => {
                if self.type_carries_resources(type_) {
                    out.push(value);
                }
                true
            }
            NirValue::MemberAccess { .. } | NirValue::UnionExtract { .. } => {
                let mut root = value;
                loop {
                    match root {
                        NirValue::MemberAccess { target, .. }
                        | NirValue::UnionExtract { value: target, .. } => root = target,
                        _ => break,
                    }
                }
                match root {
                    // Whatever a global holds was raised when it was stored.
                    NirValue::Global { .. } => true,
                    NirValue::Local(_) | NirValue::Capture { .. } => {
                        out.push(value);
                        true
                    }
                    _ => self.operand_carries_no_resource(value),
                }
            }
            NirValue::Constructor { args, .. } => {
                args.iter().all(|arg| self.named_store_operands(arg, out))
            }
            NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => values
                .iter()
                .all(|item| self.named_store_operands(item, out)),
            NirValue::MapLiteral { entries, .. } => entries.iter().all(|(key, entry)| {
                self.named_store_operands(key, out) && self.named_store_operands(entry, out)
            }),
            NirValue::UnionWrap { value, .. } => self.named_store_operands(value, out),
            NirValue::WithUpdate {
                target, updates, ..
            } => {
                self.named_store_operands(target, out)
                    && updates
                        .iter()
                        .all(|update| self.named_store_operands(&update.value, out))
            }
            NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. }
                if matches!(
                    crate::codegen::builtins::native_builtin_target(target),
                    Some("append" | "prepend" | "insert" | "set")
                ) && target.starts_with("collections.") =>
            {
                args.iter().all(|arg| self.named_store_operands(arg, out))
            }
            _ => self.operand_carries_no_resource(value),
        }
    }

    /// Whether `value`'s static type is known and reaches no resource.
    fn operand_carries_no_resource(&self, value: &NirValue) -> bool {
        self.static_type_name(value)
            .is_some_and(|type_| !self.type_carries_resources(&type_))
    }
}

/// The module instance's teardown: close every resource on the instance's
/// module-owned list (see the module doc). Clobbers the caller-saved registers
/// like any lowered function and preserves the callee-saved ones, the arena-state
/// register included; its callers park whatever they need across it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_module_resource_drain(
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &crate::target::shared::nir::NirFunction>,
    package_return_types: &HashMap<String, ParameterType>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn crate::codegen::engine::types::CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<crate::codegen::engine::types::CodeFunction, String> {
    let mut builder = crate::codegen::engine::function::helper_code_builder(
        MODULE_RESOURCE_DRAIN_SYMBOL,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    );
    builder.emit_module_resource_drain_body()?;
    builder.emit(abi::return_());
    crate::codegen::engine::function::finish_helper_function(
        builder,
        "runtime.module.resource_drain",
        Vec::new(),
        "Nothing",
    )
}

impl CodeBuilder<'_> {
    /// The drain loop: pop each node, free it, and close its resource by its
    /// kind's close op (a resource union by tag dispatch), when the record is
    /// still the instance's.
    fn emit_module_resource_drain_body(&mut self) -> Result<(), String> {
        let leaves = self.module_resource_leaves();
        let value_slot = self.allocate_stack_object("module_drain_value", 8);
        let kind_slot = self.allocate_stack_object("module_drain_kind", 8);
        let tag_slot = self.allocate_stack_object("module_drain_tag", 8);
        let top = self.label("module_drain_loop");
        let done = self.label("module_drain_done");
        self.emit(abi::label(&top));
        let head_address = self.load_global_address(MODULE_RESOURCES_GLOBAL)?;
        let node = self.allocate_register();
        self.emit(abi::load_u64(&node, &head_address, 0));
        self.emit(abi::compare_immediate(&node, "0"));
        self.emit(abi::branch_eq(&done));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, &node, 8));
        self.emit(abi::store_u64(&field, &head_address, 0));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, &node, 0));
        self.emit(abi::store_u64(&field, abi::stack_pointer(), value_slot));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, &node, 16));
        self.emit(abi::store_u64(&field, abi::stack_pointer(), kind_slot));
        let field = self.allocate_register();
        self.emit(abi::load_u64(&field, &node, 24));
        self.emit(abi::store_u64(&field, abi::stack_pointer(), tag_slot));
        self.emit(abi::move_register(abi::c_arg(0), &node));
        self.emit(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            &MODULE_NODE_SIZE.to_string(),
        ));
        self.emit_arena_free_call();
        let requested = requested_kinds();
        for (kind, leaf) in leaves
            .iter()
            .enumerate()
            .filter(|(kind, _)| requested.contains(kind))
        {
            let next_kind = self.label("module_drain_next_kind");
            let current = self.allocate_register();
            self.emit(abi::load_u64(&current, abi::stack_pointer(), kind_slot));
            let expected = self.allocate_register();
            self.emit(abi::move_immediate(&expected, "Integer", &kind.to_string()));
            self.emit(abi::compare_registers(&current, &expected));
            self.emit(abi::branch_ne(&next_kind));
            self.emit_module_drain_close(value_slot, tag_slot, leaf, &top)?;
            self.emit(abi::branch(&top));
            self.emit(abi::label(&next_kind));
        }
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Close the record in `record_slot` when the instance still owns it, then
    /// branch to `next`: by the leaf's close op, or — for a resource-union leaf — by
    /// the close op of the variant whose tag the node recorded. A close of an
    /// already-closed handle is the defined no-op; any other failure is recorded
    /// like every drop-close failure.
    fn emit_module_drain_close(
        &mut self,
        record_slot: usize,
        tag_slot: usize,
        leaf: &ParameterType,
        next: &str,
    ) -> Result<(), String> {
        let record = self.allocate_register();
        self.emit(abi::load_u64(&record, abi::stack_pointer(), record_slot));
        self.emit(abi::compare_immediate(&record, "0"));
        self.emit(abi::branch_eq(next));
        let owner = self.allocate_register();
        self.emit(abi::load_u64(&owner, &record, RESOURCE_OFFSET_OWNER));
        let module = self.emit_module_owner_token();
        self.emit(abi::compare_registers(&owner, &module));
        self.emit(abi::branch_ne(next));
        let closes: Vec<(Option<usize>, String)> = match self.resource_union_cleanup(leaf) {
            Some(variants) => variants
                .into_iter()
                .map(|(tag, close)| (Some(tag), close))
                .collect(),
            None => vec![(
                None,
                self.resource_cleanup_symbol(leaf).ok_or_else(|| {
                    format!("module-owned resource type '{leaf}' has no registered close op")
                })?,
            )],
        };
        for (tag, close) in closes {
            let other = self.label("module_drain_other_variant");
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
            self.emit_drop_close_outcome(next);
            self.emit(abi::branch(next));
            self.emit(abi::label(&other));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reservation predicate sees a resource behind any aggregate layer, and
    /// nothing in plain data.
    #[test]
    fn the_reservation_predicate_sees_nested_resources_only() {
        let mut model = TypeModel::empty();
        model.record_fields.insert(
            ParameterType::declared("Holder"),
            vec![("handle".to_string(), ParameterType::parse("RES fs.File"))],
        );
        model.record_fields.insert(
            ParameterType::declared("Plain"),
            vec![("n".to_string(), ParameterType::Integer)],
        );
        assert!(model_type_reaches_resource(
            &model,
            &ParameterType::parse("List OF List OF Holder")
        ));
        assert!(!model_type_reaches_resource(
            &model,
            &ParameterType::parse("List OF Plain")
        ));
        assert!(!model_type_reaches_resource(
            &model,
            &ParameterType::Integer
        ));
    }

    /// The module token is no frame token: `(addr << 12) - depth == token` needs
    /// `addr << 12 == token + depth` (wrapping), a multiple of 4096 only for
    /// `depth == 1`, where it is `0` — a null stack address.
    #[test]
    fn the_module_token_is_no_frame_token() {
        let module = MODULE_OWNER_TOKEN;
        for depth in 1..4096u64 {
            let shifted = module.wrapping_add(depth);
            if shifted % 4096 == 0 {
                assert_eq!(shifted >> 12, 0, "depth {depth}");
            }
        }
        // Above every frame token (a stack address below 2^47, shifted by 12).
        assert!(module > (1u64 << 59));
    }
}
