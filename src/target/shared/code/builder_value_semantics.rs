use super::*;

impl CodeBuilder<'_> {
    /// Default-initialize a `RES` binding's `STATE` payload. The resource value
    /// at `resource_slot` is a pointer to its record; if the state slot
    /// (`FILE_OFFSET_STATE`) is null, allocate and store a default `state_type`
    /// record. A resource that already carries state (moved/returned in) is left
    /// untouched. Values are spilled to the stack across allocations to avoid
    /// register aliasing.
    pub(super) fn emit_resource_state_init(
        &mut self,
        resource_slot: usize,
        state_type: &str,
    ) -> Result<(), String> {
        let ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), resource_slot));
        let current = self.allocate_register()?;
        self.emit(abi::load_u64(&current, &ptr, FILE_OFFSET_STATE));
        let done = self.label("resource_state_init_done");
        self.emit(abi::compare_immediate(&current, "0"));
        self.emit(abi::branch_ne(&done));
        let default = self.lower_default_value(state_type)?;
        let default_slot = self.allocate_stack_object("resource_state_default", 8);
        self.emit(abi::store_u64(
            &default.location,
            abi::stack_pointer(),
            default_slot,
        ));
        let ptr2 = self.allocate_register()?;
        self.emit(abi::load_u64(&ptr2, abi::stack_pointer(), resource_slot));
        let value = self.allocate_register()?;
        self.emit(abi::load_u64(&value, abi::stack_pointer(), default_slot));
        self.emit(abi::store_u64(&value, &ptr2, FILE_OFFSET_STATE));
        self.emit(abi::label(&done));
        Ok(())
    }

    pub(super) fn lower_default_value(&mut self, type_: &str) -> Result<ValueResult, String> {
        match type_ {
            "Nothing" => {
                let register = self.allocate_register()?;
                self.emit(abi::move_immediate(&register, "Integer", "0"));
                Ok(ValueResult {
                    type_: type_.to_string(),
                    location: register,
                    text: "default Nothing".to_string(),
                })
            }
            "Boolean" => {
                let register = self.allocate_register()?;
                self.emit(abi::move_immediate(&register, "Boolean", "0"));
                Ok(ValueResult {
                    type_: type_.to_string(),
                    location: register,
                    text: "default Boolean".to_string(),
                })
            }
            "Byte" | "Integer" | "Float" | "Fixed" => {
                let register = self.allocate_register()?;
                self.emit(abi::move_immediate(&register, type_, "0"));
                Ok(ValueResult {
                    type_: type_.to_string(),
                    location: register,
                    text: format!("default {type_}"),
                })
            }
            "String" => {
                let register = self.load_empty_string_constant()?;
                Ok(ValueResult {
                    type_: type_.to_string(),
                    location: register,
                    text: "default String".to_string(),
                })
            }
            _ if is_collection_type(type_) => {
                let result = self.lower_empty_collection(type_)?;
                Ok(ValueResult {
                    type_: result.type_,
                    location: result.location,
                    text: format!("default {type_}"),
                })
            }
            _ => {
                let Some(fields) = self.type_model.record_fields.get(type_).cloned() else {
                    return Err(format!(
                        "native code cannot materialize default value for type '{type_}'"
                    ));
                };
                let mut field_slots = Vec::with_capacity(fields.len());
                for (_, field_type) in &fields {
                    let value = self.lower_default_value(field_type)?;
                    let slot = self.allocate_stack_object("default_record_field", 8);
                    self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
                    field_slots.push(slot);
                }
                // Inline `String` defaults (empty String blocks) into the record's
                // data region; scalar/pointer defaults stay inline (plan-02 §4.2).
                let register = self.emit_build_inlined_record(type_, &field_slots)?;
                Ok(ValueResult {
                    type_: type_.to_string(),
                    location: register,
                    text: format!("default {type_}"),
                })
            }
        }
    }

    pub(super) fn lower_field_access(
        &mut self,
        target: &NirValue,
        member: &str,
    ) -> Result<ValueResult, String> {
        let target_value = self.lower_value(target)?;
        // `s.state` on a `RES` value loads the shared `STATE` payload pointer
        // from the resource record. Because a resource value is a pointer to its
        // record, a borrow and the owner address the same payload.
        if member == "state" {
            if let Some(state_type) =
                crate::builtins::resource::state_type_name(&target_value.type_)
            {
                let state_type = state_type.to_string();
                let register = self.allocate_register()?;
                self.emit(abi::load_u64(
                    &register,
                    &target_value.location,
                    FILE_OFFSET_STATE,
                ));
                return Ok(ValueResult {
                    type_: state_type,
                    location: register,
                    text: "state".to_string(),
                });
            }
        }
        let (field_index, field_type, payload_offset, inline_string) =
            if let Some((key_type, value_type)) = parse_map_entry_type(&target_value.type_) {
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
                let inline_string = self.record_field_is_inlined(&target_value.type_, field_type);
                (index, field_type.clone(), 0, inline_string)
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
            } else if self.type_model.union_names.contains(&target_value.type_) {
                let matches = self
                    .type_model
                    .union_variant_fields
                    .values()
                    .filter_map(|fields| {
                        fields
                            .iter()
                            .enumerate()
                            .find(|(_, (name, _))| name == member)
                            .map(|(index, (_, field_type))| (index, field_type.clone()))
                    })
                    .collect::<Vec<_>>();
                let Some((index, field_type)) = matches.first().cloned() else {
                    return Err(format!(
                        "native code union '{}' has no payload field '{}'",
                        target_value.type_, member
                    ));
                };
                (index, field_type, 8, false)
            } else {
                return Err(format!(
                    "native code field access target '{}' is not a record or variant",
                    target_value.type_
                ));
            };
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(
            &register,
            &target_value.location,
            payload_offset + 8 * field_index,
        ));
        if inline_string {
            // The slot holds a block-relative offset; the borrow pointer to the
            // inlined `String` block is the record base plus that offset
            // (plan-02 §4.2). `target_value.location` survives this add.
            self.emit(abi::add_registers(
                &register,
                &target_value.location,
                &register,
            ));
        }
        Ok(ValueResult {
            type_: field_type,
            location: register,
            text: format!("{}.{}", target_value.text, member),
        })
    }

    pub(super) fn lower_with_update(
        &mut self,
        type_: &str,
        target: &NirValue,
        updates: &[NirRecordUpdate],
    ) -> Result<ValueResult, String> {
        let fields = self
            .type_model
            .record_fields
            .get(type_)
            .cloned()
            .ok_or_else(|| format!("native code WITH target '{type_}' is not a record"))?;
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
            let value = self.lower_value(&update.value)?;
            // Observation boundary: a `Float` field updated via WITH must be
            // finite (plan-17).
            self.observe_float(&update.value, &value)?;
            // Materialize a `d`-native float before the field-payload spill
            // (plan-01 float-dnative).
            let value = self.materialize_float(value)?;
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
        // borrow pointer `base + offset`) — then rebuild the inlined record so a
        // resized `String` is re-laid-out with correct offsets (plan-02 §4.5).
        let mut field_slots = Vec::with_capacity(fields.len());
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if let Some((_, value_slot)) = updated.iter().find(|(i, _)| *i == index) {
                field_slots.push(*value_slot);
                continue;
            }
            let slot = self.allocate_stack_object("with_old_field", 8);
            self.emit(abi::load_u64("x8", abi::stack_pointer(), target_slot));
            self.emit(abi::load_u64("x9", "x8", 8 * index));
            if self.record_field_is_inlined(type_, field_type) {
                self.emit(abi::add_registers("x9", "x8", "x9"));
            }
            self.emit(abi::store_u64("x9", abi::stack_pointer(), slot));
            field_slots.push(slot);
        }
        let register = self.emit_build_inlined_record(type_, &field_slots)?;
        Ok(ValueResult {
            type_: type_.to_string(),
            location: register,
            text: format!("with {}", target_value.text),
        })
    }

    pub(super) fn lower_string_concat(
        &mut self,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        if left.type_ != "String" {
            return Err(format!(
                "native string concat left operand must be String, got {}",
                left.type_
            ));
        }
        let left_slot = self.allocate_stack_object("concat_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(right)?;
        if right.type_ != "String" {
            return Err(format!(
                "native string concat right operand must be String, got {}",
                right.type_
            ));
        }
        let right_slot = self.allocate_stack_object("concat_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        let total_slot = self.allocate_stack_object("concat_total", 8);

        let alloc_ok = self.label("string_concat_alloc_ok");
        let left_loop = self.label("string_concat_left_loop");
        let left_done = self.label("string_concat_left_done");
        let right_loop = self.label("string_concat_right_loop");
        let right_done = self.label("string_concat_right_done");

        // Copy-loop scratch: one vreg per logical value, colored per-ISA by the
        // allocator (was hand-pinned x8-x16). x1 stays physical — it is the
        // arena_alloc ABI argument/result — and result_ptr is already a vreg.
        let left_len_v = self.temporary_vreg();
        let right_len_v = self.temporary_vreg();
        let total_len_v = self.temporary_vreg();
        let left_cur_v = self.temporary_vreg();
        let write_cur_v = self.temporary_vreg();
        let l8_v = self.temporary_vreg();
        let remaining_v = self.temporary_vreg();
        let right_cur_v = self.temporary_vreg();
        let byte_v = self.temporary_vreg();
        let left_len = left_len_v.as_str();
        let right_len = right_len_v.as_str();
        let total_len = total_len_v.as_str();
        let left_cur = left_cur_v.as_str();
        let write_cur = write_cur_v.as_str();
        let l8 = l8_v.as_str();
        let remaining = remaining_v.as_str();
        let right_cur = right_cur_v.as_str();
        let byte = byte_v.as_str();

        self.emit(abi::load_u64(left_cur, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(write_cur, abi::stack_pointer(), right_slot));
        self.emit(abi::add_immediate(l8, left_cur, 8));
        self.emit(abi::add_immediate(right_cur, write_cur, 8));
        self.emit(abi::load_u64(left_len, left_cur, 0));
        self.emit(abi::load_u64(right_len, write_cur, 0));
        self.emit(abi::add_registers(total_len, left_len, right_len));
        self.emit(abi::store_u64(total_len, abi::stack_pointer(), total_slot));
        self.emit(abi::add_immediate(abi::return_register(), total_len, 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        // Capture the allocation result into a register while x1 is
        // unambiguously the call result at this boundary. The physical return
        // register is fragile to hold across the copy loops below: on ISAs
        // whose result/argument registers differ (x86-64), the loop back-edges
        // break the result-vs-argument dataflow, so a later consumer would
        // arg-map the value. A neutral register carries it safely.
        let result_ptr = self.allocate_register()?;
        self.emit(abi::move_register(&result_ptr, "x1"));
        self.emit(abi::load_u64(left_cur, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(right_cur, abi::stack_pointer(), right_slot));
        self.emit(abi::add_immediate(right_cur, right_cur, 8));
        self.emit(abi::load_u64(left_len, left_cur, 0));
        self.emit(abi::add_immediate(left_cur, left_cur, 8));
        self.emit(abi::load_u64(right_len, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(right_len, right_len, 0));
        self.emit(abi::load_u64(total_len, abi::stack_pointer(), total_slot));
        self.emit(abi::store_u64(total_len, &result_ptr, 0));
        self.emit(abi::add_immediate(write_cur, &result_ptr, 8));
        self.emit(abi::move_register(remaining, left_len));
        self.emit(abi::label(&left_loop));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&left_done));
        self.emit(abi::load_u8(byte, left_cur, 0));
        self.emit(abi::store_u8(byte, write_cur, 0));
        self.emit(abi::add_immediate(left_cur, left_cur, 1));
        self.emit(abi::add_immediate(write_cur, write_cur, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&left_loop));
        self.emit(abi::label(&left_done));
        self.emit(abi::move_register(remaining, right_len));
        self.emit(abi::label(&right_loop));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&right_done));
        self.emit(abi::load_u8(byte, right_cur, 0));
        self.emit(abi::store_u8(byte, write_cur, 0));
        self.emit(abi::add_immediate(right_cur, right_cur, 1));
        self.emit(abi::add_immediate(write_cur, write_cur, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&right_loop));
        self.emit(abi::label(&right_done));
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, write_cur, 0));

        Ok(ValueResult {
            type_: "String".to_string(),
            location: result_ptr,
            text: format!("({} & {})", left.text, right.text),
        })
    }

    pub(super) fn global_value(&self, name: &str) -> Result<GlobalValue, String> {
        self.globals
            .get(name)
            .cloned()
            .ok_or_else(|| format!("native code global '{name}' does not resolve"))
    }

    pub(super) fn load_global_address(&mut self, name: &str) -> Result<String, String> {
        let global = self.global_value(name)?;
        let register = self.allocate_register()?;
        self.emit(abi::add_immediate(
            &register,
            ARENA_STATE_REGISTER,
            global.offset,
        ));
        Ok(register)
    }

    pub(super) fn local_constant_value(&self, value: &NirValue) -> Option<NirValue> {
        match value {
            NirValue::Const { .. } => Some(value.clone()),
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.clone()),
            NirValue::Global { .. } => None,
            NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => self
                .static_primitive_text(&args[0])
                .map(|value| NirValue::Const {
                    type_: "String".to_string(),
                    value,
                }),
            NirValue::RuntimeCall { target, args, .. }
                if target == "toString" && args.len() == 1 =>
            {
                self.static_primitive_text(&args[0])
                    .map(|value| NirValue::Const {
                        type_: "String".to_string(),
                        value,
                    })
            }
            NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. }
                if target == "typeName" && args.len() == 1 =>
            {
                self.static_type_name(&args[0])
                    .map(|value| NirValue::Const {
                        type_: "String".to_string(),
                        value,
                    })
            }
            NirValue::Binary { op, .. } if op == "&" => {
                self.static_string_value(value)
                    .map(|value| NirValue::Const {
                        type_: "String".to_string(),
                        value,
                    })
            }
            _ => None,
        }
    }

    pub(super) fn static_string_value(&self, value: &NirValue) -> Option<String> {
        match value {
            NirValue::Const { type_, value } if type_ == "String" => Some(value.clone()),
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.as_ref())
                .and_then(|constant| self.static_string_value(constant)),
            NirValue::Global { .. } => None,
            NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
                self.static_primitive_text(&args[0])
            }
            NirValue::RuntimeCall { target, args, .. }
                if target == "toString" && args.len() == 1 =>
            {
                self.static_primitive_text(&args[0])
            }
            NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. }
                if target == "typeName" && args.len() == 1 =>
            {
                self.static_type_name(&args[0])
            }
            NirValue::Binary {
                op, left, right, ..
            } if op == "&" => {
                let left = self.static_string_value(left)?;
                let right = self.static_string_value(right)?;
                Some(format!("{left}{right}"))
            }
            _ => None,
        }
    }

    pub(super) fn static_primitive_text(&self, value: &NirValue) -> Option<String> {
        match value {
            NirValue::Const { type_, value } => match type_.as_str() {
                "Integer" | "Byte" | "Float" | "Fixed" | "String" => Some(value.clone()),
                "Boolean" => match value.as_str() {
                    "true" => Some("TRUE".to_string()),
                    "false" => Some("FALSE".to_string()),
                    _ => None,
                },
                _ => None,
            },
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.as_ref())
                .and_then(|constant| self.static_primitive_text(constant)),
            NirValue::Global { .. } => None,
            _ => None,
        }
    }

    pub(super) fn static_type_name(&self, value: &NirValue) -> Option<String> {
        match value {
            NirValue::Const { type_, .. } => Some(type_.clone()),
            NirValue::Local(name) => self.locals.get(name).map(|local| local.type_.clone()),
            NirValue::LocalRef { type_, .. } => Some(type_.clone()),
            NirValue::Global { name, type_ } => {
                if type_.is_empty() {
                    self.globals.get(name).map(|global| global.type_.clone())
                } else {
                    Some(type_.clone())
                }
            }
            NirValue::FunctionRef { type_, .. }
            | NirValue::Closure { type_, .. }
            | NirValue::Capture { type_, .. }
            | NirValue::Constructor { type_, .. }
            | NirValue::WithUpdate { type_, .. }
            | NirValue::ListLiteral { type_, .. }
            | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
            NirValue::UnionWrap { union_type, .. } => Some(union_type.clone()),
            NirValue::UnionExtract { type_, .. } => Some(type_.clone()),
            NirValue::ResultIsOk { .. } => Some("Boolean".to_string()),
            NirValue::ResultValue { value } => self
                .static_type_name(value)
                .and_then(|type_| type_.strip_prefix("Result OF ").map(str::to_string))
                .or_else(|| self.static_type_name(value)),
            NirValue::ResultError { .. } => Some("Error".to_string()),
            NirValue::Call { target, args, .. }
            | NirValue::CallResult { target, args, .. }
            | NirValue::RuntimeCall { target, args, .. } => match target.as_str() {
                "replace" | "typeName" | "toString" => Some("String".to_string()),
                "find" | "len" | "toInt" => Some("Integer".to_string()),
                "mid" => Some("String".to_string()),
                "toFloat" => Some("Float".to_string()),
                "toFixed" => Some("Fixed".to_string()),
                "toByte" => Some("Byte".to_string()),
                "isNumeric" => Some("Boolean".to_string()),
                "math.floor" | "math.ceil" | "math.round" => Some("Integer".to_string()),
                "math.sqrt" | "math.exp" | "math.log" | "math.log10" | "math.sin" | "math.cos"
                | "math.tan" | "math.asin" | "math.acos" | "math.atan" => {
                    args.first().and_then(|arg| self.static_type_name(arg))
                }
                "math.pow" | "math.atan2" => {
                    args.first().and_then(|arg| self.static_type_name(arg))
                }
                _ => None,
            },
            NirValue::Binary {
                op, left, right, ..
            } => {
                if matches!(
                    op.as_str(),
                    "=" | "<>" | "<" | ">" | "<=" | ">=" | "AND" | "OR" | "XOR"
                ) {
                    return Some("Boolean".to_string());
                }
                if op == "&" {
                    return Some("String".to_string());
                }
                let left = self.static_type_name(left)?;
                let right = self.static_type_name(right)?;
                Some(numeric_binary_result_type(op, &left, &right).to_string())
            }
            NirValue::Unary { op, operand, .. } => {
                if op == "NOT" {
                    Some("Boolean".to_string())
                } else {
                    self.static_type_name(operand)
                }
            }
            NirValue::MemberAccess { target, member } => {
                let target_type = self.static_type_name(target)?;
                if member == "result" {
                    if let Some(output_type) = builtins::thread::parent_thread_output(&target_type)
                    {
                        return Some(format!("Result OF {output_type}"));
                    }
                }
                let (key_type, value_type) = parse_map_entry_type(&target_type)?;
                match member.as_str() {
                    "key" => Some(key_type),
                    "value" => Some(value_type),
                    _ => None,
                }
            }
        }
    }

    pub(super) fn thread_runtime_return_type(
        &self,
        target: &str,
        args: &[NirValue],
    ) -> Option<String> {
        match target {
            "thread.start" => {
                let function_type = self.static_type_name(args.first()?)?;
                function_type
                    .strip_prefix("ISOLATED FUNC(")?
                    .split_once(") AS ")
                    .and_then(|(params, _)| split_top_level_types(params).into_iter().next())
            }
            "thread.isRunning" | "thread.poll" | "thread.isCancelled" => {
                Some("Boolean".to_string())
            }
            "thread.cancel" | "thread.send" | "thread.transferResource" | "thread.emitResource" => {
                Some("Nothing".to_string())
            }
            "thread.waitFor" => {
                let thread_type = self.static_type_name(args.first()?)?;
                builtins::thread::parent_thread_output(&thread_type).map(str::to_string)
            }
            "thread.receive" => {
                let thread_type = self.static_type_name(args.first()?)?;
                builtins::thread::thread_message(&thread_type).map(str::to_string)
            }
            // The resource plane: accept yields the thread's resource type
            // (worker reads the inbound queue, parent reads the outbound queue).
            "thread.acceptResource" | "thread.readResource" => {
                let thread_type = self.static_type_name(args.first()?)?;
                builtins::thread::thread_resource(&thread_type).map(str::to_string)
            }
            _ => None,
        }
    }

    pub(super) fn lower_match_compare(
        &mut self,
        matched: &ValueResult,
        pattern: &NirValue,
        label: &str,
    ) -> Result<(), String> {
        match pattern {
            NirValue::MemberAccess { target, member } => {
                let NirValue::Local(type_name) = target.as_ref() else {
                    return Err("native code enum match pattern must name enum type".to_string());
                };
                let ordinal = self
                    .type_model
                    .enum_members
                    .get(&(type_name.clone(), member.clone()))
                    .copied()
                    .ok_or_else(|| {
                        format!("native code enum member '{type_name}.{member}' does not resolve")
                    })?;
                self.emit(abi::compare_immediate(
                    &matched.location,
                    &ordinal.to_string(),
                ));
                self.emit(abi::branch_eq(label));
            }
            NirValue::Local(variant) if self.type_model.union_variants.contains_key(variant) => {
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(variant)
                    .copied()
                    .ok_or_else(|| {
                        format!("native code union variant '{variant}' does not resolve")
                    })?;
                let tag_register = self.allocate_register()?;
                self.emit(abi::load_u64(&tag_register, &matched.location, 0));
                self.emit(abi::compare_immediate(&tag_register, &tag.to_string()));
                self.emit(abi::branch_eq(label));
            }
            _ => {
                let pattern = self.lower_value(pattern)?;
                self.emit(abi::compare_registers(&matched.location, &pattern.location));
                self.emit(abi::branch_eq(label));
            }
        }
        Ok(())
    }

    /// True when a `Result` payload of `payload_type` is a heap block addressed by
    /// pointer (inlined whole), versus an inline scalar (stored in the 8-byte
    /// payload word). Mirrors the record/collection inline rules (plan-02 §4.3).
    pub(super) fn result_payload_is_block(&self, payload_type: &str) -> bool {
        payload_type == "String"
            || payload_type == "Error"
            || is_collection_type(payload_type)
            || payload_type.starts_with("Result OF ")
            || self.type_model.record_fields.contains_key(payload_type)
            || self.type_model.union_names.contains(payload_type)
    }

}

fn split_top_level_types(params: &str) -> Vec<String> {
    if params.trim().is_empty() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in params.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(params[start..index].trim().to_string());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(params[start..].trim().to_string());
    parts
}
