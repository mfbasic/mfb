use super::*;

impl CodeBuilder<'_> {
    pub(super) fn load_string_constant(&mut self, value: &str) -> Result<String, String> {
        let register = self.allocate_register()?;
        self.emit_load_string_constant(&register, value)?;
        Ok(register)
    }

    pub(super) fn lower_field_access(
        &mut self,
        target: &NirValue,
        member: &str,
    ) -> Result<ValueResult, String> {
        let target_value = self.lower_value(target)?;
        let (field_index, field_type, payload_offset) =
            if let Some((key_type, value_type)) = parse_map_entry_type(&target_value.type_) {
                match member {
                    "key" => (0, key_type, 0),
                    "value" => (1, value_type, 0),
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
                (index, field_type.clone(), 0)
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
                (index, field_type.clone(), 8)
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
                (index, field_type, 8)
            } else if target_value.type_ == "Error" {
                match member {
                    "code" => (0, "Integer".to_string(), 0),
                    "message" => (1, "String".to_string(), 0),
                    _ => {
                        return Err(format!("native code Error has no field '{member}'"));
                    }
                }
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
        let register = self.allocate_register()?;
        let object_offset = self.allocate_stack_object(type_, 8 * fields.len());
        for (index, _) in fields.iter().enumerate() {
            let scratch = self.allocate_register()?;
            self.emit(abi::load_u64(&scratch, &target_value.location, 8 * index));
            self.emit(abi::store_u64(
                &scratch,
                abi::stack_pointer(),
                object_offset + 8 * index,
            ));
        }
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
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                object_offset + 8 * index,
            ));
        }
        self.emit(abi::add_immediate(
            &register,
            abi::stack_pointer(),
            object_offset,
        ));
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

    pub(super) fn local_constant_value(&self, value: &NirValue) -> Option<NirValue> {
        match value {
            NirValue::Const { .. } => Some(value.clone()),
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.clone()),
            NirValue::Call { target, args } if target == "toString" && args.len() == 1 => self
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
            NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. }
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
            NirValue::Call { target, args } if target == "toString" && args.len() == 1 => {
                self.static_primitive_text(&args[0])
            }
            NirValue::RuntimeCall { target, args, .. }
                if target == "toString" && args.len() == 1 =>
            {
                self.static_primitive_text(&args[0])
            }
            NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. }
                if target == "typeName" && args.len() == 1 =>
            {
                self.static_type_name(&args[0])
            }
            NirValue::Binary { op, left, right } if op == "&" => {
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
            _ => None,
        }
    }

    pub(super) fn static_type_name(&self, value: &NirValue) -> Option<String> {
        match value {
            NirValue::Const { type_, .. } => Some(type_.clone()),
            NirValue::Local(name) => self.locals.get(name).map(|local| local.type_.clone()),
            NirValue::FunctionRef { type_, .. }
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
            NirValue::Call { target, args }
            | NirValue::CallResult { target, args }
            | NirValue::RuntimeCall { target, args, .. } => {
                match target.as_str() {
                    "replace" | "typeName" | "toString" => Some("String".to_string()),
                    "find" | "len" | "toInt" => Some("Integer".to_string()),
                    "mid" => Some("String".to_string()),
                    "toFloat" => Some("Float".to_string()),
                    "toFixed" => Some("Fixed".to_string()),
                    "toByte" => Some("Byte".to_string()),
                    "isNumeric" => Some("Boolean".to_string()),
                    "math.floor" | "math.ceil" | "math.round" => Some("Integer".to_string()),
                    "math.sqrt" | "math.exp" | "math.log" | "math.log10" | "math.sin"
                    | "math.cos" | "math.tan" | "math.asin" | "math.acos" | "math.atan" => {
                        args.first().and_then(|arg| self.static_type_name(arg))
                    }
                    "math.pow" | "math.atan2" => {
                        args.first().and_then(|arg| self.static_type_name(arg))
                    }
                    _ => None,
                }
            }
            NirValue::Binary { op, left, right } => {
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
            NirValue::Unary { op, operand } => {
                if op == "NOT" {
                    Some("Boolean".to_string())
                } else {
                    self.static_type_name(operand)
                }
            }
            NirValue::MemberAccess { target, member } => {
                let target_type = self.static_type_name(target)?;
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
            "thread.cancel" | "thread.send" | "thread.emit" => Some("Nothing".to_string()),
            "thread.waitFor" => {
                let thread_type = self.static_type_name(args.first()?)?;
                builtins::thread::thread_output(&thread_type).map(str::to_string)
            }
            "thread.read" | "thread.receive" => {
                let thread_type = self.static_type_name(args.first()?)?;
                builtins::thread::thread_message(&thread_type).map(str::to_string)
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
        let mut arg_values = Vec::new();
        let mut arg_slots = Vec::new();
        for arg in args {
            let value = self.lower_value(arg)?;
            let slot = self.allocate_stack_object("call_arg", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            arg_values.push(value);
            arg_slots.push(slot);
        }
        for (index, slot) in arg_slots.iter().enumerate() {
            self.emit(abi::load_u64("x9", abi::stack_pointer(), *slot));
            self.emit(abi::move_register(
                &abi::argument_register(index)?,
                "x9",
            ));
        }
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
            if self.trap.is_some() {
                self.route_current_result_to_trap()?;
            } else {
                self.emit(abi::return_());
            }
            self.emit(abi::label(&ok_label));
        }
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
    ) -> Result<ValueResult, String> {
        let mut arg_values = Vec::new();
        let mut arg_slots = Vec::new();
        for arg in args {
            let value = self.lower_value(arg)?;
            let slot = self.allocate_stack_object("runtime_call_arg", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            arg_values.push(value);
            arg_slots.push(slot);
        }
        for (index, slot) in arg_slots.iter().enumerate() {
            self.emit(abi::load_u64("x9", abi::stack_pointer(), *slot));
            self.emit(abi::move_register(
                &abi::argument_register(index)?,
                "x9",
            ));
        }
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

        let ok_label = self.label("runtime_call_ok");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        if self.trap.is_some() {
            self.route_current_result_to_trap()?;
        } else {
            self.emit(abi::return_());
        }
        self.emit(abi::label(&ok_label));

        if result_type == "Nothing" {
            return Ok(ValueResult {
                type_: result_type.to_string(),
                location: "void".to_string(),
                text: format!("call {target}({})", join_texts(&arg_values)),
            });
        }

        let register = self.allocate_register()?;
        self.emit(abi::move_register(&register, RESULT_VALUE_REGISTER));
        Ok(ValueResult {
            type_: result_type.to_string(),
            location: register,
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
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

    pub(super) fn emit_error_register_return(
        &mut self,
        code_register: &str,
        message: &str,
    ) -> Result<(), String> {
        let message_register = self.load_string_address(message)?;
        self.emit(abi::move_register(RESULT_VALUE_REGISTER, code_register));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        self.emit(abi::move_register(
            RESULT_ERROR_MESSAGE_REGISTER,
            &message_register,
        ));
        self.emit(abi::return_());
        Ok(())
    }

    pub(super) fn emit_error_return(&mut self, error: &NirValue) -> Result<(), String> {
        let error = self.lower_value(error)?;
        if error.type_ != "Error" {
            return Err(format!(
                "native code fail expects Error value, got `{}`",
                error.type_
            ));
        }
        let code_register = self.allocate_register()?;
        let message_register = self.allocate_register()?;
        self.emit(abi::load_u64(&code_register, &error.location, 0));
        self.emit(abi::load_u64(&message_register, &error.location, 8));
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
        self.emit(abi::return_());
        Ok(())
    }

    pub(super) fn route_error_value_to_trap(&mut self, error: &NirValue) -> Result<(), String> {
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

    pub(super) fn route_current_result_to_trap(&mut self) -> Result<(), String> {
        let code_slot = self.allocate_stack_object("trap_error_code", 8);
        let message_slot = self.allocate_stack_object("trap_error_message", 8);
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), error_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), code_slot));
        self.emit(abi::store_u64("x9", "x1", 0));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), message_slot));
        self.emit(abi::store_u64("x9", "x1", 8));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), error_slot));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), stack_offset));
        self.emit(abi::branch(&label));
        Ok(())
    }

    pub(super) fn load_string_address(&mut self, value: &str) -> Result<String, String> {
        let symbol = self
            .string_symbols
            .get(value)
            .ok_or_else(|| format!("native code string literal '{value}' has no data object"))?
            .clone();
        let register = self.allocate_register()?;
        self.emit(abi::load_page_address(&register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(&register, &register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        Ok(register)
    }

    pub(super) fn current_block_returns(&self) -> bool {
        self.instructions
            .last()
            .is_some_and(|instruction| instruction.op == CodeOp::Ret)
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
