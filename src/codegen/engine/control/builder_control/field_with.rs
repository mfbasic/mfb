//! plan-176-U: record `WITH` updates — in-place scalar fields, field stores, record temps,
//! self-update items, nested scalar `WITH` and the field self-update site. The statement entry
//! (`lower_ops`) and shared types live in `engine/control/builder_control/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// bug-424 Layer 1, generalized by plan-145-C: store the updated fields of a
    /// `WITH` over a record local (`owner` `Record`) or over a `RES … STATE`
    /// handle's payload (`State`) in place, when every updated field is stored in
    /// its own slot — no whole-record rebuild, so no re-copy of any other
    /// (possibly large, inlined) field.
    ///
    /// Two field kinds qualify:
    ///
    /// * a **scalar** (`!record_field_is_inlined && !record_field_is_pointer`): the
    ///   new value is stored over the old at `8 * index`;
    /// * a **pointer** field (`record_field_is_pointer`, not inlined — a
    ///   `json::Json`, or a record holding one): the new value is built owned
    ///   first (it may read the old one), then the old pointee is dropped exactly
    ///   as the record's scope exit would drop it, and the new pointer stored.
    ///
    /// * (plan-145-F) an inlined record field of compile-time fixed size
    ///   (`record_field_is_inlined_fixed`: `vector::*`, `color::Color`, the
    ///   `datetime` types, a user record of scalars): the new value's bytes are
    ///   copied over the field's sub-block, which is exactly the size of any value
    ///   of the type, so nothing moves. A register-native vector's lanes are stored
    ///   straight into the sub-block's slots; any other value is taken owned (the
    ///   statement's temp, or a copy of a borrowed source — it may be a view into
    ///   this very owner), copied, and freed.
    ///
    /// * (bug-766 D) the owner's LAST inlined field, when it is a `String` or a
    ///   record whose size depends on its value (`replace_field_admitted`). A
    ///   record carries no size word — every free and copy walks it — and its
    ///   last inlined sub-block ends the walk, so replacing that sub-block changes
    ///   nothing but the walked size. The decision is taken at run time: when the
    ///   old and new walked sizes round to the same 16-byte arena granule, the new
    ///   bytes are written over the old span (the block's chunk is exactly what
    ///   every later walk frees); otherwise the owner is rebuilt — a fresh block of
    ///   the new walked size, the old one freed at its own walked size.
    ///
    /// * (bug-784) over an ELEMENT-BOUND owner (a span of a list's data region,
    ///   whose entry records its exact byte length), one `String` or variable-size
    ///   record field anywhere in the record (`element_replace_admitted`): the new
    ///   bytes go over the old ones when the sizes are exactly equal — no offset
    ///   and no length changes — and otherwise the element's write-back
    ///   (`xs = set(xs, i, WITH p { … })`) runs with the values already computed
    ///   (`emit_element_replace_or_write_back`).
    ///
    /// * (bug-784) an inlined record field updated by a `WITH` of that same field
    ///   whose every update is a scalar (`inner := WITH r.inner { count := … }`):
    ///   the scalars are stored into the field's sub-block, whose size cannot
    ///   change, so this works at any inlined position and over any owner.
    ///
    /// Any other inlined field (a collection, a variable-width field that is not
    /// the last) declines to the field seam or the rebuild.
    ///
    /// Every new value is computed first, in source order (matching `WITH`, so a
    /// field that reads another field's old value sees it), spilled, and only then
    /// stored — so a failing value leaves the owner unchanged. `STATE`: the store
    /// goes through the resource record's shared STATE pointer, visible to the
    /// owner and every alias (§15), and `G25` applies. A record: `G1` declines a
    /// by-ref capture (plan-145-H). Returns `true` when handled in place.
    pub(crate) fn try_inplace_scalar_fields(
        &mut self,
        owner: FieldContainer<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        // `G13` — the update must rebuild THIS owner's current value, not install
        // some other record. plan-145-F: through a nested path, each level's.
        let Some((levels, type_, updates)) = self.peel_field_path(owner, value, true) else {
            return Ok(false);
        };
        let path: Vec<(usize, bool)> = levels
            .iter()
            .map(|level| (level.field_index, level.pointer))
            .collect();
        let Some(fields) = self.type_model.record_fields.get(&type_).cloned() else {
            return Ok(false);
        };
        // bug-784: the owner is a span of a list's data region (`lower_element_update`).
        let element = matches!(
            owner,
            FieldContainer::Record { local }
                if self.element_updates.field_owner.as_deref() == Some(local)
        );
        // Every updated field must live in its own slot — a scalar, or (plan-145-C)
        // a pointer field — or be (plan-145-F) a fixed-size inlined record.
        let mut indices = Vec::with_capacity(updates.len());
        for update in updates {
            let Some((index, (_, field_type))) = fields
                .iter()
                .enumerate()
                .find(|(_, (name, _))| name == &update.field)
            else {
                return Ok(false);
            };
            let kind = if self.record_field_is_inlined(field_type) {
                if self.record_field_is_inlined_fixed(field_type) {
                    FieldStoreKind::Overwrite(field_type.clone())
                } else if let Some(nested) = self.nested_scalar_with(
                    owner,
                    &levels,
                    &type_,
                    &update.field,
                    field_type,
                    &update.value,
                ) {
                    FieldStoreKind::Nested(nested)
                } else if (element && self.element_replace_admitted(&type_, field_type))
                    || (path.is_empty()
                        && self.replace_field_admitted(owner, &type_, &fields, index))
                {
                    FieldStoreKind::Replace(field_type.clone())
                } else {
                    return Ok(false);
                }
            } else if self.record_field_is_pointer(field_type) {
                FieldStoreKind::Pointer(field_type.clone())
            } else {
                FieldStoreKind::Scalar
            };
            indices.push((index, kind));
        }
        if indices.is_empty() {
            return Ok(false);
        }
        // bug-784: an element's size-change branch re-runs its write-back over the
        // values computed here, so every value must be one a local can stand for —
        // a fixed-size overwrite may be spilled as lanes — and one field at most
        // may change size.
        let replaces = indices
            .iter()
            .filter(|(_, kind)| matches!(kind, FieldStoreKind::Replace(_)))
            .count();
        if element
            && replaces > 0
            && (replaces > 1
                || indices.iter().any(|(_, kind)| {
                    matches!(
                        kind,
                        FieldStoreKind::Overwrite(_) | FieldStoreKind::Pointer(_)
                    )
                }))
        {
            return Ok(false);
        }
        // bug-784: the replaced field's value is a call of a destination variant
        // (`analysis::into_dest`) — the position of that update.
        let into_call = if element && path.is_empty() {
            self.element_into_call(owner, updates, &indices)
        } else {
            None
        };
        match owner {
            // `G1` — a by-ref capture's slot holds the parent's slot address;
            // plan-145-H: `emit_field_owner_block` reads the block through it.
            FieldContainer::Record { local } => {
                if self.locals.get(local).is_none() {
                    return Ok(false);
                }
            }
            // `G25` (bug-487) — an operand that can reach a `STATE` assignment writes
            // this same block while the arm holds it. This arm re-loads the STATE
            // pointer *after* the operands, so it cannot dangle the way the growing
            // collection arms did, but the divergence is the same one: a nested write
            // to a field this statement does not update survives here, while the
            // whole-state `WITH` this statement is shorthand for (§15) builds its
            // record from a read taken before the operands ran and so discards it.
            // Falling through to that replace is what makes the two agree.
            FieldContainer::State { .. } => {
                if updates.iter().any(|update| {
                    self.inplace_state_operands_reach_a_state_assign(std::slice::from_ref(
                        &update.value,
                    ))
                }) {
                    return Ok(false);
                }
            }
            // plan-145-G: `G-global-operand` over every update value. A value that
            // writes the global would have its write discarded by the `WITH` (built
            // from the global's old value); stored in place it would survive.
            FieldContainer::Global { name } => {
                let leaf = StoreLeaf::Global(name);
                if updates.iter().any(|update| {
                    self.values_reach_store(std::slice::from_ref(&update.value), leaf)
                }) {
                    return Ok(false);
                }
            }
        }
        // Eligible. Compute every new value first (source order, matching WITH so a
        // field that reads another field's old value sees it), spilling each to a
        // slot; then store them into the existing block.
        let mut stores = Vec::with_capacity(updates.len());
        let mut overwrites = Vec::new();
        let mut replace: Option<FieldReplace> = None;
        let mut nested: Vec<FieldNested> = Vec::new();
        // bug-784: per update in source order, the slot(s) its value was spilled
        // to — what an element's size-change branch reads instead of re-running
        // the value.
        let mut spilled_values: Vec<SpilledValue> = Vec::with_capacity(updates.len());
        // bug-784: a destination-variant call's `(result slot, destination slot,
        // type)`.
        let mut into_result: Option<(usize, usize, ParameterType)> = None;
        // plan-145-F: a borrowed overwrite source must be snapshotted only when
        // another store of this `WITH` could change it first.
        let snapshot = indices
            .iter()
            .filter(|(_, kind)| matches!(kind, FieldStoreKind::Overwrite(_)))
            .count()
            > 1
            || indices
                .iter()
                .any(|(_, kind)| matches!(kind, FieldStoreKind::Pointer(_)));
        for (position, (update, (index, kind))) in updates.iter().zip(indices).enumerate() {
            let pointer = match kind {
                FieldStoreKind::Scalar => None,
                FieldStoreKind::Pointer(field_type) => Some(field_type),
                FieldStoreKind::Overwrite(field_type) => {
                    let spilled =
                        self.spill_field_overwrite(&update.value, &field_type, snapshot)?;
                    overwrites.push((index, field_type, spilled));
                    continue;
                }
                FieldStoreKind::Replace(field_type) => {
                    // The new value is only READ (its bytes copied into the owner),
                    // so a fresh one stays the statement's temp and a borrowed one
                    // is left where it lies. Its size is taken now, while the
                    // source is intact. It never overlaps the span it replaces
                    // except as that same span (a record cannot inline its own
                    // type, and every other sub-block ends before it), and the
                    // copy is forward, so `WITH w { tag := w.tag }` copies in place.
                    let into = into_call == Some(position);
                    if into {
                        self.element_updates.into_call_armed = Some(
                            crate::codegen::engine::analysis::handover::call_key(&update.value),
                        );
                    }
                    let lowered = self.lower_value(&update.value);
                    self.element_updates.into_call_armed = None;
                    let lowered = lowered?;
                    // bug-784: a destination variant's result may be the element's
                    // own bytes, which no statement-end free may touch; it is
                    // freed below only when it is a fresh block.
                    if into {
                        self.claim_pending_temp(&lowered);
                    }
                    let lowered = self.materialize_value(lowered)?;
                    let slot = self.allocate_stack_object("record_field_replace", 8);
                    self.emit(abi::store_u64(
                        &lowered.location,
                        abi::stack_pointer(),
                        slot,
                    ));
                    if into {
                        // The destination's address, taken before any store can move
                        // the element (the write-back fallback may relocate it).
                        let dest_slot = self.allocate_stack_object("into_dest", 8);
                        let block = self.emit_field_record_block(owner, &path)?;
                        let dest = self.allocate_register();
                        self.emit(abi::load_u64(&dest, &block, 8 * index));
                        self.emit(abi::add_registers(&dest, &block, &dest));
                        self.emit(abi::store_u64(&dest, abi::stack_pointer(), dest_slot));
                        into_result = Some((slot, dest_slot, field_type.clone()));
                    }
                    let size_slot = self.allocate_stack_object("record_field_replace_size", 8);
                    self.emit_inlined_block_size_from_ptr_slot(&field_type, slot, size_slot)?;
                    replace = Some(FieldReplace {
                        index,
                        slot,
                        size_slot,
                    });
                    spilled_values.push(SpilledValue::One(slot));
                    continue;
                }
                FieldStoreKind::Nested(sub_updates) => {
                    // The inner `WITH`'s values, in its source order — its target is
                    // a field read, which runs nothing.
                    let mut sub_stores = Vec::with_capacity(sub_updates.len());
                    for (sub_index, sub_value) in &sub_updates {
                        let value = self.lower_value(sub_value)?;
                        self.observe_float(sub_value, &value)?;
                        let value = self.materialize_value(value)?;
                        let slot = self.allocate_stack_object("record_nested_inplace", 8);
                        self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
                        sub_stores.push((*sub_index, slot));
                    }
                    spilled_values.push(SpilledValue::Nested(
                        sub_stores.iter().map(|(_, slot)| *slot).collect(),
                    ));
                    nested.push(FieldNested {
                        index,
                        stores: sub_stores,
                    });
                    continue;
                }
            };
            let value = if pointer.is_some() {
                // The record owns the pointee it stores, exactly as `WITH` does.
                self.lower_value_stored_field(&update.value)?
            } else {
                self.lower_value(&update.value)?
            };
            // Observation boundary: a `Float` field must be finite (plan-17).
            self.observe_float(&update.value, &value)?;
            // Materialize a `d`-native float to its GP bit pattern before the spill
            // (plan-01), matching how `lower_with_update` gathers field values.
            let value = self.materialize_value(value)?;
            let slot = self.allocate_stack_object(
                match owner {
                    FieldContainer::State { .. } => "state_field_inplace",
                    FieldContainer::Record { .. } => "record_field_inplace",
                    FieldContainer::Global { .. } => "global_field_inplace",
                },
                8,
            );
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            stores.push((index, slot, pointer));
            spilled_values.push(SpilledValue::One(slot));
        }
        // Drop the pointees the stores displace. Each drop calls the arena, which
        // clobbers the caller-saved registers, so the block pointer is loaded only
        // after every one of them.
        for (index, _, pointer) in &stores {
            let Some(field_type) = pointer else {
                continue;
            };
            let block = self.emit_field_record_block(owner, &path)?;
            let old = self.allocate_register();
            self.emit(abi::load_u64(&old, &block, 8 * index));
            let old_slot = self.allocate_stack_object("record_field_replaced", 8);
            self.emit(abi::store_u64(&old, abi::stack_pointer(), old_slot));
            self.emit_owned_value_drop(&OwnedValueCleanup {
                type_: field_type.clone(),
                stack_offset: old_slot,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            })?;
        }
        let stores: Vec<(usize, usize)> = stores
            .iter()
            .map(|(index, slot, _)| (*index, *slot))
            .collect();
        match &replace {
            None => {
                // One load of the block pointer serves every store: nothing below
                // moves it.
                let block = self.emit_field_record_block(owner, &path)?;
                self.emit_field_stores(&block, &stores, &overwrites, &nested, None);
                // bug-784: a destination variant returns its destination.
                self.emit_into_result_is_owner(owner)?;
            }
            // bug-784: an element-bound owner — same size in place, else the
            // write-back over the spilled values.
            Some(replace) if element => self.emit_element_replace_or_write_back(
                owner,
                value,
                &type_,
                &path,
                &stores,
                &nested,
                replace,
                &spilled_values,
            )?,
            // bug-766 D (`path` is empty: `replace_field_admitted` asked).
            Some(replace) => self.emit_field_replace_or_rebuild(
                owner,
                &type_,
                &stores,
                &overwrites,
                &nested,
                replace,
            )?,
        }
        // bug-784: a destination variant that could not write in place returned a
        // fresh block; it has been copied (or written back) and is freed here. One
        // that wrote in place returned the destination itself, which the element
        // owns.
        if let Some((result_slot, dest_slot, field_type)) = into_result {
            let kept = self.label("into_result_kept");
            let result = self.allocate_register();
            let dest = self.allocate_register();
            self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
            self.emit(abi::load_u64(&dest, abi::stack_pointer(), dest_slot));
            self.emit(abi::compare_registers(&result, &dest));
            self.emit(abi::branch_eq(&kept));
            self.emit_owned_value_drop(&OwnedValueCleanup {
                type_: field_type,
                stack_offset: result_slot,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            })?;
            self.emit(abi::label(&kept));
        }
        // Each owned overwrite source is freed only after every copy (a later one
        // may not read it, but the frees call the arena, which clobbers the block
        // register).
        for (_, field_type, spilled) in overwrites {
            if let FieldOverwrite::Block {
                slot, owned: true, ..
            } = spilled
            {
                self.emit_owned_value_drop(&OwnedValueCleanup {
                    type_: field_type,
                    stack_offset: slot,
                    capacity_slot: None,
                    loop_alias_slot: None,
                    result_wrapper: None,
                })?;
            }
        }
        if let FieldContainer::Record { local } = owner {
            if let Some(local) = self.locals.get_mut(local) {
                local.constant = None;
            }
        }
        Ok(true)
    }

    /// The stores of [`Self::try_inplace_scalar_fields`] into the record at
    /// `block`: every scalar slot, then (plan-145-F) every fixed-size overwrite,
    /// then (bug-766 D) the replaced sub-block at the offset in `replace`'s
    /// second member, then (bug-784) every nested scalar into its field's
    /// sub-block. The nested stores come last because an overwrite or replace
    /// source may be a view into a nested field's sub-block, and every one of
    /// those copies must read it before it changes; a nested store reads nothing
    /// but its own spilled value. Nothing here calls out, so `block` stays live
    /// throughout.
    pub(super) fn emit_field_stores(
        &mut self,
        block: &abi::Reg,
        stores: &[(usize, usize)],
        overwrites: &[(usize, ParameterType, FieldOverwrite)],
        nested: &[FieldNested],
        replace: Option<(&FieldReplace, usize)>,
    ) {
        for (index, slot) in stores {
            let value = self.allocate_register();
            self.emit(abi::load_u64(&value, abi::stack_pointer(), *slot));
            self.emit(abi::store_u64(&value, block, 8 * index));
        }
        // plan-145-F: the fixed-size overwrites.
        for (index, _, spilled) in overwrites {
            let sub = self.allocate_register();
            self.emit(abi::load_u64(&sub, block, 8 * index));
            self.emit(abi::add_registers(&sub, block, &sub));
            match spilled {
                FieldOverwrite::Lanes(lanes) => {
                    for (lane, slot) in lanes.iter().enumerate() {
                        let value = self.allocate_register();
                        self.emit(abi::load_u64(&value, abi::stack_pointer(), *slot));
                        self.emit(abi::store_u64(&value, &sub, 8 * lane));
                    }
                }
                FieldOverwrite::Block {
                    slot, size_slot, ..
                } => {
                    let src = self.allocate_register();
                    let len = self.allocate_register();
                    let scratch = self.allocate_register();
                    self.emit(abi::load_u64(&src, abi::stack_pointer(), *slot));
                    self.emit(abi::load_u64(&len, abi::stack_pointer(), *size_slot));
                    self.emit_block_copy_advance(&sub, &src, &len, &scratch, "field_overwrite");
                }
            }
        }
        // bug-766 D: the new last sub-block, at the field's (unchanged) offset.
        if let Some((replace, offset_slot)) = replace {
            let dst = self.allocate_register();
            let src = self.allocate_register();
            let len = self.allocate_register();
            let scratch = self.allocate_register();
            self.emit(abi::load_u64(&dst, abi::stack_pointer(), offset_slot));
            self.emit(abi::add_registers(&dst, block, &dst));
            self.emit(abi::load_u64(&src, abi::stack_pointer(), replace.slot));
            self.emit(abi::load_u64(&len, abi::stack_pointer(), replace.size_slot));
            self.emit_block_copy_advance(&dst, &src, &len, &scratch, "field_replace");
        }
        // bug-784: the nested scalar stores, at the sub-block's stored offset.
        for field in nested {
            let sub = self.allocate_register();
            self.emit(abi::load_u64(&sub, block, 8 * field.index));
            self.emit(abi::add_registers(&sub, block, &sub));
            for (sub_index, slot) in &field.stores {
                let value = self.allocate_register();
                self.emit(abi::load_u64(&value, abi::stack_pointer(), *slot));
                self.emit(abi::store_u64(&value, &sub, 8 * sub_index));
            }
        }
    }

    /// bug-766 B: a scratch-copy binding's value — `element` is the borrowed
    /// element its `get` returned — copied into the binding's record-temp region
    /// (`size` bytes at `offset`) at once, before anything else runs, so it is a
    /// copy exactly as `materialize_owned_element`'s block would be. The region is
    /// the binding's value; no block is allocated and nothing is freed.
    pub(super) fn copy_into_record_temp(
        &mut self,
        element: ValueResult,
        offset: usize,
        size: usize,
    ) -> Result<ValueResult, String> {
        let element_slot = self.allocate_stack_object("rec_temp_source", 8);
        self.emit(abi::store_u64(
            &element.location,
            abi::stack_pointer(),
            element_slot,
        ));
        // The reservation may call the arena: the element pointer waits in its slot.
        let region = self.emit_record_temp_region(offset)?;
        let source = self.allocate_register();
        self.emit(abi::load_u64(&source, abi::stack_pointer(), element_slot));
        self.emit_copy_record_words(&region, &source, size);
        Ok(ValueResult {
            origin: None,
            type_: element.type_,
            location: Operand::from(region),
            text: element.text,
        })
    }

    /// bug-766 B: lower the item of a `set`/`append` self-update arm. A `WITH` of a
    /// fixed-size record type that fits the function's `WITH`-item region is built
    /// there (`lower_with_update_in_record_temp`): the arm copies the item into
    /// the container before it lowers anything else, and frees nothing it did not
    /// allocate, so the region is dead once the arm has run. Anything else is
    /// `lower_value_stored`.
    pub(crate) fn lower_self_update_item(
        &mut self,
        value: &NirValue,
    ) -> Result<ValueResult, String> {
        if let NirValue::WithUpdate { type_, .. } = value {
            let fits = self.record_temps.slot.is_some()
                && self.record_temps.with_item.is_some_and(|(_, region)| {
                    self.fixed_record_size(type_)
                        .is_some_and(|size| size <= region)
                });
            if fits {
                self.record_temps.with_item_armed = Some(value as *const NirValue as usize);
            }
        }
        let result = self.lower_value_stored(value);
        self.record_temps.with_item_armed = None;
        let result = result?;
        // Rule 4: the item is what this store adds to its container.
        if let Some(target) = self.statement_temps.assign_target.clone() {
            return self.emit_stored_item_float(&target, result);
        }
        Ok(result)
    }

    /// bug-766 B: when `value` is a constructor of `type_` and `type_` is a record
    /// of scalar fields only (8 bytes each, at `8 × index`), lower each argument
    /// as the constructor does — stored-field lowering, the `Float` observation
    /// boundary, materialized — and spill it; `Some(slots)` in field order.
    /// `None`, having emitted nothing, for any other value.
    fn spill_scalar_record_literal(
        &mut self,
        value: &NirValue,
        type_: &ParameterType,
    ) -> Result<Option<Vec<usize>>, String> {
        let NirValue::Constructor {
            type_: constructed,
            args,
        } = value
        else {
            return Ok(None);
        };
        let Some(fields) = self.type_model.record_fields.get(type_) else {
            return Ok(None);
        };
        if constructed != type_
            || args.len() != fields.len()
            || fields.iter().any(|(_, field)| {
                self.record_field_is_inlined(field) || self.record_field_is_pointer(field)
            })
        {
            return Ok(None);
        }
        let mut slots = Vec::with_capacity(args.len());
        for arg in args {
            let lowered = self.lower_value_stored_field(arg)?;
            // Observation boundary: a `Float` record field must be finite (plan-17).
            self.observe_float(arg, &lowered)?;
            let lowered = self.materialize_value(lowered)?;
            let slot = self.allocate_stack_object("rec_temp_literal_field", 8);
            self.emit(abi::store_u64(
                &lowered.location,
                abi::stack_pointer(),
                slot,
            ));
            slots.push(slot);
        }
        Ok(Some(slots))
    }

    /// bug-766 B: `WITH target { … }` of the fixed-size record `type_`, built in the
    /// record-temp scratch's `WITH`-item region instead of a fresh block.
    ///
    /// The order is `lower_with_update`'s: the target, then every update value in
    /// source order, each spilled — all before the first byte of the region is
    /// written, so a failing value writes nothing. Then the target's bytes are
    /// copied into the region and the updated fields stored over them
    /// (`emit_field_stores`, as `try_inplace_scalar_fields` stores into an owner).
    /// No value can alias the region (nothing names it), so a borrowed overwrite
    /// source — possibly a field of the target itself — is copied where it lies.
    pub(crate) fn lower_with_update_in_record_temp(
        &mut self,
        type_: &ParameterType,
        target: &NirValue,
        updates: &[NirRecordUpdate],
    ) -> Result<ValueResult, String> {
        let (offset, region_size) = self
            .record_temps
            .with_item
            .ok_or("native record-temp WITH: the function has no WITH-item region")?;
        let size = self
            .fixed_record_size(type_)
            .filter(|size| *size <= region_size)
            .ok_or_else(|| format!("native record-temp WITH: '{type_}' does not fit"))?;
        let fields = self
            .type_model
            .record_fields
            .get(type_)
            .cloned()
            .ok_or_else(|| format!("native code WITH target '{type_}' is not a record"))?;
        let target_value = self.lower_value(target)?;
        let target_slot = self.allocate_stack_object("rec_temp_with_target", 8);
        self.emit(abi::store_u64(
            &target_value.location,
            abi::stack_pointer(),
            target_slot,
        ));
        let mut stores = Vec::with_capacity(updates.len());
        let mut overwrites = Vec::new();
        for update in updates {
            let Some(index) = fields
                .iter()
                .position(|(field_name, _)| field_name == &update.field)
            else {
                return Err(format!(
                    "native code WITH update references unknown field '{}'",
                    update.field
                ));
            };
            let field_type = fields[index].1.clone();
            if self.record_field_is_inlined(&field_type) {
                // A literal of an all-scalar record (`vector::Float3[…]`) is
                // written field by field into the sub-block: its values are
                // lowered, observed and spilled exactly as the constructor lowers
                // them, but no block is built for them to be copied out of.
                if let Some(lanes) = self.spill_scalar_record_literal(&update.value, &field_type)? {
                    overwrites.push((index, field_type, FieldOverwrite::Lanes(lanes)));
                    continue;
                }
                // Fixed-size, as the whole record is: lanes, or a block to copy.
                let spilled = self.spill_field_overwrite(&update.value, &field_type, false)?;
                overwrites.push((index, field_type, spilled));
                continue;
            }
            let value = self.lower_value(&update.value)?;
            // Observation boundary: a `Float` field updated via WITH must be
            // finite (plan-17).
            self.observe_float(&update.value, &value)?;
            let value = self.materialize_value(value)?;
            let slot = self.allocate_stack_object("rec_temp_with_value", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            stores.push((index, slot));
        }
        let region = self.emit_record_temp_region(offset)?;
        let region_slot = self.allocate_stack_object("rec_temp_with_region", 8);
        self.emit(abi::store_u64(&region, abi::stack_pointer(), region_slot));
        let source = self.allocate_register();
        self.emit(abi::load_u64(&source, abi::stack_pointer(), target_slot));
        self.emit_copy_record_words(&region, &source, size);
        self.emit_field_stores(&abi::Reg::from(&region), &stores, &overwrites, &[], None);
        // An owned overwrite source (a fresh block) is freed after every copy.
        for (_, field_type, spilled) in overwrites {
            if let FieldOverwrite::Block {
                slot, owned: true, ..
            } = spilled
            {
                self.emit_owned_value_drop(&OwnedValueCleanup {
                    type_: field_type,
                    stack_offset: slot,
                    capacity_slot: None,
                    loop_alias_slot: None,
                    result_wrapper: None,
                })?;
            }
        }
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), region_slot));
        Ok(ValueResult {
            origin: None,
            type_: type_.clone(),
            location: Operand::from(result),
            text: format!("with {}", target_value.text),
        })
    }

    /// bug-766 D: whether field `index` of `record_type` may take the
    /// [`FieldStoreKind::Replace`] store over `owner`'s block.
    ///
    /// * It is a `String` or a (non-fixed) record, and no field after it is
    ///   inlined — its sub-block ends the block, so the size walk
    ///   (`emit_record_block_size_to_slot`: `offset + size` of the last present
    ///   inlined field) is `offset + newSize` afterwards and nothing moves.
    /// * The owner is a flat, pointer-free record (`is_freeable_flat_value`): a
    ///   single `arena_free` of its walked size is its whole drop, so a rebuild is
    ///   a byte copy and a free, with no pointee to transfer. `Error` is not taken:
    ///   its last inlined field (`source`) may be absent (offset 0, bug-371).
    /// * The owner's block is an arena chunk of its own that this statement may
    ///   free and repoint — the conditions the copying reassignment frees the
    ///   displaced block under. Never an element-bound owner (a span of a list's
    ///   data region, bug-689), a `TRAP`-owned `Mixed` binding (bug-648), or an
    ///   owner a live `FOR EACH` walks an alias into (bug-430), and never a
    ///   `RES … STATE` payload (bug-424's non-goal keeps its whole-record replace).
    fn replace_field_admitted(
        &self,
        owner: FieldContainer<'_>,
        record_type: &ParameterType,
        fields: &[(String, ParameterType)],
        index: usize,
    ) -> bool {
        let field_type = &fields[index].1;
        let variable = *field_type == ParameterType::String
            || (self.type_model.record_fields.contains_key(field_type)
                && !typed_is_collection_type(field_type));
        if !variable
            || fields
                .iter()
                .skip(index + 1)
                .any(|(_, later)| self.record_field_is_inlined(later))
            || !self.is_freeable_flat_value(record_type)
            || record_type.is_named("Error")
        {
            return false;
        }
        let walked = |name: &str| {
            self.for_each_iterable_locals.iter().any(|n| n == name)
                || self
                    .for_each_iterable_record_fields
                    .iter()
                    .any(|(base, _)| base == name)
        };
        match owner {
            FieldContainer::Record { local } => {
                self.element_updates.field_owner.as_deref() != Some(local)
                    && !self.element_updates.bindings.contains_key(local)
                    && !self.trap_owner_flags.contains_key(local)
                    && !walked(local)
            }
            // A `STATE` payload keeps the whole-record replace for an inlined
            // variable-width field: `string_state_field_still_rebuilds` (bug-424's
            // non-goal) pins it.
            FieldContainer::State { .. } => false,
            FieldContainer::Global { name } => !walked(name),
        }
    }

    /// bug-784: when `value` — the new value of the inlined record field `field`
    /// (of type `field_type`) of `<owner>.<levels…>` — is `WITH <that field> { … }`
    /// whose every update is a scalar slot of `field_type`, those updates as
    /// `(sub-field index, value)`. Storing them into the field's sub-block is the
    /// whole update: a scalar slot is 8 bytes whatever its value, so the
    /// sub-block keeps its size and nothing after it moves. Never in an `Error`
    /// (`record_type`), whose `source` sub-block may be absent — offset 0
    /// (bug-371) — where a store would land in the owner's own slots.
    fn nested_scalar_with(
        &self,
        owner: FieldContainer<'_>,
        levels: &[FieldLevel<'_>],
        record_type: &ParameterType,
        field: &str,
        field_type: &ParameterType,
        value: &NirValue,
    ) -> Option<Vec<(usize, NirValue)>> {
        let NirValue::WithUpdate {
            type_,
            target,
            updates,
        } = value
        else {
            return None;
        };
        if type_ != field_type
            || record_type.is_named("Error")
            || typed_is_collection_type(field_type)
            || updates.is_empty()
            || !field_place_is(target, owner, levels, field)
        {
            return None;
        }
        let fields = self.type_model.record_fields.get(field_type)?;
        updates
            .iter()
            .map(|update| {
                let index = fields.iter().position(|(name, _)| *name == update.field)?;
                let sub_type = &fields[index].1;
                (!self.record_field_is_inlined(sub_type) && !self.record_field_is_pointer(sub_type))
                    .then(|| (index, update.value.clone()))
            })
            .collect()
    }

    /// If `field` is a **collection** field of `record_type` that is inlined AND
    /// is the **last inlined field** (no later field is inlined, so growing its
    /// trailing sub-block extends the record block's tail without shifting any
    /// sibling), return `(field_index, field_type)`. This is the shape bug-430's
    /// in-place grow supports; every other shape falls back to the whole-record
    /// rebuild.
    ///
    /// plan-121-C: this is a *container* question — "can this field's sub-block be
    /// mutated where it lies" — and the answer does not depend on which operation
    /// is about to run. It used to reject anything that was not a `List`, because
    /// its only caller was `append`. That coupled the container to one operation
    /// and made `add`/`removeKey` on a record-held `Set`/`Map` unreachable even
    /// though their sub-blocks are inlined on exactly the same terms.
    ///
    /// Widening it is safe because **each arm still gates its own kind**: the
    /// append arms take `typed_list_element_type` right after this returns (`G9`),
    /// so a `Map`/`Set` field cannot reach a list lowering by this route.
    pub(crate) fn record_collection_last_inlined(
        &self,
        record_type: &ParameterType,
        field: &str,
    ) -> Option<(usize, ParameterType)> {
        let fields = self.type_model.record_fields.get(record_type)?;
        let index = fields.iter().position(|(name, _)| name == field)?;
        let field_type = fields[index].1.clone();
        // A `List`, `Map` or `Set` — the three block-backed collection kinds.
        // `record_field_is_inlined` below already asks `typed_is_collection_type`
        // among its composite cases; asking here too keeps the *reason* for the
        // refusal specific ("not a collection" rather than "not inlined").
        if !typed_is_collection_type(&field_type) {
            return None;
        }
        if !self.record_field_is_inlined(&field_type) {
            return None;
        }
        // No field after this one may be inlined, or growing this sub-block would
        // shift the later sub-blocks (and their stored offsets).
        if fields
            .iter()
            .skip(index + 1)
            .any(|(_, ft)| self.record_field_is_inlined(ft))
        {
            return None;
        }
        Some((index, field_type.clone()))
    }

    /// plan-145-B: the field site of `value` — `r = WITH r { f := op(r.f, …) }`
    /// for a record local (`container` `Record`) or `h.state = WITH h.state { f :=
    /// … }` for a `RES … STATE` handle (`State`; `h.state.f = v` desugars to it) —
    /// and the update's value, which the seam's arms then match as `op(field, …)`.
    ///
    /// The site's own gates (the rest are `resolve_self_update`'s): `G2` the value
    /// is a `WithUpdate`, `G13` its target is this owner (the local, or the
    /// handle's `.state`), `G14` exactly one field is updated — a second would be
    /// dropped when the arm elides the rebuild. Emits nothing.
    ///
    /// plan-145-F: a nested path — `o = WITH o { inner := WITH o.inner { f :=
    /// op(o.inner.f, …) } }` — peels each level whose update is itself a
    /// single-update `WITH` over that level's own place (`G13`/`G14` per level)
    /// into `FieldSite::path`.
    pub(crate) fn field_self_update_site<'s>(
        &self,
        container: FieldContainer<'s>,
        value: &'s NirValue,
    ) -> Option<(FieldSite<'s>, ParameterType, &'s NirValue)> {
        let (path, record_type, updates) = self.peel_field_path(container, value, false)?;
        if updates.len() != 1 {
            return None;
        }
        let update = &updates[0];
        let fields = self.type_model.record_fields.get(&record_type)?;
        let field_index = fields.iter().position(|(name, _)| *name == update.field)?;
        let field_type = fields[field_index].1.clone();
        Some((
            FieldSite {
                container,
                path,
                field: update.field.as_str(),
                field_index,
                record_type,
            },
            field_type,
            &update.value,
        ))
    }

    /// plan-145-F: the record a field `WITH` finally updates, and how it is
    /// reached. `value` must be a `WITH` over the owner (`G13`); while its only
    /// update is an inlined record field whose value is again a single-update
    /// `WITH` over that field's own place, descend. Returns the levels passed
    /// (outermost first), the innermost `WITH`'s record type, and its updates.
    /// Emits nothing.
    ///
    /// `pointer_levels` also descends a POINTER record field (a record holding a
    /// `json::Json`, which is not inlined but owns its own block): only the
    /// store routine may, since it writes that block where it lies; an arm's grow
    /// would have to repoint the parent's slot, so the field seam passes `false`.
    pub(crate) fn peel_field_path<'s>(
        &self,
        container: FieldContainer<'s>,
        value: &'s NirValue,
        pointer_levels: bool,
    ) -> Option<(Vec<FieldLevel<'s>>, ParameterType, &'s [NirRecordUpdate])> {
        let NirValue::WithUpdate {
            type_,
            target,
            updates,
        } = value
        else {
            return None;
        };
        if !field_owner_is(target, container) {
            return None;
        }
        let mut path = Vec::new();
        let mut record_type = type_.clone();
        let mut updates: &[NirRecordUpdate] = updates;
        loop {
            let [update] = updates else {
                break;
            };
            let NirValue::WithUpdate {
                type_: inner_type,
                target: inner_target,
                updates: inner_updates,
            } = &update.value
            else {
                break;
            };
            let Some(fields) = self.type_model.record_fields.get(&record_type) else {
                break;
            };
            let Some(field_index) = fields.iter().position(|(name, _)| *name == update.field)
            else {
                break;
            };
            let field_type = &fields[field_index].1;
            let inlined =
                self.record_field_is_inlined(field_type) && !typed_is_collection_type(field_type);
            let pointer = pointer_levels
                && !self.record_field_is_inlined(field_type)
                && self.record_field_is_pointer(field_type)
                && self.type_model.record_fields.contains_key(field_type);
            if !(inlined || pointer)
                || *field_type != *inner_type
                || !field_place_is(inner_target, container, &path, update.field.as_str())
            {
                break;
            }
            path.push(FieldLevel {
                field: update.field.as_str(),
                field_index,
                record_type: record_type.clone(),
                pointer,
            });
            record_type = inner_type.clone();
            updates = inner_updates;
        }
        Some((path, record_type, updates))
    }
}
