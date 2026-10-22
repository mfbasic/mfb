//! plan-176-P: field access and `WITH` updates. String concatenation and `MATCH` comparison live in
//! `memory/value/builder_value_semantics/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// bug-689: `get(xs, i).f` — a field read straight off a `get`, through any
    /// chain of record fields — reads the element where it lies when the field it
    /// finally yields is a scalar. The scalar is loaded before anything else runs,
    /// so the alias is dead at the end of the chain; the copy it replaces read the
    /// same bytes at the same moment.
    pub(crate) fn lower_field_access(
        &mut self,
        target: &NirValue,
        member: &str,
    ) -> Result<ValueResult, String> {
        let borrow = self.scalar_field_of_borrowable_get(target, member);
        self.lower_field_access_of(target, member, borrow)
    }

    /// Whether `target.member` is a scalar read off `get`/`getOr` of a record
    /// element the `get` may alias (the `Bind` arm's own `is_borrow_get` type
    /// rule: freeable-flat, not a `String`).
    fn scalar_field_of_borrowable_get(&self, target: &NirValue, member: &str) -> bool {
        let mut base = target;
        while let NirValue::MemberAccess { target, .. } = base {
            base = target;
        }
        let Some(element) = self.get_element_type(base) else {
            return false;
        };
        if !self.is_freeable_flat_value(&element)
            || element == ParameterType::String
            || !self.type_model.record_fields.contains_key(&element)
        {
            return false;
        }
        self.member_chain_type(target, member)
            .is_some_and(|field| self.is_inline_scalar(&field))
    }

    /// The element type a `get`/`getOr` call reads: a `List`'s element, or a
    /// `Map`'s value. `None` for any other value. (`static_type_name` answers only
    /// for lists, and deliberately so — it also drives numeric typing.)
    fn get_element_type(&self, value: &NirValue) -> Option<ParameterType> {
        let NirValue::Call { target, args, .. } = value else {
            return None;
        };
        if !matches!(
            crate::codegen::builtins::native_builtin_target(target),
            Some("get" | "getOr")
        ) {
            return None;
        }
        let container = self.static_type_name(args.first()?)?;
        typed_list_element_type(&container)
            .or_else(|| typed_map_type_parts(&container).map(|(_, value)| value))
            .cloned()
    }

    /// The type of `target.member` for a chain of record field reads over a
    /// `get`, from the record declarations.
    fn member_chain_type(&self, target: &NirValue, member: &str) -> Option<ParameterType> {
        let base = match target {
            NirValue::MemberAccess {
                target: inner,
                member: inner_member,
            } => self.member_chain_type(inner, inner_member)?,
            _ => self.get_element_type(target)?,
        };
        let fields = self.type_model.record_fields.get(&base)?;
        let (_, field) = fields.iter().find(|(name, _)| name == member)?;
        Some(typed_strip_res_marker(field).clone())
    }

    /// A value held in its slot by value: a numeric, `Boolean` or `Byte`, or an
    /// enum ordinal — reading one copies it out, leaving no alias behind.
    fn is_inline_scalar(&self, type_: &ParameterType) -> bool {
        if self.type_model.record_fields.contains_key(type_) {
            return false;
        }
        matches!(
            type_.name().as_ref(),
            "Integer" | "Float" | "Fixed" | "Money" | "Boolean" | "Byte"
        ) || self.type_model.enum_names.contains_key(type_)
    }

    /// The base of a borrowed field chain: each inner field read of the chain, and
    /// at the bottom the `get` itself, lowered with its own frame armed so it
    /// returns the element's address (no owning copy, no statement-scope free).
    fn lower_borrowed_chain_base(&mut self, target: &NirValue) -> Result<ValueResult, String> {
        if let NirValue::MemberAccess {
            target: inner,
            member,
        } = target
        {
            return self.lower_field_access_of(inner, member, true);
        }
        self.borrow_get_armed = true;
        let result = self.lower_value(target);
        self.borrow_get_armed = false;
        self.borrow_get_result = false;
        result
    }

    fn lower_field_access_of(
        &mut self,
        target: &NirValue,
        member: &str,
        borrow: bool,
    ) -> Result<ValueResult, String> {
        let target_value = if borrow {
            self.lower_borrowed_chain_base(target)?
        } else {
            // bug-768: the owner is read only to address one of its fields, so its
            // inlined maps are not settled here — the field read settles what it
            // yields, and a scalar sibling (`rec.before`) needs nothing.
            self.map_settle_exempt_armed = true;
            self.lower_value(target)?
        };
        // plan-01-vector: a field read of a register-native vector is a direct lane
        // read — no block load, no materialization.
        if let Some(lane) = self.vector_native_field(&target_value, member) {
            return Ok(lane);
        }
        // `s.state` on a `RES` value loads the shared `STATE` payload pointer
        // from the resource record. Because a resource value is a pointer to its
        // record, an alias and the owner address the same payload.
        if member == "state" {
            if let Some(state_type) = target_value.type_.state() {
                // A resource union value is a `{tag, record-ptr}` block; the STATE
                // lives in the active variant's record reached via `+8` (plan-74).
                // For a concrete resource the value already IS the record.
                let record =
                    self.emit_resource_record_ptr(&target_value.location, &target_value.type_)?;
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, &record, RESOURCE_OFFSET_STATE));
                // bug-768: a `STATE` payload's inlined maps may hold tombstones.
                self.record_member_read = true;
                return Ok(ValueResult {
                    origin: None,
                    type_: state_type.clone(),
                    location: Operand::from(register),
                    text: "state".to_string(),
                });
            }
        }
        let (field_index, field_type, payload_offset, inline_string) =
            if let Some((key_type, value_type)) = typed_map_entry_type_parts(&target_value.type_)
                .map(|(key, value)| (key.clone(), value.clone()))
            {
                match member {
                    "key" => (0, key_type, 0, false),
                    "value" => (1, value_type, 0, false),
                    _ => {
                        return Err(format!(
                            "native code map entry '{}' has no field '{}'",
                            target_value.type_, member
                        ));
                    }
                }
            } else if let Some(fields) = self.type_model.record_fields.get(&target_value.type_) {
                let Some((index, (_, field_type))) = fields
                    .iter()
                    .enumerate()
                    .find(|(_, (name, _))| name == member)
                else {
                    return Err(format!(
                        "native code record '{}' has no field '{}'",
                        target_value.type_, member
                    ));
                };
                let inline_string = self.record_field_is_inlined(field_type);
                // plan-114-E: a record field read yields the field type with its
                // top-level `RES ` marker stripped, leaving `Stateful { base,
                // state }` (or the bare resource when the field carries no
                // `STATE`). The VALUE is unchanged — the slot already holds the
                // record pointer; only the type spelling differs.
                //
                // This is what lets `h.handle.state` work. `split_state` matches
                // `Stateful` only at the TOP level (`src/types.rs:629`), so
                // `Res(Stateful{..}).state()` is `None` and the `.state` arm above
                // would fall through to "record has no field 'state'" — a message
                // naming the wrong problem entirely.
                //
                // Stripping is unconditional, exactly as the collection element
                // does it (`list_element("List OF RES Socket") == "Socket"`): one
                // rule for both keeps the two positions from drifting, which is
                // the same reasoning §4.1 gives.
                let field_type = typed_strip_res_marker(field_type).clone();
                // bug-768: a record field (or an inlined record) may hold
                // tombstones an in-place field removal left.
                self.record_member_read = true;
                (index, field_type, 0, inline_string)
            } else if let Some(fields) = self
                .type_model
                .union_variant_fields
                .get(&target_value.type_)
            {
                let Some((index, (_, field_type))) = fields
                    .iter()
                    .enumerate()
                    .find(|(_, (name, _))| name == member)
                else {
                    return Err(format!(
                        "native code variant '{}' has no field '{}'",
                        target_value.type_, member
                    ));
                };
                (index, field_type.clone(), 8, false)
            } else {
                // A union itself has no fields — its payload is reached with `MATCH`,
                // and `ir::verify` refuses a field read off a union value
                // (TYPE_FIELD_ACCESS_REQUIRES_RECORD) on source and on decoded package
                // IR alike — so a union, like any other non-record, is refused here.
                return Err(format!(
                    "native code field access target '{}' is not a record or variant",
                    target_value.type_
                ));
            };
        // main made `allocate_register` infallible (the value-numbering landing);
        // the if/else this closes is already closed above.
        let register = self.allocate_register();
        self.emit(abi::load_u64(
            &register,
            &target_value.location,
            payload_offset + 8 * field_index,
        ));
        if inline_string {
            // The slot holds a block-relative offset; the alias pointer to the
            // inlined `String` block is the record base plus that offset
            // (plan-02 §4.2). `target_value.location` survives this add.
            self.emit(abi::add_registers(
                &register,
                &target_value.location,
                &register,
            ));
        }
        Ok(ValueResult {
            origin: None,
            type_: field_type.clone(),
            location: Operand::from(register),
            text: format!("{}.{}", target_value.text, member),
        })
    }

    pub(crate) fn lower_with_update(
        &mut self,
        type_: &ParameterType,
        target: &NirValue,
        updates: &[NirRecordUpdate],
    ) -> Result<ValueResult, String> {
        let fields = self
            .type_model
            .record_fields
            .get(type_)
            .cloned()
            .ok_or_else(|| format!("native code WITH target '{type_}' is not a record"))?;
        let base_reg = self.temporary_vreg();
        let field_reg = self.temporary_vreg();
        let base = &base_reg;
        let field = &field_reg;
        let target_value = self.lower_value(target)?;
        let target_slot = self.allocate_stack_object("with_target", 8);
        self.emit(abi::store_u64(
            &target_value.location,
            abi::stack_pointer(),
            target_slot,
        ));

        // Resolve each updated field to its new value up front (evaluation order
        // matches source order).
        let mut updated: Vec<(usize, usize)> = Vec::with_capacity(updates.len());
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
            // plan-134-E: the updated value is stored into the rebuilt record.
            let value = self.lower_value_stored_field(&update.value)?;
            // Observation boundary: a `Float` field updated via WITH must be
            // finite (plan-17).
            self.observe_float(&update.value, &value)?;
            // Materialize a `d`-native float or a register-native vector before
            // the field-payload spill (plan-01).
            let value = self.materialize_value(value)?;
            let value_slot = self.allocate_stack_object("with_update_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            updated.push((index, value_slot));
        }

        // Gather one value slot per field — the new value where updated, else the
        // old field value read from the target (a `String` field yields the
        // alias pointer `base + offset`) — then rebuild the inlined record so a
        // resized `String` is re-laid-out with correct offsets (plan-02 §4.5).
        let mut field_slots = Vec::with_capacity(fields.len());
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if let Some((_, value_slot)) = updated.iter().find(|(i, _)| *i == index) {
                field_slots.push(*value_slot);
                continue;
            }
            let slot = self.allocate_stack_object("with_old_field", 8);
            self.emit(abi::load_u64(base, abi::stack_pointer(), target_slot));
            self.emit(abi::load_u64(field, base, 8 * index));
            if self.record_field_is_inlined(field_type) {
                self.emit(abi::add_registers(field, base, field));
            }
            self.emit(abi::store_u64(field, abi::stack_pointer(), slot));
            // plan-134-G: a kept recursive field points at a graph the target still owns and
            // frees, so the rebuilt record gets its own copy rather than sharing it.
            if self.owns_graph(field_type) {
                let old = self.temporary_vreg();
                self.emit(abi::load_u64(&old, abi::stack_pointer(), slot));
                let copied = self.copy_value_to_current_arena(field_type, &old)?;
                self.emit(abi::store_u64(&copied, abi::stack_pointer(), slot));
            }
            field_slots.push(slot);
        }
        let register = self.emit_build_inlined_record(type_, &field_slots)?;
        Ok(ValueResult {
            origin: None,
            type_: type_.clone(),
            location: Operand::from(register),
            text: format!("with {}", target_value.text),
        })
    }
}
