use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_ops(&mut self, ops: &[NirOp]) -> Result<(), String> {
        for op in ops {
            match op {
                NirOp::Bind {
                    name, type_, value, ..
                } => {
                    let stack_offset = self.allocate_stack_object(name, 8);
                    let constant = value
                        .as_ref()
                        .and_then(|value| self.local_constant_value(value));
                    self.locals.insert(
                        name.clone(),
                        LocalValue {
                            type_: type_.clone(),
                            stack_offset,
                            constant,
                        },
                    );
                    if let Some(value) = value {
                        let result = self.lower_value(value)?;
                        self.emit(abi::store_u64(
                            &result.location,
                            abi::stack_pointer(),
                            stack_offset,
                        ));
                    } else if is_collection_type(type_) {
                        let result = self.lower_empty_collection(type_)?;
                        self.emit(abi::store_u64(
                            &result.location,
                            abi::stack_pointer(),
                            stack_offset,
                        ));
                    } else if let Some(fields) = self.type_model.record_fields.get(type_).cloned() {
                        let record_offset = self.allocate_stack_object(type_, 8 * fields.len());
                        for index in 0..fields.len() {
                            self.emit(abi::store_u64(
                                "x31",
                                abi::stack_pointer(),
                                record_offset + 8 * index,
                            ));
                        }
                        let register = self.allocate_register()?;
                        self.emit(abi::add_immediate(
                            &register,
                            abi::stack_pointer(),
                            record_offset,
                        ));
                        self.emit(abi::store_u64(
                            &register,
                            abi::stack_pointer(),
                            stack_offset,
                        ));
                    }
                }
                NirOp::Assign { name, value } => {
                    let stack_offset = self
                        .locals
                        .get(name)
                        .ok_or_else(|| format!("native code assignment unknown local '{name}'"))?
                        .stack_offset;
                    let result = self.lower_value(value)?;
                    self.emit(abi::store_u64(
                        &result.location,
                        abi::stack_pointer(),
                        stack_offset,
                    ));
                    let constant = self.local_constant_value(value);
                    if let Some(local) = self.locals.get_mut(name) {
                        local.constant = constant;
                    }
                }
                NirOp::Eval { value } => {
                    self.lower_value(value)?;
                }
                NirOp::Return { value } => {
                    if let Some(value) = value {
                        let result = self.lower_value(value)?;
                        if result.type_ != "Nothing" {
                            if self.inline_collection_payload_size(&result.type_).is_some() {
                                let stable = self
                                    .materialize_inline_value_in_arena(&result.type_, &result.location)?;
                                self.emit(abi::move_register(RESULT_VALUE_REGISTER, &stable));
                            } else {
                                self.emit(abi::move_register(
                                    RESULT_VALUE_REGISTER,
                                    &result.location,
                                ));
                            }
                        }
                    }
                    self.emit(abi::move_immediate(
                        RESULT_TAG_REGISTER,
                        "Integer",
                        RESULT_OK_TAG,
                    ));
                    self.emit(abi::return_());
                }
                NirOp::Fail { error } => {
                    self.emit_error_return(error)?;
                }
                NirOp::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    let condition = self.lower_value(condition)?;
                    let else_label = self.label("if_else");
                    let end_label = self.label("if_end");
                    let constants_before_if = self.local_constants();
                    self.emit(abi::compare_immediate(&condition.location, "0"));
                    self.emit(abi::branch_eq(&else_label).field("reason", "ifFalse"));
                    self.lower_ops(then_body)?;
                    if !self.current_block_returns() {
                        self.emit(abi::branch(&end_label));
                    }
                    self.emit(abi::label(&else_label));
                    self.restore_local_constants(&constants_before_if);
                    self.lower_ops(else_body)?;
                    self.emit(abi::label(&end_label));
                    self.clear_local_constants();
            }
            NirOp::Match { value, cases } => {
                let matched = self.lower_value(value)?;
                let matched_slot = self.allocate_stack_object("match_value", 8);
                self.emit(abi::store_u64(
                    &matched.location,
                    abi::stack_pointer(),
                    matched_slot,
                ));
                let end_label = self.label("match_end");
                for case in cases {
                    let matched_register = self.allocate_register()?;
                    self.emit(abi::load_u64(
                        &matched_register,
                        abi::stack_pointer(),
                        matched_slot,
                    ));
                    let case_matched = ValueResult {
                        type_: matched.type_.clone(),
                        location: matched_register,
                        text: matched.text.clone(),
                    };
                    let next_label = self.label("match_next");
                    match &case.pattern {
                        NirMatchPattern::Else => {}
                        NirMatchPattern::Value(pattern) => {
                            let case_label = self.label("match_case");
                            self.lower_match_compare(&case_matched, pattern, &case_label)?;
                            self.emit(abi::branch(&next_label));
                            self.emit(abi::label(&case_label));
                        }
                        NirMatchPattern::OneOf(patterns) => {
                            let case_label = self.label("match_case");
                            for pattern in patterns {
                                self.lower_match_compare(&case_matched, pattern, &case_label)?;
                            }
                            self.emit(abi::branch(&next_label));
                            self.emit(abi::label(&case_label));
                        }
                    }
                    let constants_before_case = self.local_constants();
                    let mut case_locals = self.locals.clone();
                    let mut body_index = 0;
                    while let Some(NirOp::Bind {
                        name,
                        type_,
                        value: Some(NirValue::UnionExtract { .. }),
                        ..
                    }) = case.body.get(body_index)
                    {
                        let bind = &case.body[body_index..body_index + 1];
                        self.lower_ops(bind)?;
                        if let Some(local) = self.locals.get(name).cloned() {
                            case_locals.insert(
                                name.clone(),
                                LocalValue {
                                    type_: type_.clone(),
                                    stack_offset: local.stack_offset,
                                    constant: local.constant,
                                },
                            );
                        }
                        body_index += 1;
                    }
                    if let Some(guard) = &case.guard {
                        let saved_locals = self.locals.clone();
                        self.locals = case_locals.clone();
                        let guard_value = self.lower_value(guard)?;
                        self.emit(abi::compare_immediate(&guard_value.location, "0"));
                        self.emit(abi::branch_eq(&next_label).field("reason", "matchGuardFalse"));
                        self.locals = saved_locals;
                    }
                    self.lower_ops(&case.body[body_index..])?;
                    if !self.current_block_returns() {
                        self.emit(abi::branch(&end_label));
                    }
                    self.restore_local_constants(&constants_before_case);
                    self.emit(abi::label(&next_label));
                }
                self.emit(abi::label(&end_label));
                self.clear_local_constants();
            }
                NirOp::While { condition, body } => {
                    let loop_label = self.label("while_loop");
                    let end_label = self.label("while_end");
                    self.emit(abi::label(&loop_label));
                    let condition = self.lower_value(condition)?;
                    self.emit(abi::compare_immediate(&condition.location, "0"));
                    self.emit(abi::branch_eq(&end_label));
                    self.clear_local_constants();
                    self.lower_ops(body)?;
                    self.emit(abi::branch(&loop_label));
                    self.emit(abi::label(&end_label));
                    self.clear_local_constants();
                }
                NirOp::ForEach {
                    name,
                    type_,
                    iterable,
                    body,
                } => {
                    self.lower_for_each(name, type_, iterable, body)?;
                }
                NirOp::Using {
                    name,
                    type_,
                    close,
                    value,
                    body,
                } => {
                    let stack_offset = self.allocate_stack_object(name, 8);
                    let result = self.lower_value(value)?;
                    self.locals.insert(
                        name.clone(),
                        LocalValue {
                            type_: type_.clone(),
                            stack_offset,
                            constant: self.local_constant_value(value),
                        },
                    );
                    self.emit(abi::store_u64(
                        &result.location,
                        abi::stack_pointer(),
                        stack_offset,
                    ));
                    self.lower_ops(body)?;
                    let symbol = self
                        .function_symbols
                        .get(close)
                        .cloned()
                        .unwrap_or_else(|| close.clone());
                    self.emit_call(close, &symbol, &[], None)?;
                }
            }
            self.reset_temporary_registers();
        }
        Ok(())
    }

    pub(super) fn lower_for_each(
        &mut self,
        name: &str,
        type_: &str,
        iterable: &NirValue,
        body: &[NirOp],
    ) -> Result<(), String> {
        let iterable_value = self.lower_value(iterable)?;
        if !is_collection_type(&iterable_value.type_) {
            return Err(format!(
                "native code FOR EACH target '{}' is not a collection",
                iterable_value.type_
            ));
        }
        let map_entry_types = if iterable_value.type_.starts_with("Map OF ") {
            Some(map_type_parts(&iterable_value.type_).ok_or_else(|| {
                format!(
                    "native code FOR EACH target '{}' is not a valid map type",
                    iterable_value.type_
                )
            })?)
        } else {
            None
        };
        let list_element_type = iterable_value
            .type_
            .strip_prefix("List OF ")
            .map(str::to_string);
        let item_value_type = list_element_type.as_deref();
        let collection_slot = self.allocate_stack_object("for_each_collection", 8);
        let cursor_slot = self.allocate_stack_object("for_each_cursor", 8);
        let remaining_slot = self.allocate_stack_object("for_each_remaining", 8);
        let local_slot = self.allocate_stack_object(name, 8);
        let entry_payload_slot = if map_entry_types.is_some() {
            Some(self.allocate_stack_object("for_each_map_entry", 16))
        } else {
            None
        };

        self.emit(abi::store_u64(
            &iterable_value.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x10", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));

        let loop_label = self.label("for_each_loop");
        let end_label = self.label("for_each_end");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&end_label));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        if let (Some(entry_payload_slot), Some((key_type, value_type))) =
            (entry_payload_slot, map_entry_types.as_ref())
        {
            self.emit(abi::load_u64(
                "x11",
                "x10",
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64(
                "x12",
                "x10",
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
            let key_value = self.emit_load_collection_payload(key_type, "x8", "x11", "x12")?;
            self.emit(abi::store_u64(
                &key_value,
                abi::stack_pointer(),
                entry_payload_slot,
            ));
            self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
            self.emit(abi::load_u64(
                "x11",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                "x12",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
            let item_value = self.emit_load_collection_payload(value_type, "x8", "x11", "x12")?;
            self.emit(abi::store_u64(
                &item_value,
                abi::stack_pointer(),
                entry_payload_slot + 8,
            ));
            self.emit(abi::add_immediate(
                "x11",
                abi::stack_pointer(),
                entry_payload_slot,
            ));
            self.emit(abi::store_u64("x11", abi::stack_pointer(), local_slot));
        } else {
            let item_value_type = item_value_type.ok_or_else(|| {
                format!(
                    "native code FOR EACH target '{}' is not a list",
                    iterable_value.type_
                )
            })?;
            self.emit(abi::load_u64(
                "x11",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                "x12",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
            let item_value =
                self.emit_load_collection_payload(item_value_type, "x8", "x11", "x12")?;
            self.emit(abi::store_u64(
                &item_value,
                abi::stack_pointer(),
                local_slot,
            ));
        }
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate("x10", "x10", COLLECTION_ENTRY_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::subtract_immediate("x9", "x9", 1));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));

        let previous = self.locals.insert(
            name.to_string(),
            LocalValue {
                type_: type_.to_string(),
                stack_offset: local_slot,
                constant: None,
            },
        );
        self.clear_local_constants();
        self.lower_ops(body)?;
        if let Some(previous) = previous {
            self.locals.insert(name.to_string(), previous);
        } else {
            self.locals.remove(name);
        }
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&end_label));
        self.clear_local_constants();
        Ok(())
    }
}
