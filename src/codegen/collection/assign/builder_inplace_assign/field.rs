//! plan-176-Q: the field routes (plan-145-B) — in-place updates through a record field. The
//! local-variable routes live in `collection/assign/builder_inplace_assign/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// plan-145-B: the field routes of the seam arms.
//
// These are the bodies of the nine record-field arms (plan-121-C) and eight
// `RES … STATE` arms (plan-121-D, bug-430) that used to be dispatched outside the
// seam, moved here unchanged. Each serves the seam arm of the same operation when
// its site is a field (`SelfUpdateTarget::dest.is_field()`), after the seam's
// resolver has run the field container's gates (`G13`/`G14` in the site builder;
// `G15`/`G16`/`G17`/`G10`/`G25`/`G18` in `resolve_self_update`).
//
// The two containers differed in three places, and those are the only branches:
//   * the `STATE` destination is opened (the STATE pointer loaded,
//     `open_inplace_dest`) after the gates and BEFORE the operands are lowered
//     (`O-order-4`) — opening a record destination emits nothing, so calling it at
//     the same point keeps the record arms' bytes;
//   * the slot names, which the `.ncode` records (`inplace_recfield_*` /
//     `inplace_state_*`, and bug-430's `inline_state_rhs`);
//   * `close_inplace_dest` publishes a `STATE` block back through the resource
//     (`O4`) and is a no-op for a record.
// ---------------------------------------------------------------------------

/// The slot name a field route allocates: the record arm's or the `STATE` arm's.
fn field_slot_name(dest: &InPlaceDest, record: &'static str, state: &'static str) -> &'static str {
    if matches!(dest, InPlaceDest::StateField { .. }) {
        state
    } else {
        record
    }
}

impl CodeBuilder<'_> {
    /// A record field's local no longer holds a folded constant after an in-place
    /// update (the record arms cleared it; a `STATE` handle never has one).
    pub(super) fn forget_field_owner_constant(
        &mut self,
        site: &SelfUpdateSite<'_>,
        dest: &InPlaceDest,
    ) {
        if !matches!(dest, InPlaceDest::StateField { .. }) {
            if let Some(local) = self.locals.get_mut(site.name) {
                local.constant = None;
            }
        }
    }

    /// `append` at a field — one element (`bulk = false`) or a whole list — into
    /// the last-inlined `List`, growing the owner's block (bug-430).
    pub(super) fn lower_field_append(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        bulk: bool,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<bool, String> {
        // `G12` — exclude the self-alias `append(field, field)`: the grow frees the
        // old block out from under the RHS copy.
        if site.is_self(&target.args[1]) {
            return Ok(false);
        }
        // `G17` — a grow must not shift a later sibling.
        if !self.field_is_last_inlined(site) {
            return Ok(false);
        }
        // bug-895 — the grow frees the old owner before the item is copied in.
        let item_type = if bulk { list_type } else { element_type };
        if self.field_item_may_view_owner(site, &target.args[1], item_type) {
            return Ok(false);
        }
        let dest = self.open_inplace_dest(&target.dest)?;
        // A whole right-hand list is copied entry by entry (see `try_inplace_bulk_append_assign`).
        let rhs = if bulk {
            self.lower_value_stored_inline(&target.args[1])?
        } else {
            self.lower_value_stored(&target.args[1])?
        };
        self.observe_float(&target.args[1], &rhs)?;
        let rhs = self.materialize_value(rhs)?;
        let rhs_slot = self.allocate_stack_object(
            field_slot_name(&target.dest, "inplace_recfield_rhs", "inline_state_rhs"),
            8,
        );
        self.emit(abi::store_u64(
            &rhs.location,
            abi::stack_pointer(),
            rhs_slot,
        ));
        self.lower_inplace_inlined_list_grow(&dest, bulk, list_type, element_type, rhs_slot)?;
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// `add` at a field: the growing `Set` insert, through `InlineGrow`.
    pub(super) fn lower_field_set_add(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        set_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<bool, String> {
        // `G12` — exclude the self-alias `add(field, field)`.
        if site.is_self(&target.args[1]) {
            return Ok(false);
        }
        // `G17` — a grow must not shift a later sibling.
        if !self.field_is_last_inlined(site) {
            return Ok(false);
        }
        // bug-895 — the grow frees the old owner before the item is copied in.
        if self.field_item_may_view_owner(site, &target.args[1], element_type) {
            return Ok(false);
        }
        let dest = self.open_inplace_dest(&target.dest)?;
        let block_slot = dest.block_slot();
        let item = self.lower_value_stored(&target.args[1])?;
        self.observe_float(&target.args[1], &item)?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_add_item",
                "inplace_state_add_item",
            ),
            8,
        );
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        // A Set is a Map to TRUE.
        let true_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_add_true",
                "inplace_state_add_true",
            ),
            8,
        );
        let true_reg = self.allocate_register();
        self.emit(abi::move_immediate(&true_reg, "Boolean", "true"));
        self.emit(abi::store_u64(&true_reg, abi::stack_pointer(), true_slot));
        // The field's block-relative offset, read once and held across the grow:
        // the prefix is copied verbatim, so the offset survives a realloc where
        // the sub-block *address* does not.
        let field_off_slot = self.open_inplace_inlined_field_offset(&dest)?;
        let set_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_set_in_place(
            set_slot,
            item_slot,
            true_slot,
            set_type,
            element_type,
            &ParameterType::Boolean,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot,
                field_off_slot,
            }),
        )?;
        // `O4` — the grow may have moved a STATE block; publish it.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// `set` at a field. It splits by element width, not collection kind: a
    /// fixed-width `List` element is replaced where it lies, at any field (the
    /// sub-block route — it never changes length). A variable-width element stays in
    /// the list's own block for every length change except the overflow repack,
    /// which grows the owner, so it runs at the owner's last inlined field with
    /// `InlineGrow` (bug-759), and only when its item is not a value that could still
    /// point into the owner. A `Map` takes `InlineGrow`, because a new key grows it.
    pub(super) fn lower_field_set(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        collection_type: &ParameterType,
    ) -> Result<bool, String> {
        if let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(collection_type).cloned()
        {
            // bug-759: a variable-width element can change the data region's length.
            // Every length change stays in the list's own block except the overflow
            // repack, which grows the owner (`InlineGrow`) — so, like any growing
            // field arm (`G17`), only the owner's last inlined field qualifies.
            let variable_width =
                crate::codegen::collection::layout::list_element_is_fixed_width(&element_type)
                    .is_none();
            if variable_width && !self.field_is_last_inlined(site) {
                return Ok(false);
            }
            // The overflow repack frees the old record, so an item that still points
            // into it would be copied from freed memory. `lower_value_stored` does not
            // copy a flat value, so an item reading the owner is admitted only when
            // its top node always builds a new value (`rec.xs` element `& "!"`);
            // anything else declines, as `field_realloc_admitted` would. The index is
            // an `Integer` and cannot alias.
            if variable_width
                && site.read_by(&target.args[2])
                && !matches!(
                    target.args[2],
                    NirValue::Binary { .. } | NirValue::Unary { .. }
                )
            {
                return Ok(false);
            }
            let dest = self.open_inplace_dest(&target.dest)?;
            let index = self.lower_value(&target.args[1])?;
            if index.type_ != ParameterType::Integer {
                return Err(format!(
                    "native collection set list index must be Integer, got {}",
                    index.type_
                ));
            }
            let index = self.materialize_value(index)?;
            let index_slot = self.allocate_stack_object(
                field_slot_name(
                    &target.dest,
                    "inplace_recfield_set_index",
                    "inplace_state_set_index",
                ),
                8,
            );
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            let item = self.lower_value_stored(&target.args[2])?;
            // Observation boundary: an in-place replacement `Float` element must be
            // finite (plan-17).
            self.observe_float(&target.args[2], &item)?;
            if item.type_ != element_type {
                return Err(format!(
                    "native collection set list item must be {element_type}, got {}",
                    item.type_
                ));
            }
            let item = self.materialize_value(item)?;
            let item_slot = self.allocate_stack_object(
                field_slot_name(
                    &target.dest,
                    "inplace_recfield_set_item",
                    "inplace_state_set_item",
                ),
                8,
            );
            self.emit(abi::store_u64(
                &item.location,
                abi::stack_pointer(),
                item_slot,
            ));
            // `O-order-4`: both addresses are taken after the operands are lowered.
            let inline = if variable_width {
                let field_off_slot = self.open_inplace_inlined_field_offset(&dest)?;
                Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                    block_slot: dest.block_slot(),
                    field_off_slot,
                })
            } else {
                None
            };
            let buffer_slot = self.open_inplace_inlined_subblock(&dest)?;
            self.lower_list_set_in_place_with(
                buffer_slot,
                index_slot,
                item_slot,
                collection_type,
                &element_type,
                true,
                inline,
            )?;
            // `O4`. A grow repointed the block slot; a STATE write-back republishes
            // whichever pointer it now holds.
            self.close_inplace_dest(&dest)?;
            self.forget_field_owner_constant(site, &target.dest);
            return Ok(true);
        }
        let Some((key_type, value_type)) =
            crate::codegen::engine::types::typed_map_type_parts(collection_type)
                .map(|(k, v)| (k.clone(), v.clone()))
        else {
            return Ok(false);
        };
        // `G17` — a new key grows the map, which must not shift a later sibling.
        if !self.field_is_last_inlined(site) {
            return Ok(false);
        }
        // bug-895 — the grow frees the old owner before the key and value are
        // copied in.
        if self.field_item_may_view_owner(site, &target.args[1], &key_type)
            || self.field_item_may_view_owner(site, &target.args[2], &value_type)
        {
            return Ok(false);
        }
        let dest = self.open_inplace_dest(&target.dest)?;
        let block_slot = dest.block_slot();
        let key = self.lower_value(&target.args[1])?;
        // Observation boundary: an in-place `Float` map key must be finite (plan-17).
        self.observe_float(&target.args[1], &key)?;
        if key.type_ != key_type {
            return Err(format!(
                "native collection set map key must be {key_type}, got {}",
                key.type_
            ));
        }
        let key = self.materialize_value(key)?;
        let key_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_set_key",
                "inplace_state_set_key",
            ),
            8,
        );
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));
        let val = self.lower_value_stored(&target.args[2])?;
        // Observation boundary: an in-place `Float` map value must be finite (plan-17).
        self.observe_float(&target.args[2], &val)?;
        if val.type_ != value_type {
            return Err(format!(
                "native collection set map value must be {value_type}, got {}",
                val.type_
            ));
        }
        let val = self.materialize_value(val)?;
        let value_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_set_value",
                "inplace_state_set_value",
            ),
            8,
        );
        self.emit(abi::store_u64(
            &val.location,
            abi::stack_pointer(),
            value_slot,
        ));
        let field_off_slot = self.open_inplace_inlined_field_offset(&dest)?;
        let map_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_set_in_place(
            map_slot,
            key_slot,
            value_slot,
            collection_type,
            &key_type,
            &value_type,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot,
                field_off_slot,
            }),
        )?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// `removeKey` at a field: non-growing, so the sub-block route —
    /// `lower_map_remove_key_in_place` only loads the slot it is given.
    pub(super) fn lower_field_remove_key(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        map_type: &ParameterType,
        key_type: &ParameterType,
    ) -> Result<bool, String> {
        let dest = self.open_inplace_dest(&target.dest)?;
        let key = self.lower_value(&target.args[1])?;
        let key = self.materialize_value(key)?;
        let key_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_remove_key",
                "inplace_state_remove_key",
            ),
            8,
        );
        self.store_value_at(&key, abi::stack_pointer(), key_slot);
        // `O-order-4`: the sub-block address is taken AFTER the operand is lowered —
        // lowering the key can itself call `_mfb_arena_alloc`, and the address is
        // never held across an allocation.
        let map_slot = self.open_inplace_inlined_subblock(&dest)?;
        // bug-768: a tombstone, as at a binding — every read of the field, and of
        // its owner as a whole value, settles the sub-block first
        // (`CodeBuilder::emit_tombstone_holder_settle`).
        self.lower_map_remove_key_in_place(map_slot, key_slot, map_type, key_type, true)?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// `removeAt` at a field: it shifts down and decrements `count`, never
    /// reallocating, so the sub-block route serves it.
    pub(super) fn lower_field_remove_at(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<bool, String> {
        let dest = self.open_inplace_dest(&target.dest)?;
        let index = self.lower_value(&target.args[1])?;
        // `E1` — the index is Integer by construction.
        if index.type_ != ParameterType::Integer {
            return Err(format!(
                "native collection removeAt index must be Integer, got {}",
                index.type_
            ));
        }
        let index = self.materialize_value(index)?;
        let index_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_remove_at_index",
                "inplace_state_remove_at_index",
            ),
            8,
        );
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        let buffer_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_list_remove_at_in_place(buffer_slot, index_slot, list_type, element_type)?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// Set `remove` at a field: a Set is a Map to TRUE, so this is the
    /// `removeKey` sub-block route.
    pub(super) fn lower_field_set_remove(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        set_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<bool, String> {
        let dest = self.open_inplace_dest(&target.dest)?;
        let item = self.lower_value_stored(&target.args[1])?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_set_remove",
                "inplace_state_set_remove",
            ),
            8,
        );
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        let set_slot = self.open_inplace_inlined_subblock(&dest)?;
        // bug-768: a tombstone, as at a binding (see `lower_field_remove_key`).
        self.lower_map_remove_key_in_place(set_slot, item_slot, set_type, element_type, true)?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// `insert` (`arity` 3) and `prepend` (`arity` 2) at a field: both grow, so
    /// both take `InlineGrow` (`lower_list_splice_in_place`; a `prepend` is
    /// `SpliceAt::Front`). Bounds are the lowering's own.
    pub(super) fn lower_field_splice(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        arity: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<bool, String> {
        let rhs_index = arity - 1;
        // `G12` — exclude the self-alias: the grow frees the old block out from
        // under the right-hand side copy.
        if site.is_self(&target.args[rhs_index]) {
            return Ok(false);
        }
        // `G11` — the spliced-in value must be a single element.
        match self.static_operand_type(&target.args[rhs_index]) {
            Some(vt) if vt == *element_type => {}
            _ => return Ok(false),
        }
        // `G17` — a grow must not shift a later sibling.
        if !self.field_is_last_inlined(site) {
            return Ok(false);
        }
        // bug-895 — the grow frees the old owner before the item is copied in.
        if self.field_item_may_view_owner(site, &target.args[rhs_index], element_type) {
            return Ok(false);
        }
        let dest = self.open_inplace_dest(&target.dest)?;
        let block_slot = dest.block_slot();
        let at = if arity == 3 {
            let index = self.lower_value(&target.args[1])?;
            if index.type_ != ParameterType::Integer {
                return Err(format!(
                    "native collection insert index must be Integer, got {}",
                    index.type_
                ));
            }
            let index = self.materialize_value(index)?;
            let index_slot = self.allocate_stack_object(
                field_slot_name(
                    &target.dest,
                    "inplace_recfield_splice_index",
                    "inplace_state_splice_index",
                ),
                8,
            );
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            crate::codegen::collection::list::list_mutate::SpliceAt::At(index_slot)
        } else {
            crate::codegen::collection::list::list_mutate::SpliceAt::Front
        };
        let item = self.lower_value_stored(&target.args[rhs_index])?;
        self.observe_float(&target.args[rhs_index], &item)?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_splice_item",
                "inplace_state_splice_item",
            ),
            8,
        );
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        let field_off_slot = self.open_inplace_inlined_field_offset(&dest)?;
        let buffer_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_list_splice_in_place(
            buffer_slot,
            at,
            item_slot,
            list_type,
            element_type,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot,
                field_off_slot,
            }),
        )?;
        // `O4` — the grow may have moved a STATE block; publish it.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }
}
