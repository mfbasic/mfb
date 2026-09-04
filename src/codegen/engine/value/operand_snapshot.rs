//! bug-496 (audit-3 MEM-12): snapshot an operand that a LATER sibling operand's
//! call can reassign out from under it.
//!
//! Operand 0 of `g = <op>(g, f())` / `GS & f()` lowers to a pointer into the
//! global's *current* block. If a later operand calls a function that reassigns
//! that global, the reassignment's owning store frees the block (`StoreGlobal`'s
//! bug-47 old-block free) and the op then reads through the stale pointer.
//! `arena_free` keeps `[ptr, ptr+16)` as its free-node overlay, so `&` reads the
//! quick-bin link as the byte length (operand 0's bytes silently vanish) and
//! `collections::append` reads COUNT/DATA_LENGTH out of recycled words and asks
//! the arena for a nonsense size (`7-701-0001`).
//!
//! `.ai/collections.md` already states the defined semantics: operand 0 is the
//! value as it was BEFORE the nested call ran, so the nested write is (correctly)
//! lost and only the operand's bytes must survive. This module makes the
//! implementation match that: such an operand is deep-copied into a
//! statement-scope temporary the moment it is lowered — before the later operand
//! runs — and the copy is what the op consumes.
//!
//! The seam is `lower_value` itself, not the individual operand loops: user calls,
//! pre-lowered `abi_inline` bodies, self-lowering builtins, the fused concat chain
//! and every literal all lower their operands through it, so one hook covers them
//! all. Two predicates keep it narrow so the common shapes emit no copy:
//!
//! * `operand_reachable_by_later_call` — the operand must lower to a pointer into
//!   storage a callee can *reassign*: a global, a by-ref or address-taken local
//!   (reassignable through a callback's reference capture), a resource's STATE
//!   (bug-487's copying half: a `RES` parameter is an alias), or a member/extract
//!   of one of those. A plain local is unreachable — value semantics means no
//!   callee can touch it — so `x = append(x, f())` and `s & f()` on locals stay
//!   copy-free, and the in-place `x = append(x, <pure>)` fast path never sees a
//!   `Call` node at all.
//! * `value_contains_reaching_call` — some LATER operand must contain a call
//!   that can run user code: a module function, an indirect call through a FUNC
//!   value, or any call handed a FUNC value (a higher-order builtin invoking a
//!   callback). A pure native builtin (`len`, `toString`, `collections.get`) can
//!   reassign nothing, so `GS & toString(n)` stays copy-free.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use crate::target::shared::nir::visit::{walk_value, NirVisitor};
use crate::target::shared::nir::*;
use crate::types::ParameterType;

impl CodeBuilder<'_> {
    /// Record, for the multi-operand node `value` about to be lowered, every
    /// operand that must be snapshotted (see the module doc). Returns the
    /// `operand_snapshot_wanted` length to truncate back to when the node's
    /// lowering finishes.
    pub(crate) fn push_operand_snapshot_frame(&mut self, value: &NirValue) -> usize {
        let mark = self.operand_snapshot_wanted.len();
        let operands: Vec<&NirValue> = match value {
            NirValue::Call { args, .. }
            | NirValue::CallResult { args, .. }
            | NirValue::RuntimeCall { args, .. }
            | NirValue::Constructor { args, .. }
            | NirValue::ListLiteral { values: args, .. }
            | NirValue::SetLiteral { values: args, .. } => {
                // The last operand has nothing after it, so it never needs a
                // snapshot; skip the allocation unless an earlier one is reachable.
                if args.len() < 2
                    || !args[..args.len() - 1]
                        .iter()
                        .any(|arg| self.operand_reachable_by_later_call(arg))
                {
                    return mark;
                }
                args.iter().collect()
            }
            NirValue::Binary {
                op, left, right, ..
            } => {
                // Mirror `lower_value_inner`: a Level-3 `&` chain of three or more
                // operands is lowered flat, in source order, by
                // `lower_string_concat_chain`; everything else lowers `left` then
                // `right` (a nested `Binary` produces a fresh block of its own).
                let mut parts: Vec<&NirValue> = Vec::new();
                if *op == crate::operators::BinaryOp::Concat && crate::optimizer::level_enabled(3) {
                    super::builder_values::flatten_concat_spine(left, &mut parts);
                    parts.push(right);
                }
                if parts.len() <= 2 {
                    parts.clear();
                    parts.push(left);
                    parts.push(right);
                }
                if !parts[..parts.len() - 1]
                    .iter()
                    .any(|part| self.operand_reachable_by_later_call(part))
                {
                    return mark;
                }
                parts
            }
            NirValue::MapLiteral { entries, .. } => {
                if entries.len() < 2
                    && entries
                        .first()
                        .is_none_or(|(k, _)| !self.operand_reachable_by_later_call(k))
                {
                    return mark;
                }
                entries.iter().flat_map(|(k, v)| [k, v]).collect()
            }
            NirValue::WithUpdate {
                target, updates, ..
            } => {
                if updates.is_empty() {
                    return mark;
                }
                let mut operands = Vec::with_capacity(updates.len() + 1);
                operands.push(target.as_ref());
                operands.extend(updates.iter().map(|update| &update.value));
                if !operands[..operands.len() - 1]
                    .iter()
                    .any(|operand| self.operand_reachable_by_later_call(operand))
                {
                    return mark;
                }
                operands
            }
            _ => return mark,
        };
        for (index, operand) in operands.iter().enumerate() {
            if !self.operand_reachable_by_later_call(operand) {
                continue;
            }
            if operands[index + 1..]
                .iter()
                .any(|later| self.value_contains_reaching_call(later))
            {
                self.operand_snapshot_wanted
                    .push(*operand as *const NirValue as usize);
            }
        }
        mark
    }

    /// If `value` was recorded by `push_operand_snapshot_frame`, deep-copy its
    /// just-lowered `result` into a fresh block registered as a statement-scope
    /// temporary, and hand the copy to the consumer. Only a freeable-flat block
    /// can dangle (a scalar is loaded by value; a register-native vector has no
    /// block), so anything else is returned untouched.
    pub(crate) fn snapshot_aliased_operand(
        &mut self,
        value: &NirValue,
        result: ValueResult,
    ) -> Result<ValueResult, String> {
        if self.operand_snapshot_wanted.is_empty() {
            return Ok(result);
        }
        let address = value as *const NirValue as usize;
        if !self.operand_snapshot_wanted.contains(&address) {
            return Ok(result);
        }
        if Self::is_vector_native(&result) || !self.is_freeable_flat_value(&result.type_) {
            return Ok(result);
        }
        let copied = self.copy_flat_block(&result.type_, &result.location)?;
        // The copy is a fresh block nobody else owns: spill it so the statement's
        // closing `arena_free` (`drop_pending_temps_to`) survives the intervening
        // call clobbers, exactly as `register_pending_temp` does for a call result.
        // Pushed directly rather than through `register_pending_temp`, whose
        // String exclusion exists because a call's String may be rodata or a
        // borrowed view — this one is known to be arena-fresh.
        let slot = self.allocate_stack_object("operand_snapshot", 8);
        self.emit(abi::store_u64(&copied, abi::stack_pointer(), slot));
        let location = Operand::from(copied.render());
        self.pending_temp_frees.push(PendingTemp {
            type_: result.type_.clone(),
            slot,
            location: location.clone(),
        });
        Ok(ValueResult {
            origin: result.origin,
            type_: result.type_,
            location,
            text: result.text,
        })
    }

    /// Whether lowering `value` yields a pointer into storage that a function
    /// called by a LATER operand could reassign (freeing the block behind it).
    fn operand_reachable_by_later_call(&self, value: &NirValue) -> bool {
        match value {
            // Any module function may store to any global.
            NirValue::Global { .. } => true,
            // A by-ref local reads through the parent binding's slot; an
            // address-taken local is captured by reference into a callback that a
            // callee may invoke; a resource local is an alias to one live
            // resource, whose STATE block a callee holding another alias may
            // reallocate (bug-487).
            NirValue::Local(name) => self.locals.get(name).is_some_and(|local| {
                local.by_ref
                    || self.address_taken_locals.contains(name)
                    || self.type_is_resource_handle(&local.type_)
            }),
            NirValue::Capture { by_ref, .. } => *by_ref,
            // A field, a variant payload or a `Result` payload of a reachable
            // value is a pointer into that same block.
            NirValue::MemberAccess { target, .. }
            | NirValue::UnionExtract { value: target, .. }
            | NirValue::ResultValue { value: target }
            | NirValue::ResultError { value: target } => {
                self.operand_reachable_by_later_call(target)
            }
            _ => false,
        }
    }

    fn type_is_resource_handle(&self, type_: &ParameterType) -> bool {
        // `resource_names` is keyed by the bare declared type (plan-111-C), so
        // the STATE clause is peeled first; `parse ∘ name = id` makes the peeled
        // `ParameterType` the key itself — no spelling round-trip.
        matches!(
            type_,
            ParameterType::Res(_) | ParameterType::Stateful { .. }
        ) || crate::codegen::builtins::is_resource_type(type_)
            || self
                .type_model
                .resource_names
                .contains(&type_.without_state())
    }

    /// Whether `value` contains a call that can run user code — a module
    /// function, an indirect call through a FUNC value, or a call handed a FUNC
    /// value (a higher-order builtin invoking a callback). A pure native builtin
    /// can reassign nothing.
    fn value_contains_reaching_call(&self, value: &NirValue) -> bool {
        struct Finder<'b, 'a> {
            builder: &'b CodeBuilder<'a>,
            found: bool,
        }
        impl NirVisitor for Finder<'_, '_> {
            fn visit_value(&mut self, value: &NirValue) {
                if self.found {
                    return;
                }
                let reaching = match value {
                    NirValue::Call { target, args, .. }
                    | NirValue::CallResult { target, args, .. } => {
                        self.builder.call_can_run_user_code(target, args)
                    }
                    NirValue::RuntimeCall { args, .. } => {
                        args.iter().any(|arg| self.builder.is_function_value(arg))
                    }
                    _ => false,
                };
                if reaching {
                    self.found = true;
                    return;
                }
                walk_value(self, value);
            }
        }
        let mut finder = Finder {
            builder: self,
            found: false,
        };
        finder.visit_value(value);
        finder.found
    }

    fn call_can_run_user_code(&self, target: &str, args: &[NirValue]) -> bool {
        self.functions.contains_key(target)
            || self.function_symbols.contains_key(target)
            || self
                .locals
                .get(target)
                .is_some_and(|local| matches!(local.type_, ParameterType::Func(..)))
            || self
                .globals
                .get(target)
                .is_some_and(|global| matches!(global.type_, ParameterType::Func(..)))
            || args.iter().any(|arg| self.is_function_value(arg))
    }

    fn is_function_value(&self, value: &NirValue) -> bool {
        matches!(
            value,
            NirValue::FunctionRef { .. } | NirValue::Closure { .. }
        ) || matches!(self.static_type_name(value), Some(ParameterType::Func(..)))
    }
}
