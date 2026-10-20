//! plan-176-U: `lower_store_global`, the `NirOp::StoreGlobal` lowering. The statement entry (`lower_ops`) and shared types live in
//! `engine/control/builder_control/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// `NirOp::StoreGlobal`'s store itself: the element-bound and in-place arms,
    /// then the owning copy into the global's slot.
    pub(super) fn lower_store_global(
        &mut self,
        op: &NirOp,
        name: &String,
        type_: &ParameterType,
        value: Option<&NirValue>,
    ) -> Result<(), String> {
        // bug-689: an element-bound write-back — the element already
        // holds the value it writes — and the single-expression update;
        // bug-766: a fused element update is its own write-back.
        let key = crate::codegen::engine::analysis::last_use::op_key(op);
        if self.element_updates.write_backs.contains(&key) {
            return Ok(());
        }
        if let (Some(element), Some(value)) =
            (self.element_updates.updates.get(&key).cloned(), value)
        {
            return self.lower_element_update(&element, Some(key), value);
        }
        if let Some(value) = value {
            if self.try_single_expression_element_update(op, value)? {
                return Ok(());
            }
        }
        let global = self.global_value(name)?;
        let value_type = if crate::codegen::engine::types::is_unset_type(type_) {
            global.type_.clone()
        } else {
            type_.clone()
        };
        // plan-145-G (site S5): `gR = WITH gR { … }` is a field site
        // whose owner block lives in the global's slot — C's stores,
        // F's overwrite and nested paths, then the seam's arms, then
        // the mixed `WITH`, exactly as for a local record.
        if let Some(value) = value {
            let container = FieldContainer::Global { name };
            if self.try_inplace_scalar_fields(container, value)? {
                return Ok(());
            }
            if let Some((field, field_type, update)) = self.field_self_update_site(container, value)
            {
                let site = SelfUpdateSite {
                    name,
                    type_: field_type,
                    dest: InPlaceDest::GlobalField {
                        name: name.clone(),
                        field_index: field.field_index,
                        path: field.path_indices(),
                    },
                    by_ref: false,
                    field: Some(field),
                };
                if self.try_inplace_self_update(&site, update)? {
                    return Ok(());
                }
            }
            if self.try_inplace_mixed_with(container, value, None, false)? {
                return Ok(());
            }
        }
        // plan-142-H (site S2): `g = f(g, …)` mutates the global's own
        // block when an arm recognises `f`, exactly as for a local.
        if let Some(value) = value {
            if is_global_self_update_call(value, name)
                || string_self_append_operands_of(
                    value,
                    &|root| matches!(root, NirValue::Global { name: g, .. } if g == name),
                )
                .is_some()
            {
                let site = SelfUpdateSite {
                    name,
                    type_: global.type_.clone(),
                    dest: InPlaceDest::Global {
                        name: name.clone(),
                        block_slot: self.allocate_stack_object("su_global_block", 8),
                    },
                    by_ref: false,
                    field: None,
                };
                if self.try_inplace_self_update(&site, value)? {
                    return Ok(());
                }
            }
        }
        // A global outlives every scope, so it must own its value
        // independently: deep-copy an aliasing source so freeing a
        // local never dangles the global (plan-02 Phase 8).
        let result = if let Some(value) = value {
            self.lower_value_owned(value)?
        } else {
            let default = self.lower_default_value(&value_type)?;
            // bug-667: the default empty `String` is static rodata;
            // copy it into the arena so the global owns a block its
            // next store can free, exactly as a local's `Bind` does.
            if default.type_ == ParameterType::String {
                let copied = self.copy_flat_block(&ParameterType::String, &default.location)?;
                ValueResult {
                    origin: None,
                    type_: ParameterType::String,
                    location: Operand::from(copied),
                    text: default.text,
                }
            } else {
                default
            }
        };
        // Observation boundary: a `Float` global must be finite
        // (plan-17).
        if let Some(value) = value {
            self.observe_float(value, &result)?;
        }
        // Free the global's previous freeable-flat block before
        // overwriting the slot (bug-47, the bug-01 class). A global
        // carries no `OwnedValue` scope-drop cleanup, and unlike the
        // local `NirOp::Assign` path (:351-385) `StoreGlobal` had no
        // old-block free, so a freeable-flat global reassigned in a
        // loop (`g = collections::filter(g, cb)`) leaked one block per
        // iteration. The old pointer is snapshotted from the global
        // into a stack slot and freed via `emit_owned_value_drop`
        // (null-guarded, so the first store over a zero-initialized
        // global is a no-op; sized from the type incl. a map's bucket
        // region). The freshly computed value is spilled across the
        // `arena_free` (which trashes all caller-saved registers) and
        // the global address is re-derived afterward (its base, the
        // arena-state register x19, is callee-saved and survives the
        // call). `lower_value_owned` deep-copied any aliasing source,
        // so the new block never aliases the freed one — the free is
        // sound and once-only.
        // plan-142-H: a global `String` the concat arm grows keeps its
        // capacity in a hidden global; this store frees the block it
        // describes and installs a tight one, so it resets it.
        let capacity_global = self.global_string_capacity(name);
        if self.is_freeable_flat_value(&value_type) || self.owns_graph(&value_type) {
            let new_slot = self.allocate_stack_object("store_global_new", 8);
            self.emit(abi::store_u64(
                &result.location,
                abi::stack_pointer(),
                new_slot,
            ));
            let old_slot = self.allocate_stack_object("store_global_old", 8);
            let address = self.load_global_address(name)?;
            let old_ptr = self.allocate_register();
            self.emit(abi::load_u64(&old_ptr, &address, 0));
            self.emit(abi::store_u64(&old_ptr, abi::stack_pointer(), old_slot));
            let capacity_slot = match &capacity_global {
                Some(shadow) => {
                    let slot = self.allocate_stack_object("store_global_strcap", 8);
                    let address = self.load_global_address(shadow)?;
                    let spare = self.allocate_register();
                    self.emit(abi::load_u64(&spare, address, 0));
                    self.emit(abi::store_u64(&spare, abi::stack_pointer(), slot));
                    Some(slot)
                }
                None => None,
            };
            self.emit_owned_value_drop(&OwnedValueCleanup {
                type_: value_type.clone(),
                stack_offset: old_slot,
                capacity_slot,
                loop_alias_slot: None,
                result_wrapper: None,
            })?;
            let new_ptr = self.allocate_register();
            self.emit(abi::load_u64(&new_ptr, abi::stack_pointer(), new_slot));
            let stored = ValueResult {
                origin: None,
                type_: result.type_.clone(),
                location: Operand::from(new_ptr),
                text: String::new(),
            };
            let address = self.load_global_address(name)?;
            self.store_value_at(&stored, &address, 0);
            if let Some(shadow) = &capacity_global {
                let address = self.load_global_address(shadow)?;
                self.emit(abi::store_u64(abi::ZERO, address, 0));
            }
        } else {
            let address = self.load_global_address(name)?;
            self.store_value_at(&result, &address, 0);
        }
        Ok(())
    }
}
