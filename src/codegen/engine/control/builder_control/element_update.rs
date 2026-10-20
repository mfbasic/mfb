//! plan-176-U: collection element updates — element replace and write-back, `INTO` results,
//! hidden binds, append headroom, borrow-get installs and `lower_element_update`. The statement
//! entry (`lower_ops`) and shared types live in `engine/control/builder_control/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// bug-784: whether a field of type `field_type` of the element-bound record
    /// type `record_type` may take the [`FieldStoreKind::Replace`] store over the
    /// element (`emit_element_replace_or_write_back`). It is a `String` or a
    /// variable-size record — at ANY inlined position: the in-place branch runs
    /// only when the new value's size equals the old one's exactly, so no offset
    /// moves and the list entry's recorded length stays true — in a flat record
    /// that is not `Error` (whose last inlined field may be absent, bug-371).
    pub(super) fn element_replace_admitted(
        &self,
        record_type: &ParameterType,
        field_type: &ParameterType,
    ) -> bool {
        (*field_type == ParameterType::String
            || (self.type_model.record_fields.contains_key(field_type)
                && !typed_is_collection_type(field_type)))
            && self.is_freeable_flat_value(record_type)
            && !record_type.is_named("Error")
    }

    /// bug-784: the stores of a `WITH` over an element-bound owner that replaces
    /// one variable-width field, every new value already spilled.
    ///
    /// The owner's block is a span of a list's data region whose entry records
    /// its exact byte length (`lower_list_set_in_place` overwrites a payload where
    /// it lies only at that exact length), so the in-place branch runs when the
    /// new value's size equals the old sub-block's: the bytes go over the old
    /// ones, and neither any later sub-block's offset nor the element's length
    /// changes. Otherwise the element's write-back runs as the source wrote it,
    /// `xs = set(xs, i, WITH p { … })` — the `set` resizes the payload — with
    /// every update value replaced by a hidden local holding the value this
    /// statement already computed, so nothing is evaluated twice. `p` still
    /// aliases the unchanged element there (no store has run), exactly as in
    /// `lower_element_update`'s own fallback.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_element_replace_or_write_back(
        &mut self,
        owner: FieldContainer<'_>,
        value: &NirValue,
        record_type: &ParameterType,
        path: &[(usize, bool)],
        stores: &[(usize, usize)],
        nested: &[FieldNested],
        replace: &FieldReplace,
        spilled_values: &[SpilledValue],
    ) -> Result<(), String> {
        let FieldContainer::Record { local } = owner else {
            return Err("native element replace: the owner is not a record local".into());
        };
        let fields = self
            .type_model
            .record_fields
            .get(record_type)
            .cloned()
            .ok_or_else(|| format!("native element replace: '{record_type}' is not a record"))?;
        let field_type = fields
            .get(replace.index)
            .map(|(_, field_type)| field_type.clone())
            .ok_or_else(|| format!("native element replace: no field {}", replace.index))?;
        let offset_slot = self.allocate_stack_object("element_replace_offset", 8);
        let old_sub_slot = self.allocate_stack_object("element_replace_old_sub", 8);
        let old_size_slot = self.allocate_stack_object("element_replace_old_size", 8);
        let block = self.emit_field_record_block(owner, path)?;
        let offset = self.allocate_register();
        self.emit(abi::load_u64(&offset, &block, 8 * replace.index));
        self.emit(abi::store_u64(&offset, abi::stack_pointer(), offset_slot));
        let sub = self.allocate_register();
        self.emit(abi::add_registers(&sub, &block, &offset));
        self.emit(abi::store_u64(&sub, abi::stack_pointer(), old_sub_slot));
        self.emit_inlined_block_size_from_ptr_slot(&field_type, old_sub_slot, old_size_slot)?;
        let old_size = self.allocate_register();
        let new_size = self.allocate_register();
        self.emit(abi::load_u64(
            &old_size,
            abi::stack_pointer(),
            old_size_slot,
        ));
        self.emit(abi::load_u64(
            &new_size,
            abi::stack_pointer(),
            replace.size_slot,
        ));
        let write_back = self.label("element_replace_write_back");
        let done = self.label("element_replace_done");
        self.emit(abi::compare_registers(&old_size, &new_size));
        self.emit(abi::branch_ne(&write_back));
        // Same size: in place. The size helper called nothing that moves the
        // element, but it may have used the block register; load it again.
        let block = self.emit_field_record_block(owner, path)?;
        self.emit_field_stores(&block, stores, &[], nested, Some((replace, offset_slot)));
        self.emit_into_result_is_owner(owner)?;
        self.emit(abi::branch(&done));
        // A different size: the write-back, over hidden locals.
        self.emit(abi::label(&write_back));
        // bug-784: in a destination variant there is no write-back — the
        // destination is left as it is and the result is a fresh block built from
        // the same values, as the base lowering returns.
        if let Some(result_slot) = self.element_updates.into_result_slot {
            let mut hidden = Vec::new();
            let item =
                self.element_write_back_item(value, path.len(), spilled_values, &mut hidden)?;
            let fresh = self.lower_value(&item);
            for name in hidden {
                self.locals.remove(&name);
            }
            let fresh = fresh?;
            self.claim_pending_temp(&fresh);
            self.emit(abi::store_u64(
                &fresh.location,
                abi::stack_pointer(),
                result_slot,
            ));
            self.emit(abi::label(&done));
            return Ok(());
        }
        let binding = self
            .element_updates
            .bindings
            .get(local)
            .cloned()
            .ok_or_else(|| format!("native element replace: '{local}' is not element-bound"))?;
        let mut hidden = Vec::new();
        let item = self.element_write_back_item(value, path.len(), spilled_values, &mut hidden)?;
        let op = binding.write_back_op(self.element_updates.field_site, item);
        // The write-back is the copying form `lower_element_update` falls back to:
        // it runs with no element owner installed, as that fallback does.
        let owner_name = self.element_updates.field_owner.take();
        let lowered = self.lower_ops(std::slice::from_ref(&op));
        self.element_updates.field_owner = owner_name;
        for name in hidden {
            self.locals.remove(&name);
        }
        lowered?;
        self.emit(abi::label(&done));
        Ok(())
    }

    /// bug-784: in a destination variant's `RETURN WITH <param> { … }`, record
    /// that the result is the destination itself — the owner's own block.
    pub(super) fn emit_into_result_is_owner(
        &mut self,
        owner: FieldContainer<'_>,
    ) -> Result<(), String> {
        let Some(result_slot) = self.element_updates.into_result_slot else {
            return Ok(());
        };
        let block = self.emit_field_owner_block(owner)?;
        self.emit(abi::store_u64(&block, abi::stack_pointer(), result_slot));
        Ok(())
    }

    /// bug-784: which update of an element-bound owner's `WITH` (its position)
    /// may call its callee's destination variant, writing the new field value
    /// straight into the element (`analysis::into_dest`).
    ///
    /// The variant writes the destination while this `WITH`'s values are still
    /// being evaluated, before any of its own stores, so it is admitted only when
    /// that cannot be seen:
    ///
    /// * the update is the one variable-width replace (`FieldStoreKind::Replace`)
    ///   of a site `into_dest` recorded, whose destination argument reads exactly
    ///   `<owner>.<field>`. Every update before it is a scalar or nested scalar,
    ///   spilled as a value, never a view of the element;
    /// * every update after it reads nothing the variant may have written: no
    ///   `<owner>.<field>`, no `<owner>` whole, no other binding that may alias the
    ///   element (a borrowed `get`, an element binding), not the container, and no
    ///   call other than an infallible built-in (`later_value_misses_destination`);
    /// * when an update after it can fail, the failure is not observable: no
    ///   function-level `TRAP` and no inline result capture, and the container is a
    ///   plain owned local of this function — the error leaves the function and
    ///   the container with it. Otherwise a later failure would leave the element
    ///   half updated, where the `WITH` it stands for leaves it unchanged.
    ///
    /// The variant itself reads every source before it writes (it is
    /// `try_inplace_scalar_fields` over the destination: all values first, then
    /// scalar slots, then the one replaced sub-block, whose source can overlap it
    /// only as that same span).
    pub(super) fn element_into_call(
        &self,
        owner: FieldContainer<'_>,
        updates: &[NirRecordUpdate],
        indices: &[(usize, FieldStoreKind)],
    ) -> Option<usize> {
        if self.element_updates.into_result_slot.is_some() {
            return None;
        }
        let FieldContainer::Record { local: owner_name } = owner else {
            return None;
        };
        let binding = self.element_updates.bindings.get(owner_name)?;
        let container = match &binding.template {
            NirOp::Assign { name, .. } => {
                crate::codegen::engine::analysis::borrow_get::ContainerRef::Local(name.clone())
            }
            NirOp::StoreGlobal { name, .. } => {
                crate::codegen::engine::analysis::borrow_get::ContainerRef::Global(name.clone())
            }
            _ => return None,
        };
        let position = indices
            .iter()
            .position(|(_, kind)| matches!(kind, FieldStoreKind::Replace(_)))?;
        if indices[..position]
            .iter()
            .any(|(_, kind)| !matches!(kind, FieldStoreKind::Scalar | FieldStoreKind::Nested(_)))
        {
            return None;
        }
        let update = updates.get(position)?;
        let NirValue::Call { target, args, .. } = &update.value else {
            return None;
        };
        let site = self.element_updates.into_sites.get(
            &crate::codegen::engine::analysis::handover::call_key(&update.value),
        )?;
        if site.target != *target
            || !args
                .get(site.param)
                .is_some_and(|arg| field_place_is(arg, owner, &[], update.field.as_str()))
        {
            return None;
        }
        let mut later_can_fail = false;
        for later in &updates[position + 1..] {
            if !self.later_value_misses_destination(
                &later.value,
                owner_name,
                &update.field,
                &container,
            ) {
                return None;
            }
            later_can_fail |= !self.nir_value_cannot_fail(&later.value);
        }
        if later_can_fail {
            let crate::codegen::engine::analysis::borrow_get::ContainerRef::Local(name) =
                &container
            else {
                return None;
            };
            let plain_local = self.locals.get(name).is_some_and(|local| !local.by_ref)
                && !self.address_taken_locals.contains(name);
            if self.trap.is_some() || self.raw_result_capture.is_some() || !plain_local {
                return None;
            }
        }
        Some(position)
    }

    /// bug-784: whether `value`, evaluated after a destination variant wrote
    /// `<owner>.<field>`, reads only what that write cannot have changed:
    /// constants, other scalar locals, fields of the owner other than `field`
    /// (separate bytes: the write kept the sub-block's exact size), and
    /// infallible built-ins over such values — never the owner whole, the
    /// container, or a binding that may alias the element.
    fn later_value_misses_destination(
        &self,
        value: &NirValue,
        owner: &str,
        field: &str,
        container: &crate::codegen::engine::analysis::borrow_get::ContainerRef,
    ) -> bool {
        use crate::codegen::engine::analysis::borrow_get::ContainerRef;
        match value {
            NirValue::Const { .. } => true,
            NirValue::Local(name) => {
                name != owner
                    && *container != ContainerRef::Local(name.clone())
                    && !self.borrow_get_locals.contains(name)
                    && !self.element_updates.bindings.contains_key(name)
            }
            NirValue::Global { name, .. } => *container != ContainerRef::Global(name.clone()),
            NirValue::MemberAccess { target, member } => match target.as_ref() {
                NirValue::Local(name) if name == owner => member != field,
                _ => self.later_value_misses_destination(target, owner, field, container),
            },
            NirValue::Binary { left, right, .. } => {
                self.later_value_misses_destination(left, owner, field, container)
                    && self.later_value_misses_destination(right, owner, field, container)
            }
            NirValue::Unary { operand, .. } => {
                self.later_value_misses_destination(operand, owner, field, container)
            }
            NirValue::Call { args, .. } | NirValue::RuntimeCall { args, .. } => {
                self.nir_call_is_infallible_builtin(value)
                    && args.iter().all(|arg| {
                        self.later_value_misses_destination(arg, owner, field, container)
                    })
            }
            _ => false,
        }
    }

    /// bug-784: `value` (an element update's `WITH`) with each update value at the
    /// end of its `depth`-level field path replaced by a hidden local bound to the
    /// slot it was spilled to (`spilled`, in update order) — a nested scalar
    /// `WITH` keeps its target and has each of its own values replaced. The
    /// hidden locals' names are pushed to `hidden` for the caller to unbind.
    fn element_write_back_item(
        &mut self,
        value: &NirValue,
        depth: usize,
        spilled: &[SpilledValue],
        hidden: &mut Vec<String>,
    ) -> Result<NirValue, String> {
        let NirValue::WithUpdate {
            type_,
            target,
            updates,
        } = value
        else {
            return Err("native element replace: the update is not a WITH".into());
        };
        let fields = self
            .type_model
            .record_fields
            .get(type_)
            .cloned()
            .ok_or_else(|| format!("native element replace: '{type_}' is not a record"))?;
        let field_type = |field: &str| -> Result<ParameterType, String> {
            fields
                .iter()
                .find(|(name, _)| name == field)
                .map(|(_, field_type)| field_type.clone())
                .ok_or_else(|| format!("native element replace: no field '{field}'"))
        };
        let new_updates = if depth > 0 {
            let [update] = updates.as_slice() else {
                return Err("native element replace: a path level has one update".into());
            };
            vec![NirRecordUpdate {
                field: update.field.clone(),
                value: self.element_write_back_item(&update.value, depth - 1, spilled, hidden)?,
            }]
        } else {
            if updates.len() != spilled.len() {
                return Err("native element replace: one spilled value per update".into());
            }
            let mut new_updates = Vec::with_capacity(updates.len());
            for (update, spilled) in updates.iter().zip(spilled) {
                let field_type = field_type(&update.field)?;
                let value = match spilled {
                    SpilledValue::One(slot) => self.bind_hidden_value(field_type, *slot, hidden),
                    SpilledValue::Nested(slots) => {
                        self.element_nested_write_back_value(&update.value, slots, hidden)?
                    }
                };
                new_updates.push(NirRecordUpdate {
                    field: update.field.clone(),
                    value,
                });
            }
            new_updates
        };
        Ok(NirValue::WithUpdate {
            type_: type_.clone(),
            target: target.clone(),
            updates: new_updates,
        })
    }

    /// bug-784: a [`FieldStoreKind::Nested`] update's `WITH r.inner { … }` with
    /// each of its values replaced by a hidden local bound to its slot (`slots`,
    /// in the inner `WITH`'s order); its target is kept.
    fn element_nested_write_back_value(
        &mut self,
        value: &NirValue,
        slots: &[usize],
        hidden: &mut Vec<String>,
    ) -> Result<NirValue, String> {
        let NirValue::WithUpdate {
            type_,
            target,
            updates,
        } = value
        else {
            return Err("native element replace: a nested update is a WITH".into());
        };
        let fields = self
            .type_model
            .record_fields
            .get(type_)
            .cloned()
            .ok_or_else(|| format!("native element replace: '{type_}' is not a record"))?;
        if updates.len() != slots.len() {
            return Err("native element replace: one slot per nested update".into());
        }
        let mut rewritten = Vec::with_capacity(slots.len());
        for (update, slot) in updates.iter().zip(slots) {
            let sub_type = fields
                .iter()
                .find(|(name, _)| *name == update.field)
                .map(|(_, sub_type)| sub_type.clone())
                .ok_or_else(|| format!("native element replace: no field '{}'", update.field))?;
            rewritten.push(NirRecordUpdate {
                field: update.field.clone(),
                value: self.bind_hidden_value(sub_type, *slot, hidden),
            });
        }
        Ok(NirValue::WithUpdate {
            type_: type_.clone(),
            target: target.clone(),
            updates: rewritten,
        })
    }

    /// bug-784: a hidden local of `type_` whose slot is `slot` (a value already
    /// spilled there), and a read of it. It owns nothing: no cleanup is
    /// registered, and the value's own owner (the statement's temp, or the place
    /// it was borrowed from) still frees it.
    fn bind_hidden_value(
        &mut self,
        type_: ParameterType,
        slot: usize,
        hidden: &mut Vec<String>,
    ) -> NirValue {
        let name = format!("$elementvalue{}", self.element_updates.hidden);
        self.element_updates.hidden += 1;
        self.locals.insert(
            name.clone(),
            LocalValue {
                type_,
                stack_offset: slot,
                constant: None,
                by_ref: false,
            },
        );
        hidden.push(name.clone());
        NirValue::Local(name)
    }

    /// bug-770 C: whether this `Bind` gets its `get`/`getOr` copy with growth
    /// headroom. `analysis::append_headroom` approved the shape (a `MUT` `List`
    /// bound from `get`/`getOr`, the next statement `x = append(x, item)`); this
    /// adds the gates only the builder can answer, failing closed on each: the item
    /// is a single element (not a bulk `append(x, list)`), the element is flat —
    /// the `copy_flat_block` branch of `materialize_owned_element`, not a graph
    /// copy — and the local is not address-taken.
    pub(super) fn append_headroom_bind(
        &self,
        op: &NirOp,
        name: &str,
        type_: &ParameterType,
    ) -> bool {
        let key = crate::codegen::engine::analysis::last_use::op_key(op);
        let Some(item) = self.append_headroom_binds.get(&key) else {
            return false;
        };
        let Some(element) = crate::codegen::engine::types::typed_list_element_type(type_) else {
            return false;
        };
        !self.address_taken_locals.contains(name)
            && self.is_freeable_flat_value(type_)
            && !self.needs_graph_copy(type_)
            && self.static_operand_type(item).as_ref() == Some(element)
    }

    /// plan-86 E, bug-689: take the function's `get` borrows. An element-bound
    /// binding is kept only for an element this lowering really aliases — a record
    /// the `Bind` arm borrows (`is_borrow_get`: freeable-flat, not a `String`) and
    /// not a promoted vector (whose lanes live in registers, so the element would
    /// never see its update). A dropped one leaves `names` too: it is reassigned,
    /// so it must own its block.
    pub(crate) fn install_borrow_gets(
        &mut self,
        borrow_gets: crate::codegen::engine::analysis::borrow_get::BorrowGets,
    ) {
        let mut names = borrow_gets.names;
        let mut updates = ElementUpdates::default();
        for (name, binding) in borrow_gets.elements {
            let aliased = self.is_freeable_flat_value(&binding.type_)
                && binding.type_ != ParameterType::String
                && self.type_model.record_fields.contains_key(&binding.type_)
                && !self.promotable_vector_locals.contains(&name);
            if !aliased {
                names.remove(&name);
                continue;
            }
            for (update, write_back) in binding.sites() {
                if let Some(update) = update {
                    updates.updates.insert(update, name.clone());
                }
                updates.write_backs.extend(write_back);
            }
            updates.bindings.insert(name, binding);
        }
        // bug-784: a re-bound name's role is switched at each of its binds
        // (`apply_rebound`), so it starts in no name-keyed set. An element-bound
        // binding the lowering does not alias is a copy, as above; its sites are
        // registered now (they are keyed by op).
        use crate::codegen::engine::analysis::borrow_get::ReboundRole;
        for (key, mut rebound) in borrow_gets.rebound {
            names.remove(&rebound.name);
            if let ReboundRole::Element(binding) = &rebound.role {
                let aliased = self.is_freeable_flat_value(&binding.type_)
                    && binding.type_ != ParameterType::String
                    && self.type_model.record_fields.contains_key(&binding.type_)
                    && !self.promotable_vector_locals.contains(&rebound.name);
                if aliased {
                    for (update, write_back) in binding.sites() {
                        if let Some(update) = update {
                            updates.updates.insert(update, rebound.name.clone());
                        }
                        updates.write_backs.extend(write_back);
                    }
                } else {
                    rebound.role = ReboundRole::Copy;
                }
            }
            updates.rebound.insert(key, rebound);
        }
        self.borrow_get_locals = names;
        self.element_updates = updates;
        self.install_scratch_copies(borrow_gets.scratch);
    }

    /// bug-784: at the `Bind` `key` of a re-bound name, make the name's
    /// name-keyed borrow state this binding's (`BorrowGets::rebound`). The bindings
    /// of a name have disjoint scopes and the lowering visits them in order, so
    /// every read of the name in this binding's scope sees this state. `true` when
    /// the binding borrows (its scratch copy, if the name has one, is skipped).
    pub(super) fn apply_rebound(&mut self, key: usize) -> bool {
        use crate::codegen::engine::analysis::borrow_get::ReboundRole;
        let Some(rebound) = self.element_updates.rebound.get(&key).cloned() else {
            return false;
        };
        match rebound.role {
            ReboundRole::Copy => {
                self.borrow_get_locals.remove(&rebound.name);
                self.element_updates.bindings.remove(&rebound.name);
                false
            }
            ReboundRole::Borrow => {
                self.borrow_get_locals.insert(rebound.name.clone());
                self.element_updates.bindings.remove(&rebound.name);
                true
            }
            ReboundRole::Element(binding) => {
                self.borrow_get_locals.insert(rebound.name.clone());
                self.element_updates
                    .bindings
                    .insert(rebound.name.clone(), *binding);
                true
            }
        }
    }

    /// bug-689: `p = WITH p { … }` on an element-bound `p` — `p`'s slot holds the
    /// address of the element inside its list, so the plan-145 field routes store
    /// into the element where it lies. None of them may grow the element's block
    /// (`field_is_last_inlined` answers no for it): the block is a span of the
    /// list's data region, not an allocation of its own.
    ///
    /// When no route serves the update, lower what the pair of statements means,
    /// `xs = set(xs, i, WITH p { … })` — the `WITH` builds an owned record reading
    /// the element, and `set` replaces the element (in place when its arm can,
    /// resizing the payload if it must). The write-back that follows is lowered as
    /// nothing either way, and `p` is dead after it, so its slot is never read
    /// again once the payload may have moved.
    ///
    /// bug-766: `site` is the update statement's `op_key` (`None` for a
    /// single-expression update); it may be `LET u = WITH p { … }` (`u` is never
    /// bound — its one read is the write-back) or the fused
    /// `xs = set(xs, i, WITH p { … })`, whose `WITH` is the update and which is
    /// its own write-back.
    pub(super) fn lower_element_update(
        &mut self,
        name: &str,
        site: Option<usize>,
        value: &NirValue,
    ) -> Result<(), String> {
        let value = crate::codegen::engine::analysis::borrow_get::element_update_with(value);
        let binding = self
            .element_updates
            .bindings
            .get(name)
            .cloned()
            .ok_or_else(|| format!("native element update: '{name}' is not element-bound"))?;
        let slot = self
            .locals
            .get(name)
            .map(|local| local.stack_offset)
            .ok_or_else(|| format!("native element update: unknown local '{name}'"))?;
        let owner = FieldContainer::Record { local: name };
        self.element_updates.field_owner = Some(name.to_string());
        self.element_updates.field_site = site;
        let in_place = (|| -> Result<bool, String> {
            if self.try_inplace_scalar_fields(owner, value)? {
                return Ok(true);
            }
            let field_site =
                self.field_self_update_site(owner, value)
                    .map(|(field, field_type, update)| {
                        (
                            SelfUpdateSite {
                                name,
                                type_: field_type,
                                dest: InPlaceDest::Inlined {
                                    block_slot: slot,
                                    field_index: field.field_index,
                                    path: field.path_indices(),
                                    write_back: WriteBack::None,
                                },
                                by_ref: false,
                                field: Some(field),
                            },
                            update,
                        )
                    });
            if let Some((site, update)) = &field_site {
                if self.try_inplace_self_update(site, update)? {
                    return Ok(true);
                }
            }
            self.try_inplace_mixed_with(owner, value, Some(slot), false)
        })();
        self.element_updates.field_owner = None;
        self.element_updates.field_site = None;
        if in_place? {
            return Ok(());
        }
        let write_back = binding.write_back_op(site, value.clone());
        self.lower_ops(std::slice::from_ref(&write_back))
    }

    /// bug-689: `xs = set(xs, i, WITH get(xs, i) { … })` — the single-expression
    /// element update — lowered as the element-bound pair it abbreviates: a hidden
    /// binding holds the element's address (the `get` borrowed, so a bad `i` raises
    /// before anything is written), every `get(xs, i).f` in the `WITH` reads it, and
    /// [`Self::lower_element_update`] updates the element. `Ok(false)` — having
    /// emitted nothing — when the statement is not that shape, or a use of the
    /// element other than a field read, a non-lookup read of a local `xs`, or a
    /// call that can store to a global `xs` rules it out.
    pub(super) fn try_single_expression_element_update(
        &mut self,
        op: &NirOp,
        value: &NirValue,
    ) -> Result<bool, String> {
        use crate::codegen::engine::analysis::borrow_get::{
            bind_single_expression_element, single_expression_update, ContainerRef, ElementBinding,
        };
        let Some((container, index, with)) = single_expression_update(value) else {
            return Ok(false);
        };
        let NirValue::WithUpdate {
            type_,
            target: get,
            updates,
        } = with
        else {
            return Ok(false);
        };
        let element_ok = self.is_freeable_flat_value(type_)
            && *type_ != ParameterType::String
            && self.type_model.record_fields.contains_key(type_);
        let container_ok = match &container {
            ContainerRef::Local(name) => {
                !self.address_taken_locals.contains(name)
                    && self.locals.get(name).is_some_and(|local| !local.by_ref)
            }
            // Nothing the updates run may store to the global: it would free or
            // replace the block the element lives in while the update holds it.
            ContainerRef::Global(name) => !updates.iter().any(|update| {
                self.values_reach_store(
                    std::slice::from_ref(&update.value),
                    StoreLeaf::Global(name),
                )
            }),
        };
        if !element_ok || !container_ok {
            return Ok(false);
        }
        let hidden = format!("$element{}", self.element_updates.hidden);
        let Some(rewritten) = bind_single_expression_element(with, &container, index, &hidden)
        else {
            return Ok(false);
        };
        self.element_updates.hidden += 1;
        let stack_offset = self.allocate_stack_object(&hidden, 8);
        self.locals.insert(
            hidden.clone(),
            LocalValue {
                type_: type_.clone(),
                stack_offset,
                constant: None,
                by_ref: false,
            },
        );
        // The borrowed `get`: its own frame skips the owning copy and the
        // statement-scope free (the alias is the container's, not a fresh block).
        self.borrow_get_armed = true;
        let element = self.lower_value(get);
        self.borrow_get_armed = false;
        self.borrow_get_result = false;
        let element = element?;
        self.store_value_at(&element, abi::stack_pointer(), stack_offset);
        self.element_updates.bindings.insert(
            hidden.clone(),
            ElementBinding {
                type_: type_.clone(),
                template: op.clone(),
                update: None,
                write_back: None,
                other_sites: Vec::new(),
            },
        );
        let result = self.lower_element_update(&hidden, None, &rewritten);
        self.element_updates.bindings.remove(&hidden);
        self.locals.remove(&hidden);
        result.map(|()| true)
    }
}
