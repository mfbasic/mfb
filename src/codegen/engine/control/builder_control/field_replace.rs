//! plan-176-U: field replace and rebuild — owner installs, overwrite spills, inlined fixed
//! fields, the record/owner blocks and the mixed `WITH`. The statement entry (`lower_ops`) and
//! shared types live in `engine/control/builder_control/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// bug-766 D: the stores of a `WITH` that replaces its owner's last inlined
    /// sub-block, every new value already spilled.
    ///
    /// `offset` is the sub-block's stored offset; the owner's walked size is
    /// `offset + oldSize` before and `offset + newSize` after
    /// (`replace_field_admitted`). When both round up to the same 16-byte arena
    /// granule — the size `arena_alloc` handed out and `arena_free` normalizes
    /// every free to (`mfb spec memory arenas`) — the new bytes go over the old
    /// span in place: every later walk of the block frees the chunk it lives in.
    /// Otherwise the owner is rebuilt exactly as the copying `WITH` would build
    /// it: a fresh block of the new walked size (allocated before anything is
    /// written, so `ErrOutOfMemory` leaves the owner unchanged) takes the old
    /// block's bytes up to `offset` (the slots and every earlier sub-block, whose
    /// offsets are block-relative), then the stores; the old block is freed at its
    /// own walked size and the owner repointed.
    pub(super) fn emit_field_replace_or_rebuild(
        &mut self,
        owner: FieldContainer<'_>,
        record_type: &ParameterType,
        stores: &[(usize, usize)],
        overwrites: &[(usize, ParameterType, FieldOverwrite)],
        nested: &[FieldNested],
        replace: &FieldReplace,
    ) -> Result<(), String> {
        let field_type = self
            .type_model
            .record_fields
            .get(record_type)
            .and_then(|fields| fields.get(replace.index))
            .map(|(_, field_type)| field_type.clone())
            .ok_or_else(|| format!("native field replace: no field {}", replace.index))?;
        let old_slot = self.allocate_stack_object("record_replace_old", 8);
        let offset_slot = self.allocate_stack_object("record_replace_offset", 8);
        let old_sub_slot = self.allocate_stack_object("record_replace_old_sub", 8);
        let old_size_slot = self.allocate_stack_object("record_replace_old_size", 8);
        let new_size_slot = self.allocate_stack_object("record_replace_new_size", 8);
        let block = self.emit_field_owner_block(owner)?;
        self.emit(abi::store_u64(&block, abi::stack_pointer(), old_slot));
        let offset = self.allocate_register();
        self.emit(abi::load_u64(&offset, &block, 8 * replace.index));
        self.emit(abi::store_u64(&offset, abi::stack_pointer(), offset_slot));
        let sub = self.allocate_register();
        self.emit(abi::add_registers(&sub, &block, &offset));
        self.emit(abi::store_u64(&sub, abi::stack_pointer(), old_sub_slot));
        self.emit_inlined_block_size_from_ptr_slot(&field_type, old_sub_slot, old_size_slot)?;
        // The two walked sizes, then each rounded to the granule.
        let old_size = self.allocate_register();
        let new_size = self.allocate_register();
        let offset = self.allocate_register();
        let scratch = self.allocate_register();
        self.emit(abi::load_u64(&offset, abi::stack_pointer(), offset_slot));
        self.emit(abi::load_u64(
            &old_size,
            abi::stack_pointer(),
            old_size_slot,
        ));
        self.emit(abi::add_registers(&old_size, &old_size, &offset));
        self.emit(abi::load_u64(
            &new_size,
            abi::stack_pointer(),
            replace.size_slot,
        ));
        self.emit(abi::add_registers(&new_size, &new_size, &offset));
        self.emit(abi::store_u64(
            &new_size,
            abi::stack_pointer(),
            new_size_slot,
        ));
        self.emit_align_offset_register(&old_size, 16, &scratch);
        self.emit_align_offset_register(&new_size, 16, &scratch);
        let rebuild = self.label("record_replace_rebuild");
        let done = self.label("record_replace_done");
        self.emit(abi::compare_registers(&old_size, &new_size));
        self.emit(abi::branch_ne(&rebuild));
        // Same granule: in place.
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), old_slot));
        self.emit_field_stores(
            &abi::Reg::from(&block),
            stores,
            overwrites,
            nested,
            Some((replace, offset_slot)),
        );
        self.emit(abi::branch(&done));
        // A different granule: the rebuild.
        self.emit(abi::label(&rebuild));
        let new_slot = self.allocate_stack_object("record_replace_new", 8);
        let alloc_ok = self.label("record_replace_alloc_ok");
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            new_size_slot,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            new_slot,
        ));
        let dst = self.allocate_register();
        let src = self.allocate_register();
        let len = self.allocate_register();
        self.emit(abi::load_u64(&dst, abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64(&src, abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64(&len, abi::stack_pointer(), offset_slot));
        self.emit_copy_bytes(&dst, &src, &len, "record_replace_prefix");
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), new_slot));
        self.emit_field_stores(
            &abi::Reg::from(&block),
            stores,
            overwrites,
            nested,
            Some((replace, offset_slot)),
        );
        // Every source has been read; free the displaced block at its walked size
        // (it is untouched, so the walk is its allocation's), then repoint.
        self.emit_owned_value_drop(&OwnedValueCleanup {
            type_: record_type.clone(),
            stack_offset: old_slot,
            capacity_slot: None,
            loop_alias_slot: None,
            result_wrapper: None,
        })?;
        self.emit_field_owner_install(owner, new_slot)?;
        self.emit(abi::label(&done));
        Ok(())
    }

    /// bug-766 D: store the block pointer in `slot` as `owner`'s block — the
    /// inverse of [`Self::emit_field_owner_block`]: a record local's slot (a
    /// by-ref capture's through the parent slot's address) or the global's slot.
    /// A `STATE` payload is never admitted (`replace_field_admitted`).
    /// `pub(crate)` for `engine/tests/field_owner_refusals.rs`.
    pub(crate) fn emit_field_owner_install(
        &mut self,
        owner: FieldContainer<'_>,
        slot: usize,
    ) -> Result<(), String> {
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), slot));
        match owner {
            FieldContainer::Record { local } => {
                let local = self
                    .locals
                    .get(local)
                    .ok_or_else(|| format!("native code field store unknown local '{local}'"))?;
                let (local_slot, by_ref) = (local.stack_offset, local.by_ref);
                if by_ref {
                    let parent = self.allocate_register();
                    self.emit(abi::load_u64(&parent, abi::stack_pointer(), local_slot));
                    self.emit(abi::store_u64(&block, &parent, 0));
                } else {
                    self.emit(abi::store_u64(&block, abi::stack_pointer(), local_slot));
                }
            }
            FieldContainer::State { resource } => {
                return Err(format!(
                    "native field replace: '{resource}'.state is never a replace owner \
                     (replace_field_admitted)"
                ));
            }
            FieldContainer::Global { name } => {
                let address = self.load_global_address(name)?;
                self.emit(abi::store_u64(&block, address, 0));
            }
        }
        Ok(())
    }

    /// plan-145-F: evaluate a fixed-size inlined field's new value and spill it —
    /// a register-native vector as its lanes, anything else as an OWNED block and
    /// its byte size (read before any store, while the source is intact). A
    /// borrowed source (a local, a field — possibly of this very owner, as in a
    /// swap `WITH r { v := r.w, w := r.v }`) is copied first when `snapshot` —
    /// when another store of the same `WITH` could change it before its copy runs
    /// (a second overwrite, or a pointer field whose old pointee is dropped). With
    /// no such store the borrowed bytes are copied where they lie, and nothing
    /// is allocated or freed for them.
    pub(super) fn spill_field_overwrite(
        &mut self,
        value: &NirValue,
        field_type: &ParameterType,
        snapshot: bool,
    ) -> Result<FieldOverwrite, String> {
        let lowered = self.lower_value(value)?;
        if let Some(lanes) = self.vector_native_lanes(&lowered) {
            let mut slots = Vec::with_capacity(lanes.len());
            for lane in lanes {
                let lane = self.materialize_float(lane)?;
                let slot = self.allocate_stack_object("record_field_overwrite_lane", 8);
                self.emit(abi::store_u64(&lane.location, abi::stack_pointer(), slot));
                slots.push(slot);
            }
            return Ok(FieldOverwrite::Lanes(slots));
        }
        let (source, owned) = if !self.value_needs_owning_copy(value) {
            // A fresh block: the statement registered it as a temp; this store
            // frees it itself, after the copy.
            self.claim_pending_temp(&lowered);
            (abi::Reg::from(&lowered.location), true)
        } else if snapshot {
            (
                abi::Reg::from(self.copy_flat_block(field_type, &lowered.location)?),
                true,
            )
        } else {
            (abi::Reg::from(&lowered.location), false)
        };
        let slot = self.allocate_stack_object("record_field_overwrite", 8);
        self.emit(abi::store_u64(source, abi::stack_pointer(), slot));
        let size_slot = self.allocate_stack_object("record_field_overwrite_size", 8);
        self.emit_inlined_block_size_from_ptr_slot(field_type, slot, size_slot)?;
        Ok(FieldOverwrite::Block {
            slot,
            size_slot,
            owned,
        })
    }

    /// plan-145-F: an inlined record whose byte size is fixed at compile time —
    /// every field a scalar, or itself such a record (the census's
    /// `FieldKindClass::InlinedFixed`). Any value of the type fills exactly its
    /// sub-block, so a new one can be copied over the old where it lies.
    pub(crate) fn record_field_is_inlined_fixed(&self, type_: &ParameterType) -> bool {
        let Some(fields) = self.type_model.record_fields.get(type_) else {
            return false;
        };
        fields.iter().all(|(_, field)| {
            if self.record_field_is_inlined(field) {
                !typed_is_collection_type(field) && self.record_field_is_inlined_fixed(field)
            } else {
                !self.record_field_is_pointer(field)
            }
        })
    }

    /// plan-145-F: the block of the record a field store writes — the owner's
    /// block, then down `path`: an inlined level adds its stored offset, a pointer
    /// level (`true`) loads the record pointer its slot holds. An empty path is
    /// `emit_field_owner_block`, unchanged.
    pub(super) fn emit_field_record_block(
        &mut self,
        owner: FieldContainer<'_>,
        path: &[(usize, bool)],
    ) -> Result<abi::Reg, String> {
        let block = self.emit_field_owner_block(owner)?;
        let mut base = block;
        for (index, pointer) in path {
            let inner = self.allocate_register();
            if *pointer {
                self.emit(abi::load_u64(&inner, base, 8 * index));
            } else {
                let offset = self.allocate_register();
                self.emit(abi::load_u64(&offset, base, 8 * index));
                self.emit(abi::add_registers(&inner, base, &offset));
            }
            base = inner.into();
        }
        Ok(base)
    }

    /// The owner's block pointer, loaded into a register: a record local's slot,
    /// or the `RES` handle's STATE pointer out of its resource record.
    /// `pub(crate)` for `engine/tests/field_owner_refusals.rs`.
    pub(crate) fn emit_field_owner_block(
        &mut self,
        owner: FieldContainer<'_>,
    ) -> Result<abi::Reg, String> {
        match owner {
            FieldContainer::Record { local } => {
                let local = self
                    .locals
                    .get(local)
                    .ok_or_else(|| format!("native code field store unknown local '{local}'"))?;
                let (slot, by_ref) = (local.stack_offset, local.by_ref);
                let block = self.allocate_register();
                self.emit(abi::load_u64(&block, abi::stack_pointer(), slot));
                // plan-145-H: a by-ref capture's slot holds the parent slot's
                // address; the record pointer is one load further.
                if by_ref {
                    let parent_block = self.allocate_register();
                    self.emit(abi::load_u64(&parent_block, &block, 0));
                    return Ok(parent_block.into());
                }
                Ok(block.into())
            }
            FieldContainer::State { resource } => {
                let local = self.locals.get(resource).ok_or_else(|| {
                    format!("native code state assignment unknown local '{resource}'")
                })?;
                let stack_offset = local.stack_offset;
                let resource_type = local.type_.clone();
                let block = self.allocate_register();
                self.emit(abi::load_u64(&block, abi::stack_pointer(), stack_offset));
                let record = self.emit_resource_record_ptr(&block, &resource_type)?;
                let state_ptr = self.allocate_register();
                self.emit(abi::load_u64(&state_ptr, &record, RESOURCE_OFFSET_STATE));
                Ok(state_ptr.into())
            }
            // plan-145-G: the global's slot.
            FieldContainer::Global { name } => {
                let address = self.load_global_address(name)?;
                let block = self.allocate_register();
                self.emit(abi::load_u64(&block, address, 0));
                Ok(block.into())
            }
        }
    }

    /// plan-145-C: a mixed `WITH` — one update of a collection field that a seam arm
    /// serves, the rest scalar fields (`r = WITH r { xs := append(r.xs, k), n := k
    /// }`) — mutated in place: the arm mutates its field, then the scalars are
    /// stored in their slots. `G14` (one update) is relaxed exactly this far.
    ///
    /// `WITH` evaluates its updates in source order, every one reading the OLD
    /// record, and a failure leaves the record unchanged. The scalar values are
    /// therefore computed BEFORE the arm mutates anything — at its first emission
    /// (`open_inplace_dest`, via `field_pre_emit`) — which keeps both rules:
    ///
    /// * a scalar written BEFORE the arm's update in source order runs before the
    ///   arm's operands anyway;
    /// * one written AFTER it runs earlier than source order, which is only
    ///   unobservable when it has no effect (`nir_value_is_effect_free`) and when it
    ///   and the arm cannot both fail with different errors: it cannot fail
    ///   (`nir_value_cannot_fail`), or the arm's operands and the arm itself cannot.
    ///   Anything else declines to the rebuild.
    ///
    /// A pointer field is not admitted here (its new value would be built before an
    /// arm that can still fail, and leak on that path); the slot-only `WITH` is
    /// `try_inplace_scalar_fields`'s. `STATE`: `G25` applies to every update.
    pub(super) fn try_inplace_mixed_with(
        &mut self,
        container: FieldContainer<'_>,
        value: &NirValue,
        block_slot: Option<usize>,
        by_ref: bool,
    ) -> Result<bool, String> {
        let NirValue::WithUpdate {
            type_,
            target,
            updates,
        } = value
        else {
            return Ok(false);
        };
        if updates.len() < 2 || !Self::with_target_is_owner(container, target) {
            return Ok(false);
        }
        let Some(fields) = self.type_model.record_fields.get(type_).cloned() else {
            return Ok(false);
        };
        let mut arm = None;
        let mut scalars = Vec::with_capacity(updates.len() - 1);
        for (position, update) in updates.iter().enumerate() {
            let Some(index) = fields.iter().position(|(name, _)| *name == update.field) else {
                return Ok(false);
            };
            let field_type = &fields[index].1;
            if self.record_field_is_inlined(field_type) {
                if !typed_is_collection_type(field_type) || arm.is_some() {
                    return Ok(false);
                }
                arm = Some((position, index, field_type.clone()));
            } else if self.record_field_is_pointer(field_type) {
                return Ok(false);
            } else {
                scalars.push((position, index, &update.value));
            }
        }
        // At least two updates, at most one of them the arm and every other one a
        // scalar (anything else returned above), so `scalars` is never empty.
        let Some((arm_position, arm_index, arm_type)) = arm else {
            return Ok(false);
        };
        let arm_value = &updates[arm_position].value;
        let arm_clean = match arm_value {
            NirValue::Call { target, args, .. } => {
                !Self::field_arm_may_raise(target, &arm_type)
                    && args.iter().skip(1).all(|arg| {
                        self.nir_value_is_effect_free(arg) && self.nir_value_cannot_fail(arg)
                    })
            }
            _ => false,
        };
        for (position, _, value) in &scalars {
            if *position > arm_position
                && (!self.nir_value_is_effect_free(value)
                    || !(arm_clean || self.nir_value_cannot_fail(value)))
            {
                return Ok(false);
            }
        }
        // `G25` over every scalar's value and over the arm's OPERANDS — the arm's
        // call itself is the builtin the arm replaces (a `.mfb`-bodied builtin such
        // as `sortBy` would otherwise read as user code), exactly the operands the
        // single-field seam asks about (`resolve_self_update`).
        // plan-145-G: `G-global-operand` — nothing the statement runs may store
        // to the global whose block the arm and the stores write.
        if let FieldContainer::Global { name } = container {
            let leaf = StoreLeaf::Global(name);
            if updates
                .iter()
                .any(|update| self.values_reach_store(std::slice::from_ref(&update.value), leaf))
            {
                return Ok(false);
            }
        }
        if let FieldContainer::State { .. } = container {
            let arm_operands: &[NirValue] = match arm_value {
                NirValue::Call { args, .. } => args,
                other => std::slice::from_ref(other),
            };
            if self.inplace_state_operands_reach_a_state_assign(arm_operands)
                || scalars.iter().any(|(_, _, value)| {
                    self.inplace_state_operands_reach_a_state_assign(std::slice::from_ref(*value))
                })
            {
                return Ok(false);
            }
        }
        let (name, dest) = match container {
            FieldContainer::Record { local } => {
                let slot = block_slot.ok_or("native mixed WITH: no record slot")?;
                (
                    local,
                    // plan-145-H: a by-ref owner writes back through its reference.
                    if by_ref {
                        InPlaceDest::RefField {
                            ref_slot: slot,
                            field_index: arm_index,
                            path: Vec::new(),
                        }
                    } else {
                        InPlaceDest::Inlined {
                            block_slot: slot,
                            field_index: arm_index,
                            path: Vec::new(),
                            write_back: WriteBack::None,
                        }
                    },
                )
            }
            FieldContainer::State { resource } => (
                resource,
                InPlaceDest::StateField {
                    resource: resource.to_string(),
                    field_index: arm_index,
                    path: Vec::new(),
                },
            ),
            FieldContainer::Global { name } => (
                name,
                InPlaceDest::GlobalField {
                    name: name.to_string(),
                    field_index: arm_index,
                    path: Vec::new(),
                },
            ),
        };
        let site = SelfUpdateSite {
            name,
            type_: arm_type,
            dest,
            by_ref,
            field: Some(FieldSite {
                container,
                path: Vec::new(),
                field: updates[arm_position].field.as_str(),
                field_index: arm_index,
                record_type: type_.clone(),
            }),
        };
        self.field_pre_emit = Some(
            scalars
                .iter()
                .map(|(_, index, value)| (*index, (*value).clone()))
                .collect(),
        );
        let accepted = self.try_inplace_self_update(&site, arm_value)?;
        self.field_pre_emit = None;
        let slots = self.field_pre_emitted.take();
        if !accepted {
            return Ok(false);
        }
        let slots = slots.ok_or("native mixed WITH: the arm ran without evaluating its scalars")?;
        // The arm may have moved the block (a grow); load it after.
        let block = self.emit_field_owner_block(container)?;
        for (index, slot) in slots {
            let value = self.allocate_register();
            self.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
            self.emit(abi::store_u64(&value, &block, 8 * index));
        }
        Ok(true)
    }

    /// `G13` for a `WITH`: its target is the owner — the record local itself, the
    /// handle's `.state`, or (plan-145-G) the global.
    fn with_target_is_owner(container: FieldContainer<'_>, target: &NirValue) -> bool {
        field_owner_is(target, container)
    }
}
