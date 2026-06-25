use super::*;

impl CodeBuilder<'_> {
    pub(super) fn emit_symbol_call(&mut self, symbol: &str) {
        self.emit(abi::branch_link(symbol));
        let (binding, library) = if let Some(library) = self.platform_imports.get(symbol) {
            ("external".to_string(), Some(library.clone()))
        } else {
            ("internal".to_string(), None)
        };
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: "branch26".to_string(),
            binding,
            library,
        });
    }

    fn emit_prepared_call_args(
        &mut self,
        args: &[NirValue],
        slot_name: &str,
    ) -> Result<Vec<ValueResult>, String> {
        let mut arg_values = Vec::new();
        let mut arg_slots = Vec::new();
        for arg in args {
            let value = self.lower_value(arg)?;
            let slot = self.allocate_stack_object(slot_name, 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            arg_values.push(value);
            arg_slots.push(slot);
            self.reset_temporary_registers();
        }
        self.reset_temporary_registers();
        for (index, slot) in arg_slots.iter().enumerate() {
            self.emit(abi::load_u64("x9", abi::stack_pointer(), *slot));
            self.emit(abi::move_register(&abi::argument_register(index)?, "x9"));
        }
        Ok(arg_values)
    }

    pub(super) fn emit_raw_call(
        &mut self,
        symbol: &str,
        args: &[NirValue],
        slot_name: &str,
    ) -> Result<Vec<ValueResult>, String> {
        let arg_values = self.emit_prepared_call_args(args, slot_name)?;
        self.emit_symbol_call(symbol);
        Ok(arg_values)
    }

    pub(super) fn load_empty_string_constant(&mut self) -> Result<String, String> {
        let register = self.allocate_register()?;
        self.emit_load_static_string_symbol(&register, EMPTY_STRING_SYMBOL);
        Ok(register)
    }

    pub(super) fn load_string_constant(&mut self, value: &str) -> Result<String, String> {
        let register = self.allocate_register()?;
        self.emit_load_string_constant(&register, value)?;
        Ok(register)
    }

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
                let inline_string =
                    self.record_field_is_inlined(&target_value.type_, field_type);
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

        self.emit(abi::load_u64("x11", abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), right_slot));
        self.emit(abi::add_immediate("x13", "x11", 8));
        self.emit(abi::add_immediate("x15", "x12", 8));
        self.emit(abi::load_u64("x8", "x11", 0));
        self.emit(abi::load_u64("x9", "x12", 0));
        self.emit(abi::add_registers("x10", "x8", "x9"));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), total_slot));
        self.emit(abi::add_immediate(abi::return_register(), "x10", 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::load_u64("x11", abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64("x15", abi::stack_pointer(), right_slot));
        self.emit(abi::add_immediate("x15", "x15", 8));
        self.emit(abi::load_u64("x8", "x11", 0));
        self.emit(abi::add_immediate("x11", "x11", 8));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64("x9", "x9", 0));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), total_slot));
        self.emit(abi::store_u64("x10", "x1", 0));
        self.emit(abi::add_immediate("x12", "x1", 8));
        self.emit(abi::move_register("x14", "x8"));
        self.emit(abi::label(&left_loop));
        self.emit(abi::compare_immediate("x14", "0"));
        self.emit(abi::branch_eq(&left_done));
        self.emit(abi::load_u8("x16", "x11", 0));
        self.emit(abi::store_u8("x16", "x12", 0));
        self.emit(abi::add_immediate("x11", "x11", 1));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::subtract_immediate("x14", "x14", 1));
        self.emit(abi::branch(&left_loop));
        self.emit(abi::label(&left_done));
        self.emit(abi::move_register("x14", "x9"));
        self.emit(abi::label(&right_loop));
        self.emit(abi::compare_immediate("x14", "0"));
        self.emit(abi::branch_eq(&right_done));
        self.emit(abi::load_u8("x16", "x15", 0));
        self.emit(abi::store_u8("x16", "x12", 0));
        self.emit(abi::add_immediate("x15", "x15", 1));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::subtract_immediate("x14", "x14", 1));
        self.emit(abi::branch(&right_loop));
        self.emit(abi::label(&right_done));
        self.emit(abi::move_immediate("x16", "Integer", "0"));
        self.emit(abi::store_u8("x16", "x12", 0));

        Ok(ValueResult {
            type_: "String".to_string(),
            location: "x1".to_string(),
            text: format!("({} & {})", left.text, right.text),
        })
    }

    pub(super) fn emit_load_string_constant(
        &mut self,
        register: &str,
        value: &str,
    ) -> Result<(), String> {
        let symbol = self
            .string_symbols
            .get(value)
            .ok_or_else(|| format!("native code string literal '{value}' has no data object"))?
            .clone();
        self.emit(abi::load_page_address(register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(register, register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        Ok(())
    }

    pub(super) fn emit_load_static_string_symbol(&mut self, register: &str, symbol: &str) {
        self.emit(abi::load_page_address(register, symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(register, register, symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        });
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
            NirValue::Binary { op, left, right, .. } if op == "&" => {
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
            NirValue::Binary { op, left, right, .. } => {
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

    pub(super) fn emit_call(
        &mut self,
        target: &str,
        symbol: &str,
        args: &[NirValue],
        return_type: Option<&str>,
    ) -> Result<ValueResult, String> {
        let arg_values = self.emit_raw_call(symbol, args, "call_arg")?;
        let result_type = return_type
            .map(|type_| type_.to_string())
            .or_else(|| {
                self.functions
                    .get(target)
                    .map(|function| function.returns.clone())
            })
            .or_else(|| self.package_return_types.get(target).cloned())
            .unwrap_or_else(|| "Unknown".to_string());
        if result_type == "Nothing" {
            if return_type.is_none() {
                let ok_label = self.label("call_ok");
                self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
                self.emit(abi::branch_eq(&ok_label));
                self.emit_current_result_exit(self.error_exit_destination())?;
                self.emit(abi::label(&ok_label));
            }
            self.deactivate_moved_thread_arguments(target, args);
            self.deactivate_moved_resource_arguments(target, args);
            return Ok(ValueResult {
                type_: result_type,
                location: "void".to_string(),
                text: format!("call {target}({})", join_texts(&arg_values)),
            });
        }
        if return_type.is_none() {
            let ok_label = self.label("call_ok");
            self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&ok_label));
            self.emit_current_result_exit(self.error_exit_destination())?;
            self.emit(abi::label(&ok_label));
        }
        self.deactivate_moved_thread_arguments(target, args);
        self.deactivate_moved_resource_arguments(target, args);
        let register = self.allocate_register()?;
        self.emit(abi::move_register(&register, RESULT_VALUE_REGISTER));
        Ok(ValueResult {
            type_: result_type,
            location: register,
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    pub(super) fn emit_function_value_call(
        &mut self,
        target: &str,
        callable: &ValueResult,
        args: &[NirValue],
        return_type: Option<&str>,
    ) -> Result<ValueResult, String> {
        let arg_values = self.emit_prepared_call_args(args, "call_arg")?;
        let saved_env_slot = self.allocate_stack_object("closure_saved_env", 8);
        let code_register = self.allocate_register()?;
        let env_register = self.allocate_register()?;
        self.emit(abi::store_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
        self.emit(abi::load_u64(
            &code_register,
            &callable.location,
            CLOSURE_OFFSET_CODE,
        ));
        self.emit(abi::load_u64(
            &env_register,
            &callable.location,
            CLOSURE_OFFSET_ENV,
        ));
        self.emit(abi::move_register(CLOSURE_ENV_REGISTER, &env_register));
        self.emit(abi::branch_link_register(&code_register));
        self.emit(abi::load_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
        let result_type = return_type
            .map(|type_| type_.to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        if result_type == "Nothing" {
            if return_type.is_none() {
                let ok_label = self.label("call_value_ok");
                self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
                self.emit(abi::branch_eq(&ok_label));
                self.emit_current_result_exit(self.error_exit_destination())?;
                self.emit(abi::label(&ok_label));
            }
            for arg in args {
                self.maybe_deactivate_moved_thread_local(arg);
            }
            self.deactivate_moved_resource_arguments(target, args);
            return Ok(ValueResult {
                type_: result_type,
                location: "void".to_string(),
                text: format!("call {target}({})", join_texts(&arg_values)),
            });
        }
        if return_type.is_none() {
            let ok_label = self.label("call_value_ok");
            self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&ok_label));
            self.emit_current_result_exit(self.error_exit_destination())?;
            self.emit(abi::label(&ok_label));
        }
        for arg in args {
            self.maybe_deactivate_moved_thread_local(arg);
        }
        self.deactivate_moved_resource_arguments(target, args);
        let register = self.allocate_register()?;
        self.emit(abi::move_register(&register, RESULT_VALUE_REGISTER));
        Ok(ValueResult {
            type_: result_type,
            location: register,
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    pub(super) fn emit_runtime_helper_call(
        &mut self,
        target: &str,
        symbol: &str,
        args: &[NirValue],
        result_type: &str,
        raw: bool,
    ) -> Result<ValueResult, String> {
        if matches!(
            target,
            "thread.send" | "thread.emit" | "thread.transferResource" | "thread.emitResource"
        ) {
            return self.emit_thread_send_runtime_helper_call(
                target,
                symbol,
                args,
                result_type,
                raw,
            );
        }

        let arg_values = self.emit_raw_call(symbol, args, "runtime_call_arg")?;

        // An inline `TRAP` traps the raw `Result`: do not auto-propagate on
        // error; materialize the outcome (with the success value copied into the
        // current arena) for the trap to inspect. Owned handles/resources passed
        // to a consuming helper are consumed regardless of success or failure.
        if raw {
            self.deactivate_moved_thread_arguments(target, args);
            self.deactivate_moved_resource_arguments(target, args);
            let _ = arg_values;
            return self.materialize_current_result(
                result_type,
                format!("callResult {target}"),
                target == "thread.waitFor",
            );
        }

        let ok_label = self.label("runtime_call_ok");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // A runtime helper error originates at this call site: stamp the origin
        // before propagating so a trapped error reports the true location.
        // `thread.waitFor` instead propagates a worker's terminal error whose
        // origin (and message) must be deep-copied out of the worker arena before
        // the impending `thread.drop` cleanup frees it.
        if target == "thread.waitFor" {
            self.emit_finalize_worker_error_source()?;
        } else {
            self.emit_stamp_current_error_source()?;
        }
        self.emit_current_result_exit(self.error_exit_destination())?;
        self.emit(abi::label(&ok_label));
        self.deactivate_moved_thread_arguments(target, args);
        self.deactivate_moved_resource_arguments(target, args);

        if result_type == "Nothing" {
            return Ok(ValueResult {
                type_: result_type.to_string(),
                location: "void".to_string(),
                text: format!("call {target}({})", join_texts(&arg_values)),
            });
        }

        let register = if matches!(
            target,
            "thread.waitFor"
                | "thread.read"
                | "thread.receive"
                | "thread.acceptResource"
                | "thread.readResource"
        ) {
            self.reset_temporary_registers();
            self.copy_value_to_current_arena(result_type, RESULT_VALUE_REGISTER)?
        } else {
            let register = self.allocate_register()?;
            self.emit(abi::move_register(&register, RESULT_VALUE_REGISTER));
            register
        };
        Ok(ValueResult {
            type_: result_type.to_string(),
            location: register,
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    fn emit_thread_send_runtime_helper_call(
        &mut self,
        target: &str,
        symbol: &str,
        args: &[NirValue],
        result_type: &str,
        raw: bool,
    ) -> Result<ValueResult, String> {
        if args.len() < 2 {
            return Err(format!(
                "native runtime call '{target}' expects a handle and message"
            ));
        }
        let mut arg_values = Vec::new();
        let mut arg_slots = Vec::new();
        self.reset_temporary_registers();
        for arg in args {
            let value = self.lower_value(arg)?;
            let slot = self.allocate_stack_object("runtime_thread_send_arg", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            arg_values.push(value);
            arg_slots.push(slot);
            self.reset_temporary_registers();
        }

        self.reset_temporary_registers();
        let saved_arena_slot = self.allocate_stack_object("runtime_thread_send_saved_arena", 8);
        let copied_message_slot =
            self.allocate_stack_object("runtime_thread_send_copied_message", 8);
        let arena_offset = if target == "thread.emit" {
            THREAD_OFFSET_PARENT_ARENA_STATE
        } else {
            THREAD_OFFSET_ARENA_STATE
        };
        self.emit(abi::store_u64(
            ARENA_STATE_REGISTER,
            abi::stack_pointer(),
            saved_arena_slot,
        ));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), arg_slots[0]));
        self.emit(abi::load_u64("x10", "x9", arena_offset));
        self.emit(abi::move_register(ARENA_STATE_REGISTER, "x10"));
        self.error_arena_restore_slot = Some(saved_arena_slot);
        self.emit(abi::load_u64("x9", abi::stack_pointer(), arg_slots[1]));
        let copied = self.copy_value_to_current_arena(&arg_values[1].type_, "x9")?;
        self.error_arena_restore_slot = None;
        self.emit(abi::store_u64(
            &copied,
            abi::stack_pointer(),
            copied_message_slot,
        ));
        self.reset_temporary_registers();
        self.emit(abi::load_u64(
            ARENA_STATE_REGISTER,
            abi::stack_pointer(),
            saved_arena_slot,
        ));
        self.emit(abi::load_u64(
            "x9",
            abi::stack_pointer(),
            copied_message_slot,
        ));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), arg_slots[1]));

        for (index, slot) in arg_slots.iter().enumerate() {
            self.emit(abi::load_u64("x9", abi::stack_pointer(), *slot));
            self.emit(abi::move_register(&abi::argument_register(index)?, "x9"));
        }
        self.emit_symbol_call(symbol);

        // An inline `TRAP` traps the raw send `Result`. On failure the sent value
        // remains owned by the caller (the typechecker restores the binding into
        // the handler scope); the success continuation treats it as moved.
        if raw {
            self.deactivate_moved_thread_arguments(target, args);
            self.deactivate_moved_resource_arguments(target, args);
            let _ = arg_values;
            // thread.send/emit errors originate at this call site, not a worker.
            return self.materialize_current_result(
                result_type,
                format!("callResult {target}"),
                false,
            );
        }

        let ok_label = self.label("runtime_thread_send_ok");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit_stamp_current_error_source()?;
        self.emit_current_result_exit(self.error_exit_destination())?;
        self.emit(abi::label(&ok_label));
        self.deactivate_moved_thread_arguments(target, args);
        self.deactivate_moved_resource_arguments(target, args);

        if result_type != "Nothing" {
            return Err(format!(
                "native runtime call '{target}' expected Nothing result, got '{result_type}'"
            ));
        }
        Ok(ValueResult {
            type_: result_type.to_string(),
            location: "void".to_string(),
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    pub(super) fn materialize_current_result(
        &mut self,
        success_type: &str,
        text: String,
        // When true (an inline-trapped `thread::waitFor`), the error's message and
        // origin live in the worker arena and arrive in x2/x3; they are deep-copied
        // into the caller arena. Otherwise the error originates at this inline
        // expression and its `ErrorLoc` is built from the current source location.
        worker_error_source: bool,
    ) -> Result<ValueResult, String> {
        let tag_slot = self.allocate_stack_object("raw_result_tag", 8);
        let value_slot = self.allocate_stack_object("raw_result_value", 8);
        let message_slot = self.allocate_stack_object("raw_result_message", 8);
        let source_raw_slot = self.allocate_stack_object("raw_result_source_raw", 8);
        let payload_slot = self.allocate_stack_object("raw_result_payload", 8);
        let result_slot = self.allocate_stack_object("raw_result", 8);
        let alloc_ok = self.label("result_construct_alloc_ok");
        let error_alloc_ok = self.label("result_error_alloc_ok");
        let wrap_error_label = self.label("result_wrap_error");
        let have_payload_label = self.label("result_have_payload");

        self.emit(abi::store_u64(
            RESULT_TAG_REGISTER,
            abi::stack_pointer(),
            tag_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            value_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_raw_slot,
        ));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), tag_slot));
        self.emit(abi::compare_immediate("x9", RESULT_OK_TAG));
        self.emit(abi::branch_ne(&wrap_error_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), value_slot));
        let copied_success = self.copy_value_to_current_arena(success_type, "x9")?;
        self.emit(abi::store_u64(
            &copied_success,
            abi::stack_pointer(),
            payload_slot,
        ));
        self.emit(abi::branch(&have_payload_label));

        self.emit(abi::label(&wrap_error_label));
        let source_slot = self.allocate_stack_object("raw_result_source", 8);
        if worker_error_source {
            // A propagated worker error: deep-copy its message and origin out of
            // the (still-alive) worker arena into the caller arena. If the helper
            // raised its own error (source == 0), stamp this inline expression.
            self.emit(abi::load_u64("x9", abi::stack_pointer(), message_slot));
            let copied_message = self.copy_value_to_current_arena("String", "x9")?;
            self.emit(abi::store_u64(
                &copied_message,
                abi::stack_pointer(),
                message_slot,
            ));
            let own = self.label("raw_worker_error_own");
            let done = self.label("raw_worker_error_done");
            self.emit(abi::load_u64("x9", abi::stack_pointer(), source_raw_slot));
            self.emit(abi::compare_immediate("x9", "0"));
            self.emit(abi::branch_eq(&own));
            let copied_source = self.copy_value_to_current_arena("ErrorLoc", "x9")?;
            self.emit(abi::store_u64(&copied_source, abi::stack_pointer(), source_slot));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&own));
            let loc = self.emit_build_error_loc()?;
            self.emit(abi::store_u64(&loc, abi::stack_pointer(), source_slot));
            self.emit(abi::label(&done));
        } else {
            // The error originates at the current inline expression.
            let loc_register = self.emit_build_error_loc()?;
            self.emit(abi::store_u64(&loc_register, abi::stack_pointer(), source_slot));
        }
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &ERROR_OBJECT_SIZE.to_string(),
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&error_alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&error_alloc_ok));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), value_slot));
        self.emit(abi::store_u64("x9", "x1", 0));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), message_slot));
        self.emit(abi::store_u64("x9", "x1", 8));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64("x9", "x1", 16));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), payload_slot));

        self.emit(abi::label(&have_payload_label));
        self.emit(abi::move_immediate(abi::return_register(), "Integer", "16"));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), tag_slot));
        self.emit(abi::store_u64("x9", "x1", 0));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), payload_slot));
        self.emit(abi::store_u64("x9", "x1", 8));

        let register = self.allocate_register()?;
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: format!("Result OF {success_type}"),
            location: register,
            text,
        })
    }

    pub(super) fn copy_value_to_current_arena(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
        match type_ {
            "Nothing" | "Boolean" | "Byte" | "Integer" | "Float" | "Fixed" => {
                let result = self.allocate_register()?;
                self.emit(abi::move_register(&result, source));
                Ok(result)
            }
            // A standalone `String` is already a flat, pointer-free block
            // (length word + bytes + NUL), so the generic flat copy is a sound
            // deep copy (plan-02 §4.1, Phase 1).
            "String" => self.copy_flat_block("String", source),
            "Error" => self.copy_error_to_current_arena(source),
            other if other.starts_with("Result OF ") => {
                self.copy_result_to_current_arena(other, source)
            }
            other if is_collection_type(other) => {
                self.copy_collection_to_current_arena(other, source)
            }
            other if crate::builtins::is_thread_sendable_resource_type(other) => {
                self.copy_resource_to_current_arena(source)
            }
            other => {
                if self.type_model.record_fields.contains_key(other) {
                    return self.copy_record_to_current_arena(other, source);
                }
                if self.type_model.union_names.contains(other) {
                    return self.copy_union_to_current_arena(other, source);
                }
                Err(format!(
                    "native thread transfer cannot copy value of type '{other}'"
                ))
            }
        }
    }

    fn copy_error_to_current_arena(&mut self, source: &str) -> Result<String, String> {
        let source_slot = self.allocate_stack_object("thread_copy_error_source", 8);
        let result_slot = self.allocate_stack_object("thread_copy_error_result", 8);
        let alloc_ok = self.label("thread_copy_error_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &ERROR_OBJECT_SIZE.to_string(),
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        let field_slot = self.allocate_stack_object("thread_copy_error_field", 8);
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        // code: copied directly.
        self.emit(abi::load_u64("x10", "x9", 0));
        self.emit(abi::store_u64("x10", "x1", 0));
        // message: deep-copied String into the destination arena.
        self.emit(abi::load_u64("x10", "x9", 8));
        let copied_message = self.copy_value_to_current_arena("String", "x10")?;
        self.emit(abi::store_u64(&copied_message, abi::stack_pointer(), field_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), field_slot));
        self.emit(abi::store_u64("x10", "x9", 8));
        // source: deep-copied ErrorLoc record (its filename String comes along).
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x10", "x9", 16));
        let copied_source = self.copy_value_to_current_arena("ErrorLoc", "x10")?;
        self.emit(abi::store_u64(&copied_source, abi::stack_pointer(), field_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), field_slot));
        self.emit(abi::store_u64("x10", "x9", 16));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// Materialize a thread-sendable resource handle (e.g. `File`) into the
    /// current arena. The handle is a two-word struct (a host resource word
    /// such as a file descriptor, followed by a closed flag); moving it copies
    /// both words so the receiver owns the underlying OS resource. The sender's
    /// lexical cleanup is deactivated on the successful-transfer path, so the
    /// resource is closed exactly once by the receiver.
    fn copy_resource_to_current_arena(&mut self, source: &str) -> Result<String, String> {
        let source_slot = self.allocate_stack_object("thread_copy_resource_source", 8);
        let result_slot = self.allocate_stack_object("thread_copy_resource_result", 8);
        let alloc_ok = self.label("thread_copy_resource_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            RESOURCE_RECORD_SIZE,
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x10", "x9", 0));
        self.emit(abi::store_u64("x10", "x1", 0));
        self.emit(abi::load_u64("x10", "x9", 8));
        self.emit(abi::store_u64("x10", "x1", 8));
        self.emit(abi::load_u64("x10", "x9", FILE_OFFSET_STATE));
        self.emit(abi::store_u64("x10", "x1", FILE_OFFSET_STATE));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    fn copy_result_to_current_arena(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
        let success_type = type_
            .strip_prefix("Result OF ")
            .ok_or_else(|| {
                format!("native thread transfer result type '{type_}' does not resolve")
            })?
            .to_string();
        let source_slot = self.allocate_stack_object("thread_copy_result_source", 8);
        let tag_slot = self.allocate_stack_object("thread_copy_result_tag", 8);
        let payload_slot = self.allocate_stack_object("thread_copy_result_payload", 8);
        let result_slot = self.allocate_stack_object("thread_copy_result_result", 8);
        let alloc_ok = self.label("thread_copy_result_alloc_ok");
        let copy_error = self.label("thread_copy_result_error");
        let have_payload = self.label("thread_copy_result_have_payload");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x9", source, 0));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), tag_slot));
        self.emit(abi::compare_immediate("x9", RESULT_OK_TAG));
        self.emit(abi::branch_ne(&copy_error));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x10", "x9", 8));
        let copied_success = self.copy_value_to_current_arena(&success_type, "x10")?;
        self.emit(abi::store_u64(
            &copied_success,
            abi::stack_pointer(),
            payload_slot,
        ));
        self.emit(abi::branch(&have_payload));

        self.emit(abi::label(&copy_error));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x10", "x9", 8));
        let copied_error = self.copy_value_to_current_arena("Error", "x10")?;
        self.emit(abi::store_u64(
            &copied_error,
            abi::stack_pointer(),
            payload_slot,
        ));

        self.emit(abi::label(&have_payload));
        self.emit(abi::move_immediate(abi::return_register(), "Integer", "16"));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), tag_slot));
        self.emit(abi::store_u64("x9", "x1", 0));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), payload_slot));
        self.emit(abi::store_u64("x9", "x1", 8));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// True when field `field_type` of `record_type` is a pointer to a separate
    /// allocation that a whole-block `memcpy` would alias and must therefore be
    /// deep-copied. Inlined fields (`String` and fully-flat nested records) come
    /// along with the block copy; only still-pointer composites (`Union`/`List`/
    /// `Map`/`Result`/`Error`, a not-yet-flat nested record) and the built-in
    /// pointer-`String` records' `String` fields need the fix.
    fn record_field_is_pointer_in(&self, record_type: &str, field_type: &str) -> bool {
        if self.record_field_is_inlined(record_type, field_type) {
            return false;
        }
        field_type == "String" || self.record_field_is_pointer(field_type)
    }

    fn record_needs_pointer_field_fix(&self, record_type: &str) -> bool {
        self.type_model
            .record_fields
            .get(record_type)
            .map(|fields| {
                fields
                    .iter()
                    .any(|(_, ft)| self.record_field_is_pointer_in(record_type, ft))
            })
            .unwrap_or(false)
    }

    fn copy_record_to_current_arena(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
        let fields = self
            .type_model
            .record_fields
            .get(type_)
            .cloned()
            .ok_or_else(|| {
                format!("native thread transfer record type '{type_}' does not resolve")
            })?;
        let source_slot = self.allocate_stack_object("thread_copy_record_source", 8);
        let size_slot = self.allocate_stack_object("thread_copy_record_size", 8);
        let result_slot = self.allocate_stack_object("thread_copy_record_result", 8);
        let alloc_ok = self.label("thread_copy_record_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        // The record's flat block (fixed slots + inlined String data) is
        // self-describing; size it, allocate, and copy the whole block. Inlined
        // String fields come along verbatim because their offsets are
        // block-relative (plan-02 §4.1/§4.2).
        self.emit_record_block_size_to_slot(type_, source_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), size_slot));
        self.emit_copy_bytes("x1", "x9", "x10", "thread_copy_record_block");
        // Deep-copy any pointer fields so the copy shares nothing.
        let copied_slot = self.allocate_stack_object("thread_copy_record_field", 8);
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if !self.record_field_is_pointer_in(type_, field_type) {
                continue;
            }
            self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
            self.emit(abi::load_u64("x10", "x9", index * 8));
            let copied = self.copy_value_to_current_arena(field_type, "x10")?;
            // Stash the copied value before reloading the result pointer into x9:
            // `copied` is an allocated register that may itself be x9.
            self.emit(abi::store_u64(&copied, abi::stack_pointer(), copied_slot));
            self.emit(abi::load_u64("x9", abi::stack_pointer(), result_slot));
            self.emit(abi::load_u64("x10", abi::stack_pointer(), copied_slot));
            self.emit(abi::store_u64("x10", "x9", index * 8));
        }
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    fn copy_union_to_current_arena(&mut self, type_: &str, source: &str) -> Result<String, String> {
        let source_slot = self.allocate_stack_object("thread_copy_union_source", 8);
        let size_slot = self.allocate_stack_object("thread_copy_union_size", 8);
        let result_slot = self.allocate_stack_object("thread_copy_union_result", 8);
        let alloc_ok = self.label("thread_copy_union_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        // A data union is `{tag, size, variant-record-block}`: its total size is
        // the runtime `size` word at +8 (plan-02 §4.3). A resource union is the
        // fixed `{tag, resource-ptr}` block.
        if self.union_is_data(type_) {
            self.emit_data_union_size_to_slot(source_slot, size_slot);
        } else {
            let size = self.inline_collection_payload_size(type_).ok_or_else(|| {
                format!("native thread transfer union type '{type_}' does not resolve")
            })?;
            self.emit(abi::move_immediate("x8", "Integer", &size.to_string()));
            self.emit(abi::store_u64("x8", abi::stack_pointer(), size_slot));
        }
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), size_slot));
        self.emit_copy_bytes("x1", "x9", "x13", "thread_copy_union_raw");
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), result_slot));
        self.copy_union_fields_into_existing(type_, "x9", "x10")?;
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// True when a collection of `type_` embeds pointer payloads (nested
    /// collections, records, unions, `Result`/`Error`) that a plain byte copy
    /// would alias rather than deep-copy, so the per-payload transfer fix is
    /// still required. A collection whose key/value payloads are all inline
    /// (scalars, `String`) is already flat and copies generically.
    fn collection_needs_transfer_fix(&self, type_: &str) -> Result<bool, String> {
        let (key_type, value_type) = if let Some(value_type) = type_.strip_prefix("List OF ") {
            (None, value_type.to_string())
        } else {
            let (key, value) = map_type_parts(type_).ok_or_else(|| {
                format!("native thread transfer collection type '{type_}' does not resolve")
            })?;
            (Some(key), value)
        };
        if let Some(key_type) = key_type.as_deref() {
            if self.collection_payload_needs_transfer_fix(key_type) {
                return Ok(true);
            }
        }
        Ok(self.collection_payload_needs_transfer_fix(&value_type))
    }

    fn copy_collection_to_current_arena(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
        // A collection with only inline payloads is a flat, pointer-free block:
        // copy it with the generic flat copy (plan-02 §4.1, Phase 1). Only
        // collections embedding pointer payloads keep the per-payload fix below.
        if !self.collection_needs_transfer_fix(type_)? {
            return self.copy_flat_block(type_, source);
        }
        let source_slot = self.allocate_stack_object("thread_copy_collection_source", 8);
        let size_slot = self.allocate_stack_object("thread_copy_collection_size", 8);
        let result_slot = self.allocate_stack_object("thread_copy_collection_result", 8);
        let alloc_ok = self.label("thread_copy_collection_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x9", source, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::move_immediate(
            "x10",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x9", "x9", "x10"));
        self.emit(abi::add_immediate("x9", "x9", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x10", "x10", COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::add_registers("x9", "x9", "x10"));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), size_slot));
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), size_slot));
        self.emit_copy_bytes("x1", "x9", "x10", "thread_copy_collection");
        self.fix_collection_transfer_payloads(type_, source_slot, result_slot)?;
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    fn fix_collection_transfer_payloads(
        &mut self,
        type_: &str,
        source_slot: usize,
        result_slot: usize,
    ) -> Result<(), String> {
        let (key_type, value_type) = if let Some(value_type) = type_.strip_prefix("List OF ") {
            (None, value_type.to_string())
        } else {
            let (key, value) = map_type_parts(type_).ok_or_else(|| {
                format!("native thread transfer collection type '{type_}' does not resolve")
            })?;
            (Some(key), value)
        };
        if let Some(key_type) = key_type.as_deref() {
            if self.collection_payload_needs_transfer_fix(key_type) {
                self.fix_collection_transfer_payload(source_slot, result_slot, key_type, true)?;
            }
        }
        if self.collection_payload_needs_transfer_fix(&value_type) {
            self.fix_collection_transfer_payload(source_slot, result_slot, &value_type, false)?;
        }
        Ok(())
    }

    fn collection_payload_needs_transfer_fix(&self, type_: &str) -> bool {
        if self.type_model.record_fields.contains_key(type_) {
            // A record payload was byte-copied whole (inlined String fields came
            // along); it only needs the per-payload fix if it still has pointer
            // fields to deep-copy (plan-02 §4.2).
            return self.record_needs_pointer_field_fix(type_);
        }
        is_collection_type(type_)
            || self.type_model.union_names.contains(type_)
            || type_.starts_with("Result OF ")
            || type_ == "Error"
    }

    fn fix_collection_transfer_payload(
        &mut self,
        source_slot: usize,
        result_slot: usize,
        payload_type: &str,
        key_payload: bool,
    ) -> Result<(), String> {
        let index_slot = self.allocate_stack_object("thread_copy_collection_index", 8);
        let source_entry_slot =
            self.allocate_stack_object("thread_copy_collection_source_entry", 8);
        let dest_entry_slot = self.allocate_stack_object("thread_copy_collection_dest_entry", 8);
        let source_payload_slot =
            self.allocate_stack_object("thread_copy_collection_source_payload", 8);
        let dest_payload_slot =
            self.allocate_stack_object("thread_copy_collection_dest_payload", 8);
        let loop_label = self.label("thread_copy_collection_fix_loop");
        let next_label = self.label("thread_copy_collection_fix_next");
        let done_label = self.label("thread_copy_collection_fix_done");
        let entry_offset = if key_payload {
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET
        };
        self.emit(abi::move_immediate("x9", "Integer", "0"));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), index_slot));
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x10", "x9", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::compare_registers("x8", "x10"));
        self.emit(abi::branch_ge(&done_label));

        self.emit(abi::move_immediate(
            "x10",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x11", "x8", "x10"));
        self.emit(abi::add_immediate("x11", "x11", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::add_registers("x12", "x9", "x11"));
        self.emit(abi::store_u64(
            "x12",
            abi::stack_pointer(),
            source_entry_slot,
        ));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), result_slot));
        self.emit(abi::add_registers("x12", "x9", "x11"));
        self.emit(abi::store_u64("x12", abi::stack_pointer(), dest_entry_slot));
        self.emit(abi::load_u8("x9", "x12", COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::compare_immediate(
            "x9",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::branch_ne(&next_label));

        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit_collection_data_pointer("x10", "x9");
        self.emit(abi::load_u64(
            "x11",
            abi::stack_pointer(),
            source_entry_slot,
        ));
        self.emit(abi::load_u64("x12", "x11", entry_offset));
        self.emit(abi::add_registers("x10", "x10", "x12"));
        self.emit(abi::store_u64(
            "x10",
            abi::stack_pointer(),
            source_payload_slot,
        ));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer("x10", "x9");
        self.emit(abi::load_u64("x11", abi::stack_pointer(), dest_entry_slot));
        self.emit(abi::load_u64("x12", "x11", entry_offset));
        self.emit(abi::add_registers("x10", "x10", "x12"));
        self.emit(abi::store_u64(
            "x10",
            abi::stack_pointer(),
            dest_payload_slot,
        ));

        if is_collection_type(payload_type)
            || payload_type.starts_with("Result OF ")
            || payload_type == "Error"
        {
            self.emit(abi::load_u64(
                "x9",
                abi::stack_pointer(),
                source_payload_slot,
            ));
            self.emit(abi::load_u64("x10", "x9", 0));
            let copied = self.copy_value_to_current_arena(payload_type, "x10")?;
            // Stash before reloading the destination pointer: `copied` may be x9.
            let payload_copied_slot = self.allocate_stack_object("thread_copy_payload_field", 8);
            self.emit(abi::store_u64(&copied, abi::stack_pointer(), payload_copied_slot));
            self.emit(abi::load_u64("x9", abi::stack_pointer(), dest_payload_slot));
            self.emit(abi::load_u64("x10", abi::stack_pointer(), payload_copied_slot));
            self.emit(abi::store_u64("x10", "x9", 0));
        } else if self.type_model.record_fields.contains_key(payload_type) {
            self.emit(abi::load_u64(
                "x9",
                abi::stack_pointer(),
                source_payload_slot,
            ));
            self.emit(abi::load_u64(
                "x10",
                abi::stack_pointer(),
                dest_payload_slot,
            ));
            self.copy_record_fields_into_existing(payload_type, "x9", "x10")?;
        } else if self.type_model.union_names.contains(payload_type) {
            self.emit(abi::load_u64(
                "x9",
                abi::stack_pointer(),
                source_payload_slot,
            ));
            self.emit(abi::load_u64(
                "x10",
                abi::stack_pointer(),
                dest_payload_slot,
            ));
            self.copy_union_fields_into_existing(payload_type, "x9", "x10")?;
        }

        self.emit(abi::label(&next_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), index_slot));
        self.emit(abi::add_immediate("x9", "x9", 1));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), index_slot));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    fn copy_record_fields_into_existing(
        &mut self,
        type_: &str,
        source: &str,
        destination: &str,
    ) -> Result<(), String> {
        let fields = self
            .type_model
            .record_fields
            .get(type_)
            .cloned()
            .ok_or_else(|| {
                format!("native thread transfer record type '{type_}' does not resolve")
            })?;
        let source_slot = self.allocate_stack_object("thread_copy_record_inline_source", 8);
        let destination_slot =
            self.allocate_stack_object("thread_copy_record_inline_destination", 8);
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(
            destination,
            abi::stack_pointer(),
            destination_slot,
        ));
        // The whole record block was already byte-copied into `destination`
        // (inlined String fields came along). Only deep-copy pointer fields so
        // the copy aliases nothing (plan-02 §4.2).
        let copied_slot = self.allocate_stack_object("thread_copy_into_field", 8);
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if !self.record_field_is_pointer_in(type_, field_type) {
                continue;
            }
            self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
            self.emit(abi::load_u64("x10", "x9", index * 8));
            let copied = self.copy_value_to_current_arena(field_type, "x10")?;
            // Stash before reloading the destination pointer: `copied` may be x9.
            self.emit(abi::store_u64(&copied, abi::stack_pointer(), copied_slot));
            self.emit(abi::load_u64("x9", abi::stack_pointer(), destination_slot));
            self.emit(abi::load_u64("x10", abi::stack_pointer(), copied_slot));
            self.emit(abi::store_u64("x10", "x9", index * 8));
        }
        Ok(())
    }

    fn copy_union_fields_into_existing(
        &mut self,
        type_: &str,
        source: &str,
        destination: &str,
    ) -> Result<(), String> {
        let mut variants = self
            .type_model
            .variants_for_union(type_)
            .map(|variant| {
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(variant)
                    .copied()
                    .ok_or_else(|| {
                        format!("native thread transfer union variant '{variant}' has no tag")
                    })?;
                let fields = self
                    .type_model
                    .union_variant_fields
                    .get(variant)
                    .cloned()
                    .unwrap_or_default();
                Ok((variant.clone(), tag, fields))
            })
            .collect::<Result<Vec<_>, String>>()?;
        variants.sort_by_key(|(_, tag, _)| *tag);
        let source_slot = self.allocate_stack_object("thread_copy_union_inline_source", 8);
        let destination_slot =
            self.allocate_stack_object("thread_copy_union_inline_destination", 8);
        let done_label = self.label("thread_copy_union_inline_done");
        let fallback_label = self.label("thread_copy_union_inline_fallback");
        let labels = variants
            .iter()
            .map(|(variant, _, _)| {
                (
                    variant.clone(),
                    self.label("thread_copy_union_inline_variant"),
                )
            })
            .collect::<HashMap<_, _>>();
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(
            destination,
            abi::stack_pointer(),
            destination_slot,
        ));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x10", "x9", 0));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), destination_slot));
        self.emit(abi::store_u64("x10", "x9", 0));
        for (variant, tag, _) in &variants {
            self.emit(abi::compare_immediate("x10", &tag.to_string()));
            self.emit(abi::branch_eq(&labels[variant]));
        }
        self.emit(abi::branch(&fallback_label));
        let is_data_union = self.union_is_data(type_);
        let union_copied_slot = self.allocate_stack_object("thread_copy_union_field", 8);
        for (variant, _, fields) in &variants {
            self.emit(abi::label(&labels[variant]));
            if is_data_union {
                // The active variant's flat record block was byte-copied at +16
                // by the whole-union memcpy; deep-copy only its pointer fields so
                // the union copy aliases nothing (plan-02 §4.3).
                self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
                self.emit(abi::add_immediate("x9", "x9", 16));
                self.emit(abi::load_u64("x10", abi::stack_pointer(), destination_slot));
                self.emit(abi::add_immediate("x10", "x10", 16));
                self.copy_record_fields_into_existing(variant, "x9", "x10")?;
                self.emit(abi::branch(&done_label));
                continue;
            }
            for (index, (_, field_type)) in fields.iter().enumerate() {
                self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
                self.emit(abi::load_u64("x10", "x9", 8 * (index + 1)));
                let copied = self.copy_value_to_current_arena(field_type, "x10")?;
                // Stash before reloading the destination pointer: `copied` may be x9.
                self.emit(abi::store_u64(&copied, abi::stack_pointer(), union_copied_slot));
                self.emit(abi::load_u64("x9", abi::stack_pointer(), destination_slot));
                self.emit(abi::load_u64("x10", abi::stack_pointer(), union_copied_slot));
                self.emit(abi::store_u64("x10", "x9", 8 * (index + 1)));
            }
            self.emit(abi::branch(&done_label));
        }
        self.emit(abi::label(&fallback_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    pub(super) fn allocate_register(&mut self) -> Result<String, String> {
        let register = abi::temporary_register(self.next_register).map_err(|err| {
            format!(
                "{err} while lowering native function '{}'",
                self.current_symbol
            )
        })?;
        self.next_register += 1;
        self.mark_register_used(&register);
        Ok(register)
    }

    pub(super) fn mark_register_used(&mut self, register: &str) {
        if abi::is_callee_saved(register)
            && !self.used_callee_saved.iter().any(|saved| saved == register)
        {
            self.used_callee_saved.push(register.to_string());
        }
    }

    pub(super) fn reset_temporary_registers(&mut self) {
        self.next_register = 8;
    }

    pub(super) fn local_constants(&self) -> HashMap<String, Option<NirValue>> {
        self.locals
            .iter()
            .map(|(name, local)| (name.clone(), local.constant.clone()))
            .collect()
    }

    pub(super) fn restore_local_constants(
        &mut self,
        constants: &HashMap<String, Option<NirValue>>,
    ) {
        for (name, local) in &mut self.locals {
            local.constant = constants.get(name).cloned().unwrap_or(None);
        }
    }

    pub(super) fn clear_local_constants(&mut self) {
        for local in self.locals.values_mut() {
            local.constant = None;
        }
    }

    pub(super) fn allocate_stack_object(&mut self, name: &str, size: usize) -> usize {
        let offset = self.stack_size;
        let size = align(size, 8);
        self.stack_size += size;
        self.stack_slots.push(CodeStackSlot {
            name: format!("{name}_{}", self.stack_slots.len()),
            type_: name.to_string(),
            offset: offset as i32,
        });
        offset
    }

    pub(super) fn label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}_{}", self.next_label);
        self.next_label += 1;
        label
    }

    pub(super) fn emit(&mut self, instruction: CodeInstruction) {
        self.instructions.push(instruction);
    }

    pub(super) fn emit_overflow_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_OVERFLOW_CODE, ERR_OVERFLOW_MESSAGE)
    }

    pub(super) fn emit_underflow_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_UNDERFLOW_CODE, ERR_UNDERFLOW_MESSAGE)
    }

    pub(super) fn emit_float_domain_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_FLOAT_DOMAIN_CODE, ERR_FLOAT_DOMAIN_MESSAGE)
    }

    pub(super) fn emit_float_nan_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_FLOAT_NAN_CODE, ERR_FLOAT_NAN_MESSAGE)
    }

    pub(super) fn emit_float_inf_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_FLOAT_INF_CODE, ERR_FLOAT_INF_MESSAGE)
    }

    pub(super) fn emit_float_overflow_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_FLOAT_OVERFLOW_CODE, ERR_FLOAT_OVERFLOW_MESSAGE)
    }

    pub(super) fn emit_invalid_argument_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_MESSAGE)
    }

    pub(super) fn emit_invalid_format_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_INVALID_FORMAT_CODE, ERR_INVALID_FORMAT_MESSAGE)
    }

    pub(super) fn emit_allocation_error_return(&mut self) -> Result<(), String> {
        self.emit_error_register_return(RESULT_TAG_REGISTER, ERR_ALLOCATION_MESSAGE)
    }

    pub(super) fn emit_index_out_of_range_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_INDEX_OUT_OF_RANGE_CODE, ERR_INDEX_OUT_OF_RANGE_MESSAGE)
    }

    pub(super) fn emit_not_found_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_NOT_FOUND_CODE, ERR_NOT_FOUND_MESSAGE)
    }

    pub(super) fn emit_encoding_error_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_ENCODING_CODE, ERR_ENCODING_MESSAGE)
    }

    pub(super) fn emit_error_code_return(
        &mut self,
        code: &str,
        message: &str,
    ) -> Result<(), String> {
        let code_register = self.allocate_register()?;
        self.emit(abi::move_immediate(&code_register, "Integer", code));
        self.emit_error_register_return(&code_register, message)
    }

    /// Build an `ErrorLoc` record for the current source location and return a
    /// register holding its pointer. The pointer is left null only when the
    /// allocation itself fails (OOM), where no `ErrorLoc` could be allocated
    /// regardless. This never routes back through the error-return path, so it is
    /// safe to call from `emit_error_register_return`.
    /// Allocation-free: uses only the `x9` scratch register and stack slots, and
    /// returns the pointer in `x9`. Error-emitting paths are terminal, so they
    /// must not consume the temporary-register pool (the surrounding expression
    /// may already be near the physical-register limit). Callers must save any
    /// live register inputs to the stack before invoking this.
    pub(super) fn emit_build_error_loc(&mut self) -> Result<String, String> {
        let result_slot = self.allocate_stack_object("error_loc_result", 8);
        let filename_slot = self.allocate_stack_object("error_loc_filename", 8);
        let alloc_ok = self.label("error_loc_alloc_ok");
        let done = self.label("error_loc_done");
        // Default the result to a null pointer for the OOM fall-through path.
        self.emit(abi::move_immediate("x9", "Integer", "0"));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), result_slot));
        // Resolve the filename string pointer before the allocation call clobbers
        // caller-saved registers.
        let filename = self.current_file.clone();
        if filename.is_empty() {
            self.emit(abi::move_immediate("x9", "Integer", "0"));
        } else {
            self.emit_load_string_constant("x9", &filename)?;
        }
        self.emit(abi::store_u64("x9", abi::stack_pointer(), filename_slot));
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &ERROR_LOC_OBJECT_SIZE.to_string(),
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&alloc_ok));
        // x1 holds the new ErrorLoc pointer.
        self.emit(abi::load_u64("x9", abi::stack_pointer(), filename_slot));
        self.emit(abi::store_u64("x9", "x1", 0));
        self.emit(abi::move_immediate(
            "x9",
            "Integer",
            &self.current_loc.line.to_string(),
        ));
        self.emit(abi::store_u64("x9", "x1", 8));
        self.emit(abi::move_immediate(
            "x9",
            "Integer",
            &self.current_loc.column.to_string(),
        ));
        self.emit(abi::store_u64("x9", "x1", 16));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::label(&done));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), result_slot));
        Ok("x9".to_string())
    }

    /// Finalize a `thread::waitFor` error so it survives the worker arena being
    /// freed by the impending `thread.drop` cleanup. A propagated worker error
    /// arrives with its origin `ErrorLoc` in `x3` and its message in `x2`, both
    /// living in the worker arena which is still alive at this point — so they are
    /// deep-copied into the caller arena here. `waitFor`'s own errors arrive with
    /// `x3 == 0` (their message is a static string) and are stamped with this call
    /// site. All raw inputs are saved to the stack first because every copy/alloc
    /// clobbers the caller-saved registers.
    pub(super) fn emit_finalize_worker_error_source(&mut self) -> Result<(), String> {
        let code_slot = self.allocate_stack_object("worker_error_code", 8);
        let message_raw_slot = self.allocate_stack_object("worker_error_message_raw", 8);
        let source_raw_slot = self.allocate_stack_object("worker_error_source_raw", 8);
        let message_slot = self.allocate_stack_object("worker_error_message", 8);
        let source_slot = self.allocate_stack_object("worker_error_source", 8);
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_raw_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_raw_slot,
        ));
        // Deep-copy the message into the caller arena.
        self.emit(abi::load_u64("x9", abi::stack_pointer(), message_raw_slot));
        let copied_message = self.copy_value_to_current_arena("String", "x9")?;
        self.emit(abi::store_u64(
            &copied_message,
            abi::stack_pointer(),
            message_slot,
        ));
        // Deep-copy the worker source `ErrorLoc`, or stamp the call site if the
        // error originated in `waitFor` itself (no worker origin).
        let own = self.label("worker_error_own_origin");
        let done = self.label("worker_error_source_done");
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_raw_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&own));
        let copied_source = self.copy_value_to_current_arena("ErrorLoc", "x9")?;
        self.emit(abi::store_u64(&copied_source, abi::stack_pointer(), source_slot));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&own));
        let loc = self.emit_build_error_loc()?;
        self.emit(abi::store_u64(&loc, abi::stack_pointer(), source_slot));
        self.emit(abi::label(&done));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        self.emit(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_slot,
        ));
        Ok(())
    }

    /// Stamp the current source location into the error-source register for an
    /// error that a native runtime helper just returned in the standard error
    /// registers. The helper sets code (x1) and message (x2) but not the origin,
    /// so the call site (whose location is in `self.current_loc`) supplies it.
    /// The error code/message are preserved across the `ErrorLoc` allocation.
    pub(super) fn emit_stamp_current_error_source(&mut self) -> Result<(), String> {
        let code_slot = self.allocate_stack_object("error_source_code", 8);
        let message_slot = self.allocate_stack_object("error_source_message", 8);
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        let loc_register = self.emit_build_error_loc()?;
        self.emit(abi::move_register(
            RESULT_ERROR_SOURCE_REGISTER,
            &loc_register,
        ));
        // Building the ErrorLoc allocates, which clobbers the tag register (x0):
        // re-assert the error tag along with the restored code/message.
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        self.emit(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        let _ = loc_register;
        Ok(())
    }

    pub(super) fn emit_error_register_return(
        &mut self,
        code_register: &str,
        message: &str,
    ) -> Result<(), String> {
        // Preserve the error code across the ErrorLoc allocation (which clobbers
        // caller-saved registers), then stamp the origin source location into the
        // dedicated error-source register. Allocation-free (terminal path).
        let code_slot = self.allocate_stack_object("error_return_code", 8);
        let loc_slot = self.allocate_stack_object("error_return_loc", 8);
        self.emit(abi::store_u64(code_register, abi::stack_pointer(), code_slot));
        let loc_register = self.emit_build_error_loc()?;
        self.emit(abi::store_u64(&loc_register, abi::stack_pointer(), loc_slot));
        self.emit_load_string_address_into(RESULT_ERROR_MESSAGE_REGISTER, message)?;
        self.emit(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            loc_slot,
        ));
        if let Some(slot) = self.error_arena_restore_slot {
            self.emit(abi::load_u64(
                ARENA_STATE_REGISTER,
                abi::stack_pointer(),
                slot,
            ));
        }
        // Inside a raw-capture region (inline `TRAP` on an inline built-in) the
        // error is not propagated: leave the raw `Result` in the standard
        // registers and join the capture point so it can be materialized.
        if let Some(label) = self.raw_result_capture.clone() {
            self.emit(abi::branch(&label));
        } else {
            self.emit(abi::return_());
        }
        Ok(())
    }

    fn ensure_pending_result_slots(&mut self) -> PendingResultSlots {
        if let Some(slots) = self.pending_result_slots {
            return slots;
        }
        let slots = PendingResultSlots {
            value: self.allocate_stack_object("pending_result_value", 8),
            tag: self.allocate_stack_object("pending_result_tag", 8),
            message: self.allocate_stack_object("pending_result_message", 8),
            source: self.allocate_stack_object("pending_result_source", 8),
        };
        self.pending_result_slots = Some(slots);
        slots
    }

    fn store_pending_success_result(&mut self, value: Option<&ValueResult>) -> Result<(), String> {
        let slots = self.ensure_pending_result_slots();
        let value_register = if let Some(value) = value {
            if value.type_ == "Nothing" {
                let register = self.allocate_register()?;
                self.emit(abi::move_immediate(&register, "Integer", "0"));
                register
            } else if self.inline_collection_payload_size(&value.type_).is_some() {
                self.materialize_inline_value_in_arena(&value.type_, &value.location)?
            } else {
                value.location.clone()
            }
        } else {
            let register = self.allocate_register()?;
            self.emit(abi::move_immediate(&register, "Integer", "0"));
            register
        };
        let message_register = self.allocate_register()?;
        self.emit(abi::move_immediate(&message_register, "Integer", "0"));
        self.emit(abi::store_u64(
            &value_register,
            abi::stack_pointer(),
            slots.value,
        ));
        self.emit(abi::move_immediate("x9", "Integer", RESULT_OK_TAG));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), slots.tag));
        self.emit(abi::store_u64(
            &message_register,
            abi::stack_pointer(),
            slots.message,
        ));
        // Success results carry no error source.
        self.emit(abi::move_immediate("x9", "Integer", "0"));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), slots.source));
        Ok(())
    }

    fn store_pending_error_registers(
        &mut self,
        code_register: &str,
        message_register: &str,
        source_register: &str,
    ) {
        let slots = self.ensure_pending_result_slots();
        self.emit(abi::store_u64(
            code_register,
            abi::stack_pointer(),
            slots.value,
        ));
        self.emit(abi::move_immediate("x9", "Integer", RESULT_ERR_TAG));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), slots.tag));
        self.emit(abi::store_u64(
            message_register,
            abi::stack_pointer(),
            slots.message,
        ));
        self.emit(abi::store_u64(
            source_register,
            abi::stack_pointer(),
            slots.source,
        ));
    }

    fn store_pending_error_from_value(&mut self, error: &NirValue) -> Result<(), String> {
        let error = self.lower_value(error)?;
        if error.type_ != "Error" {
            return Err(format!(
                "cleanup error exit expects Error value, got `{}`",
                error.type_
            ));
        }
        let code_register = self.allocate_register()?;
        let message_register = self.allocate_register()?;
        let source_register = self.allocate_register()?;
        self.emit(abi::load_u64(&code_register, &error.location, 0));
        self.emit(abi::load_u64(&message_register, &error.location, 8));
        self.emit(abi::load_u64(&source_register, &error.location, 16));
        self.store_pending_error_registers(&code_register, &message_register, &source_register);
        Ok(())
    }

    fn emit_direct_error_return(&mut self, error: &NirValue) -> Result<(), String> {
        let error = self.lower_value(error)?;
        if error.type_ != "Error" {
            return Err(format!(
                "native code fail expects Error value, got `{}`",
                error.type_
            ));
        }
        let code_register = self.allocate_register()?;
        let message_register = self.allocate_register()?;
        let source_register = self.allocate_register()?;
        self.emit(abi::load_u64(&code_register, &error.location, 0));
        self.emit(abi::load_u64(&message_register, &error.location, 8));
        self.emit(abi::load_u64(&source_register, &error.location, 16));
        self.emit(abi::move_register(RESULT_VALUE_REGISTER, &code_register));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        self.emit(abi::move_register(
            RESULT_ERROR_MESSAGE_REGISTER,
            &message_register,
        ));
        self.emit(abi::move_register(
            RESULT_ERROR_SOURCE_REGISTER,
            &source_register,
        ));
        self.emit(abi::return_());
        Ok(())
    }

    fn emit_direct_error_route_to_trap(&mut self, error: &NirValue) -> Result<(), String> {
        let error = self.lower_value(error)?;
        if error.type_ != "Error" {
            return Err(format!(
                "trap routing expects Error value, got `{}`",
                error.type_
            ));
        }
        let (stack_offset, label) = self
            .trap
            .as_ref()
            .and_then(|trap| {
                self.locals
                    .get(&trap.name)
                    .map(|local| (local.stack_offset, trap.label.clone()))
            })
            .ok_or_else(|| "trap routing requires bound trap local".to_string())?;
        self.emit(abi::store_u64(
            &error.location,
            abi::stack_pointer(),
            stack_offset,
        ));
        self.emit(abi::branch(&label));
        Ok(())
    }

    fn store_pending_current_result(&mut self) {
        let slots = self.ensure_pending_result_slots();
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            slots.value,
        ));
        self.emit(abi::store_u64(
            RESULT_TAG_REGISTER,
            abi::stack_pointer(),
            slots.tag,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            slots.message,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            slots.source,
        ));
    }

    fn load_pending_result_registers(&mut self) {
        let slots = self
            .pending_result_slots
            .expect("pending result slots must exist before loading");
        self.emit(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            slots.value,
        ));
        self.emit(abi::load_u64(
            RESULT_TAG_REGISTER,
            abi::stack_pointer(),
            slots.tag,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            slots.message,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            slots.source,
        ));
    }

    pub(super) fn error_exit_destination(&self) -> ExitDestination {
        if self.trap.as_ref().is_some_and(|trap| !trap.in_trap_body) {
            ExitDestination::Trap
        } else {
            ExitDestination::Return
        }
    }

    pub(super) fn is_thread_type(type_: &str) -> bool {
        type_.starts_with("Thread OF ")
    }

    pub(super) fn thread_drop_symbol() -> String {
        runtime::symbol_for_call(runtime::RuntimeHelper::Thread, "thread.drop")
    }

    pub(super) fn deactivate_thread_cleanup(&mut self, name: &str) {
        if let Some(index) = self.active_cleanups.iter().rposition(
            |cleanup| matches!(cleanup, ActiveCleanup::Thread(thread) if thread.name == name),
        ) {
            self.active_cleanups.remove(index);
        }
    }

    pub(super) fn maybe_deactivate_moved_thread_local(&mut self, value: &NirValue) {
        let NirValue::Local(name) = value else {
            return;
        };
        if self
            .locals
            .get(name)
            .is_some_and(|local| Self::is_thread_type(&local.type_))
        {
            self.deactivate_thread_cleanup(name);
        }
    }

    fn deactivate_moved_thread_arguments(&mut self, target: &str, args: &[NirValue]) {
        match target {
            "thread.start"
            | "thread.send"
            | "thread.emit"
            | "thread.transferResource"
            | "thread.emitResource" => {
                if let Some(arg) = args.get(1) {
                    self.maybe_deactivate_moved_thread_local(arg);
                }
            }
            target if !target.starts_with("thread.") => {
                for arg in args {
                    self.maybe_deactivate_moved_thread_local(arg);
                }
            }
            _ => {}
        }
    }

    pub(super) fn resource_cleanup_symbol(&self, type_: &str) -> Option<String> {
        let close = crate::builtins::resource_close_function(type_)?;
        let symbol = self
            .function_symbols
            .get(close)
            .cloned()
            .or_else(|| {
                runtime::helper_for_call(close)
                    .map(|helper| runtime::symbol_for_call(helper, close))
            })
            .unwrap_or_else(|| close.to_string());
        Some(symbol)
    }

    /// If `type_` is a resource union (every variant is a resource), the
    /// `(tag, close_symbol)` pairs for tag-dispatched drop; otherwise `None`.
    pub(super) fn resource_union_cleanup(&self, type_: &str) -> Option<Vec<(usize, String)>> {
        if !self.type_model.union_names.contains(type_) {
            return None;
        }
        let variants: Vec<String> = self
            .type_model
            .variants_for_union(type_)
            .cloned()
            .collect();
        if variants.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for variant in variants {
            if !crate::builtins::is_resource_type(&variant) {
                return None;
            }
            let tag = *self.type_model.union_variant_tags.get(&variant)?;
            let symbol = self.resource_cleanup_symbol(&variant)?;
            out.push((tag, symbol));
        }
        Some(out)
    }

    pub(super) fn deactivate_resource_cleanup(&mut self, name: &str) {
        if let Some(index) = self.active_cleanups.iter().rposition(|cleanup| {
            matches!(cleanup, ActiveCleanup::Resource(resource) if resource.name == name)
                || matches!(cleanup, ActiveCleanup::ResourceUnion(u) if u.name == name)
        }) {
            self.active_cleanups.remove(index);
        }
    }

    /// Tag-dispatched drop of a resource union: read the union tag and call the
    /// active variant's registered close op on its resource pointer (offset 8).
    pub(super) fn emit_resource_union_cleanup_call(
        &mut self,
        cleanup: &ResourceUnionCleanup,
    ) -> Result<(), String> {
        let stack_offset = match self.locals.get(&cleanup.name) {
            Some(local) => local.stack_offset,
            None => return Ok(()),
        };
        let union_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&union_ptr, abi::stack_pointer(), stack_offset));
        let union_slot = self.allocate_stack_object("resource_union_drop_ptr", 8);
        self.emit(abi::store_u64(&union_ptr, abi::stack_pointer(), union_slot));
        let tag_register = self.allocate_register()?;
        self.emit(abi::load_u64(&tag_register, &union_ptr, 0));
        let tag_slot = self.allocate_stack_object("resource_union_drop_tag", 8);
        self.emit(abi::store_u64(&tag_register, abi::stack_pointer(), tag_slot));
        let done = self.label("resource_union_drop_done");
        let payload_slot = self.allocate_stack_object("resource_union_drop_payload", 8);
        for (tag, symbol) in cleanup.variants.clone() {
            let next = self.label("resource_union_drop_next");
            let tag_reg = self.allocate_register()?;
            self.emit(abi::load_u64(&tag_reg, abi::stack_pointer(), tag_slot));
            self.emit(abi::compare_immediate(&tag_reg, &tag.to_string()));
            self.emit(abi::branch_ne(&next));
            // Load the variant's resource pointer (payload at offset 8) and close it.
            let base = self.allocate_register()?;
            self.emit(abi::load_u64(&base, abi::stack_pointer(), union_slot));
            let payload = self.allocate_register()?;
            self.emit(abi::load_u64(&payload, &base, 8));
            self.emit(abi::store_u64(&payload, abi::stack_pointer(), payload_slot));
            let arg = NirValue::Local(format!("__resource_union_payload@{payload_slot}"));
            self.locals.insert(
                format!("__resource_union_payload@{payload_slot}"),
                LocalValue {
                    type_: "File".to_string(),
                    stack_offset: payload_slot,
                    constant: None,
                },
            );
            self.emit_raw_call(&symbol, std::slice::from_ref(&arg), "resource_union_drop_arg")?;
            let after = self.label("resource_union_drop_check");
            self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&after));
            self.record_secondary_cleanup_failure();
            self.emit(abi::label(&after));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&next));
        }
        self.emit(abi::label(&done));
        Ok(())
    }

    fn deactivate_moved_resource_arguments(&mut self, target: &str, args: &[NirValue]) {
        for (index, arg) in args.iter().enumerate() {
            let NirValue::Local(name) = arg else {
                continue;
            };
            let Some(local) = self.locals.get(name) else {
                continue;
            };
            let Some(close) = crate::builtins::resource_close_function(&local.type_) else {
                continue;
            };
            let consumed = if target == close {
                index == 0
            } else if matches!(
                target,
                "thread.start"
                    | "thread.send"
                    | "thread.emit"
                    | "thread.transferResource"
                    | "thread.emitResource"
            ) {
                // A thread-sendable resource is moved into the thread on a
                // successful transfer. Deactivation runs only on the success
                // path (after the result-tag branch), so the sender keeps
                // ownership and cleanup when the transfer fails with `Err`.
                index == 1 && crate::builtins::is_thread_sendable_resource_type(&local.type_)
            } else if crate::builtins::is_builtin_call(target) {
                false
            } else {
                // Ordinary user calls borrow the resource: the caller retains
                // ownership and its scope-drop cleanup. Only the fixed
                // invalidation events (registered close, thread transfer,
                // `RETURN`) hand off ownership.
                false
            };
            if consumed {
                self.deactivate_resource_cleanup(name);
            }
        }
    }

    pub(super) fn emit_resource_cleanup_call(
        &mut self,
        cleanup: &ResourceCleanup,
    ) -> Result<(), String> {
        let arg = NirValue::Local(cleanup.name.clone());
        self.emit_raw_call(
            &cleanup.symbol,
            std::slice::from_ref(&arg),
            "resource_drop_arg",
        )?;
        let done = self.label("resource_cleanup_done");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&done));
        self.record_secondary_cleanup_failure();
        self.emit(abi::label(&done));
        Ok(())
    }

    fn record_secondary_cleanup_failure(&mut self) {
        self.emit(abi::load_u64(
            "x9",
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
        ));
        self.emit(abi::add_immediate("x9", "x9", 1));
        self.emit(abi::store_u64(
            "x9",
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
        ));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_CODE_OFFSET,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
        ));
    }

    pub(super) fn emit_thread_cleanup_call(
        &mut self,
        cleanup: &ThreadCleanup,
    ) -> Result<(), String> {
        let arg = NirValue::Local(cleanup.name.clone());
        self.emit_raw_call(
            &cleanup.symbol,
            std::slice::from_ref(&arg),
            "thread_drop_arg",
        )?;
        Ok(())
    }

    pub(super) fn emit_thread_cleanup_for_name(&mut self, name: &str) -> Result<(), String> {
        let cleanup = ThreadCleanup {
            name: name.to_string(),
            symbol: Self::thread_drop_symbol(),
        };
        self.emit_thread_cleanup_call(&cleanup)
    }

    /// The close op symbol for a resource collection's element/value type, or an
    /// error if `type_` is not a collection whose element is a single resource.
    fn collection_resource_close_symbol(&self, type_: &str) -> Result<String, String> {
        let element = list_element_type(type_)
            .or_else(|| map_type_parts(type_).map(|(_, value)| value))
            .ok_or_else(|| format!("owned-list owner '{type_}' is not a collection"))?;
        self.resource_cleanup_symbol(&element).ok_or_else(|| {
            format!("owned-list element type '{element}' has no registered close op")
        })
    }

    /// Allocate and register a runtime owned-list for an owner collection binding
    /// (§15.6): a head-pointer stack slot (initialized empty) plus an
    /// [`ActiveCleanup::OwnedList`] obligation drained on every exit path.
    pub(super) fn setup_owned_list(&mut self, name: &str, type_: &str) -> Result<(), String> {
        let close_symbol = self.collection_resource_close_symbol(type_)?;
        let head_slot = self.allocate_stack_object(&format!("owned_list_{name}"), 8);
        self.emit(abi::move_immediate("x9", "Integer", "0"));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), head_slot));
        self.owned_list_heads.insert(name.to_string(), head_slot);
        self.active_cleanups
            .push(ActiveCleanup::OwnedList(OwnedListCleanup {
                name: name.to_string(),
                head_slot,
                close_symbol,
            }));
        Ok(())
    }

    /// Transfer a returned resource collection's owned-list to the caller: drop
    /// its drain obligation from this scope so the resources are not closed here
    /// (the caller's scope adopts and closes them). Other scopes' owned-lists are
    /// untouched (§15.6).
    pub(super) fn deactivate_owned_list(&mut self, name: &str) {
        if let Some(index) = self
            .active_cleanups
            .iter()
            .rposition(|cleanup| matches!(cleanup, ActiveCleanup::OwnedList(o) if o.name == name))
        {
            self.active_cleanups.remove(index);
        }
    }

    /// Whether a NIR type string is a `RES`-marked resource collection
    /// (`List OF RES File`, `Map OF K TO RES File`): its scope-ownership transfers
    /// across a function boundary (§15.6).
    pub(super) fn is_res_marked_resource_collection(type_: &str) -> bool {
        type_.strip_prefix("List OF ").is_some_and(|e| e.starts_with("RES "))
            || type_
                .strip_prefix("Map OF ")
                .and_then(|rest| rest.split_once(" TO "))
                .is_some_and(|(_, value)| value.starts_with("RES "))
    }

    /// Push the resource record at `resource_slot` onto `collection`'s owned-list
    /// as a fresh `{record, next}` node (§15.6).
    pub(super) fn emit_owned_list_push(
        &mut self,
        collection: &str,
        resource_slot: usize,
    ) -> Result<(), String> {
        let head_slot = *self.owned_list_heads.get(collection).ok_or_else(|| {
            format!("resource floats to '{collection}', which has no owned-list")
        })?;
        // Allocate a 16-byte node (record ptr at 0, next at 8).
        let alloc_ok = self.label("owned_list_alloc_ok");
        self.emit(abi::move_immediate(abi::return_register(), "Integer", "16"));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(abi::return_register(), RESULT_OK_TAG));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        // x1 = node pointer.
        self.emit(abi::load_u64("x9", abi::stack_pointer(), resource_slot));
        self.emit(abi::store_u64("x9", "x1", 0));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), head_slot));
        self.emit(abi::store_u64("x10", "x1", 8));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), head_slot));
        Ok(())
    }

    /// Adopt the resources of a `List OF RES File` value transferred in from a
    /// call: walk the collection and push each element record onto this scope's
    /// owned-list, so the scope closes each once at exit (§15.6).
    pub(super) fn emit_owned_list_seed_from_collection(
        &mut self,
        collection: &str,
        collection_slot: usize,
        element_type: &str,
    ) -> Result<(), String> {
        let cursor_slot = self.allocate_stack_object("adopt_cursor", 8);
        let remaining_slot = self.allocate_stack_object("adopt_remaining", 8);
        let elem_slot = self.allocate_stack_object("adopt_elem", 8);
        self.initialize_collection_loop_slots(collection_slot, cursor_slot, remaining_slot);
        let loop_label = self.label("owned_list_seed_loop");
        let done_label = self.label("owned_list_seed_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done_label));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), elem_slot));
        self.emit_owned_list_push(collection, elem_slot)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label);
        self.emit(abi::label(&done_label));
        Ok(())
    }

    /// Drain an owned-list: walk it head-first, closing each record once. The
    /// close is closed-flag idempotent, so a record reachable through more than
    /// one path closes exactly once (§15.6).
    pub(super) fn emit_owned_list_drain(
        &mut self,
        cleanup: &OwnedListCleanup,
    ) -> Result<(), String> {
        let loop_label = self.label("owned_list_drain_loop");
        let done_label = self.label("owned_list_drain_done");
        let close_ok = self.label("owned_list_close_ok");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), cleanup.head_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done_label));
        // Advance the head past this node before the call, which clobbers
        // caller-saved registers; the loop reloads the head from memory.
        self.emit(abi::load_u64(abi::return_register(), "x9", 0));
        self.emit(abi::load_u64("x10", "x9", 8));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cleanup.head_slot));
        self.emit_symbol_call(&cleanup.close_symbol);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&close_ok));
        self.record_secondary_cleanup_failure();
        self.emit(abi::label(&close_ok));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    fn emit_cleanup_sequence(&mut self) -> Result<(), String> {
        let cleanups = self.active_cleanups.clone();
        for cleanup in cleanups.iter().rev() {
            match cleanup {
                ActiveCleanup::Thread(cleanup) => {
                    self.emit_thread_cleanup_call(cleanup)?;
                }
                ActiveCleanup::Resource(cleanup) => {
                    self.emit_resource_cleanup_call(cleanup)?;
                }
                ActiveCleanup::ResourceUnion(cleanup) => {
                    self.emit_resource_union_cleanup_call(cleanup)?;
                }
                ActiveCleanup::OwnedList(cleanup) => {
                    self.emit_owned_list_drain(cleanup)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn emit_cleanup_branch_to_depth(
        &mut self,
        target: &str,
        cleanup_depth: usize,
    ) -> Result<(), String> {
        let cleanups = self.active_cleanups[cleanup_depth..].to_vec();
        for cleanup in cleanups.iter().rev() {
            match cleanup {
                ActiveCleanup::Thread(cleanup) => self.emit_thread_cleanup_call(cleanup)?,
                ActiveCleanup::Resource(cleanup) => self.emit_resource_cleanup_call(cleanup)?,
                ActiveCleanup::ResourceUnion(cleanup) => {
                    self.emit_resource_union_cleanup_call(cleanup)?
                }
                ActiveCleanup::OwnedList(cleanup) => self.emit_owned_list_drain(cleanup)?,
            }
        }
        self.emit(abi::branch(target));
        Ok(())
    }

    pub(super) fn emit_program_exit_value(&mut self, code: &NirValue) -> Result<(), String> {
        let result = self.lower_value(code)?;
        self.emit(abi::move_register(abi::return_register(), &result.location));
        self.emit(abi::move_register(
            RESULT_VALUE_REGISTER,
            abi::return_register(),
        ));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_PROGRAM_EXIT_TAG,
        ));
        self.emit(abi::move_immediate(
            RESULT_ERROR_MESSAGE_REGISTER,
            "Integer",
            "0",
        ));
        self.emit_current_result_exit(ExitDestination::Return)
    }

    pub(super) fn emit_current_result_exit(
        &mut self,
        destination: ExitDestination,
    ) -> Result<(), String> {
        if self.active_cleanups.is_empty() {
            match destination {
                ExitDestination::Return => self.emit(abi::return_()),
                ExitDestination::Trap => self.route_current_result_to_trap()?,
            }
            return Ok(());
        }
        self.store_pending_current_result();
        self.emit_cleanup_sequence()?;
        self.load_pending_result_registers();
        match destination {
            ExitDestination::Return => self.emit(abi::return_()),
            ExitDestination::Trap => self.route_current_result_to_trap()?,
        }
        Ok(())
    }

    pub(super) fn emit_error_value_exit(
        &mut self,
        error: &NirValue,
        destination: ExitDestination,
    ) -> Result<(), String> {
        if self.active_cleanups.is_empty() {
            return match destination {
                ExitDestination::Return => self.emit_direct_error_return(error),
                ExitDestination::Trap => self.emit_direct_error_route_to_trap(error),
            };
        }
        self.store_pending_error_from_value(error)?;
        self.emit_cleanup_sequence()?;
        self.load_pending_result_registers();
        match destination {
            ExitDestination::Return => self.emit(abi::return_()),
            ExitDestination::Trap => self.route_current_result_to_trap()?,
        }
        Ok(())
    }

    pub(super) fn emit_return_exit(&mut self, value: Option<&NirValue>) -> Result<(), String> {
        if self.active_cleanups.is_empty() {
            if let Some(value) = value {
                let result = self.lower_value(value)?;
                if result.type_ != "Nothing" {
                    if self.inline_collection_payload_size(&result.type_).is_some() {
                        let stable = self
                            .materialize_inline_value_in_arena(&result.type_, &result.location)?;
                        self.emit(abi::move_register(RESULT_VALUE_REGISTER, &stable));
                    } else {
                        self.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
                    }
                }
            }
            self.emit(abi::move_immediate(
                RESULT_TAG_REGISTER,
                "Integer",
                RESULT_OK_TAG,
            ));
            self.emit(abi::return_());
            return Ok(());
        }
        let result = if let Some(value) = value {
            Some(self.lower_value(value)?)
        } else {
            None
        };
        self.store_pending_success_result(result.as_ref())?;
        if let Some(value) = value {
            if let NirValue::Local(name) = value {
                if result
                    .as_ref()
                    .is_some_and(|result| Self::is_thread_type(&result.type_))
                {
                    self.deactivate_thread_cleanup(name);
                }
                if result.as_ref().is_some_and(|result| {
                    crate::builtins::resource_close_function(&result.type_).is_some()
                }) {
                    self.deactivate_resource_cleanup(name);
                }
                // Returning a `List OF RES File` transfers its owned-list to the
                // caller: drop this scope's drain so the resources are not closed
                // here (§15.6).
                if result
                    .as_ref()
                    .is_some_and(|result| Self::is_res_marked_resource_collection(&result.type_))
                {
                    self.deactivate_owned_list(name);
                }
            }
        }
        self.emit_cleanup_sequence()?;
        self.load_pending_result_registers();
        self.emit(abi::return_());
        Ok(())
    }

    pub(super) fn route_current_result_to_trap(&mut self) -> Result<(), String> {
        self.emit(abi::compare_immediate(
            RESULT_TAG_REGISTER,
            RESULT_PROGRAM_EXIT_TAG,
        ));
        let trap_label = self.label("trap_route_error");
        self.emit(abi::branch_ne(&trap_label));
        self.emit(abi::return_());
        self.emit(abi::label(&trap_label));

        let code_slot = self.allocate_stack_object("trap_error_code", 8);
        let message_slot = self.allocate_stack_object("trap_error_message", 8);
        let source_slot = self.allocate_stack_object("trap_error_source", 8);
        let error_slot = self.allocate_stack_object("trap_error", 8);
        let alloc_ok = self.label("trap_error_alloc_ok");
        let (stack_offset, label) = self
            .trap
            .as_ref()
            .and_then(|trap| {
                self.locals
                    .get(&trap.name)
                    .map(|local| (local.stack_offset, trap.label.clone()))
            })
            .ok_or_else(|| "trap routing requires bound trap local".to_string())?;

        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_slot,
        ));
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &ERROR_OBJECT_SIZE.to_string(),
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), error_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), code_slot));
        self.emit(abi::store_u64("x9", "x1", 0));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), message_slot));
        self.emit(abi::store_u64("x9", "x1", 8));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64("x9", "x1", 16));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), error_slot));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), stack_offset));
        self.emit(abi::branch(&label));
        Ok(())
    }

    /// Load the address of a string constant into the given register without
    /// allocating from the temporary-register pool.
    pub(super) fn emit_load_string_address_into(
        &mut self,
        register: &str,
        value: &str,
    ) -> Result<(), String> {
        let symbol = self
            .string_symbols
            .get(value)
            .ok_or_else(|| format!("native code string literal '{value}' has no data object"))?
            .clone();
        self.emit_load_static_string_symbol(register, &symbol);
        Ok(())
    }

    pub(super) fn current_block_returns(&self) -> bool {
        self.instructions
            .last()
            .is_some_and(|instruction| matches!(instruction.op, CodeOp::Ret | CodeOp::Branch))
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
