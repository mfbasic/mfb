use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_value(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        if let Some(string_value) = self.static_string_value(value) {
            let register = self.load_string_constant(&string_value)?;
            return Ok(ValueResult {
                type_: "String".to_string(),
                location: register,
                text: format!("String({string_value})"),
            });
        }
        match value {
            NirValue::Const { type_, value } => {
                let register = self.allocate_register()?;
                if type_ == "String" {
                    self.emit_load_string_constant(&register, value)?;
                } else {
                    let immediate = native_immediate_value(type_, value)?;
                    self.emit(abi::move_immediate(&register, type_, &immediate));
                }
                Ok(ValueResult {
                    type_: type_.clone(),
                    location: register,
                    text: format!("{type_}({value})"),
                })
            }
            NirValue::Local(name) => {
                if self.type_model.union_variants.contains_key(name) {
                    return Ok(ValueResult {
                        type_: "VariantTag".to_string(),
                        location: name.clone(),
                        text: name.clone(),
                    });
                }
                let local = self
                    .locals
                    .get(name)
                    .ok_or_else(|| format!("native code local '{name}' does not resolve"))?;
                let type_ = local.type_.clone();
                let stack_offset = local.stack_offset;
                let register = self.allocate_register()?;
                self.emit(abi::load_u64(&register, abi::stack_pointer(), stack_offset));
                Ok(ValueResult {
                    type_,
                    location: register,
                    text: name.clone(),
                })
            }
            NirValue::FunctionRef { name, type_ } => {
                let symbol = builtin_function_symbol_for_type(name, type_)
                    .or_else(|| self.function_symbols.get(name).cloned())
                    .unwrap_or_else(|| name.clone());
                let function_register = self.allocate_register()?;
                self.emit(abi::load_page_address(&function_register, &symbol));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: symbol.clone(),
                    kind: "page21".to_string(),
                    binding: "data".to_string(),
                    library: None,
                });
                self.emit(abi::add_page_offset(&function_register, &function_register, &symbol));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: symbol,
                    kind: "pageoff12".to_string(),
                    binding: "data".to_string(),
                    library: None,
                });
                let function_slot = self.allocate_stack_object("function_ref_code", 8);
                self.emit(abi::store_u64(
                    &function_register,
                    abi::stack_pointer(),
                    function_slot,
                ));
                let closure_register = self.allocate_register()?;
                let alloc_ok = self.label("function_ref_alloc_ok");
                self.emit(abi::move_immediate(abi::return_register(), "Integer", &CLOSURE_OBJECT_SIZE.to_string()));
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
                self.emit(abi::load_u64(
                    &function_register,
                    abi::stack_pointer(),
                    function_slot,
                ));
                self.emit(abi::store_u64(&function_register, "x1", CLOSURE_OFFSET_CODE));
                self.emit(abi::store_u64("x31", "x1", CLOSURE_OFFSET_ENV));
                self.emit(abi::move_register(&closure_register, "x1"));
                Ok(ValueResult {
                    type_: type_.clone(),
                    location: closure_register,
                    text: name.clone(),
                })
            }
            NirValue::Closure {
                name,
                type_,
                captures,
            } => {
                let symbol = self
                    .function_symbols
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                let function_register = self.allocate_register()?;
                self.emit(abi::load_page_address(&function_register, &symbol));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: symbol.clone(),
                    kind: "page21".to_string(),
                    binding: "data".to_string(),
                    library: None,
                });
                self.emit(abi::add_page_offset(&function_register, &function_register, &symbol));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: symbol,
                    kind: "pageoff12".to_string(),
                    binding: "data".to_string(),
                    library: None,
                });
                let function_slot = self.allocate_stack_object("closure_code", 8);
                self.emit(abi::store_u64(
                    &function_register,
                    abi::stack_pointer(),
                    function_slot,
                ));
                let env_slot = if captures.is_empty() {
                    None
                } else {
                    let env_register = self.allocate_register()?;
                    let env_slot = self.allocate_stack_object("closure_env", 8);
                    let alloc_ok = self.label("closure_env_alloc_ok");
                    let env_size = (captures.len() * 8).to_string();
                    self.emit(abi::move_immediate(abi::return_register(), "Integer", &env_size));
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
                    self.emit(abi::move_register(&env_register, "x1"));
                    self.emit(abi::store_u64(
                        &env_register,
                        abi::stack_pointer(),
                        env_slot,
                    ));
                    for (index, capture) in captures.iter().enumerate() {
                        let value = self.lower_value(capture)?;
                        self.emit(abi::store_u64(&value.location, &env_register, index * 8));
                    }
                    Some(env_slot)
                };
                let closure_register = self.allocate_register()?;
                let alloc_ok = self.label("closure_alloc_ok");
                self.emit(abi::move_immediate(abi::return_register(), "Integer", &CLOSURE_OBJECT_SIZE.to_string()));
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
                self.emit(abi::load_u64(
                    &function_register,
                    abi::stack_pointer(),
                    function_slot,
                ));
                self.emit(abi::store_u64(&function_register, "x1", CLOSURE_OFFSET_CODE));
                if let Some(env_slot) = env_slot {
                    let env_register = self.allocate_register()?;
                    self.emit(abi::load_u64(
                        &env_register,
                        abi::stack_pointer(),
                        env_slot,
                    ));
                    self.emit(abi::store_u64(&env_register, "x1", CLOSURE_OFFSET_ENV));
                } else {
                    self.emit(abi::store_u64("x31", "x1", CLOSURE_OFFSET_ENV));
                }
                self.emit(abi::move_register(&closure_register, "x1"));
                Ok(ValueResult {
                    type_: type_.clone(),
                    location: closure_register,
                    text: name.clone(),
                })
            }
            NirValue::Capture { index, type_ } => {
                let register = self.allocate_register()?;
                self.emit(abi::load_u64(&register, CLOSURE_ENV_REGISTER, index * 8));
                Ok(ValueResult {
                    type_: type_.clone(),
                    location: register,
                    text: format!("capture[{index}]"),
                })
            }
            NirValue::Call { target, args } => {
                if let Some(local) = self.locals.get(target).cloned() {
                    if local.type_.starts_with("FUNC(") {
                        let return_type = callable_return_type(&local.type_).ok_or_else(|| {
                            format!("native call through `{target}` has invalid callable type `{}`", local.type_)
                        })?;
                        let callable = ValueResult {
                            type_: local.type_,
                            location: {
                                let register = self.allocate_register()?;
                                self.emit(abi::load_u64(&register, abi::stack_pointer(), local.stack_offset));
                                register
                            },
                            text: target.clone(),
                        };
                        return self.emit_function_value_call(target, &callable, args, Some(&return_type));
                    }
                }
                if let Some(result) = self.lower_fs_path_call(target, args)? {
                    return Ok(result);
                }
                if let Some(result) = self.lower_strings_package_call(target, args)? {
                    return Ok(result);
                }
                if target == "contains" && args.len() == 2 {
                    return self.lower_collection_contains(args);
                }
                if target == "get" && args.len() == 2 {
                    return self.lower_collection_get(args);
                }
                if target == "getOr" && args.len() == 3 {
                    return self.lower_collection_get_or(args);
                }
                if target == "find" && (args.len() == 2 || args.len() == 3) {
                    return self.lower_find(args);
                }
                if target == "len" && args.len() == 1 {
                    return self.lower_len(&args[0]);
                }
                if target == "mid" && args.len() == 3 {
                    return self.lower_mid(args);
                }
                if target == "replace" && args.len() == 3 {
                    return self.lower_replace(args);
                }
                if target == "append" && args.len() == 2 {
                    return self.lower_collection_append(args);
                }
                if target == "prepend" && args.len() == 2 {
                    return self.lower_collection_prepend(args);
                }
                if target == "insert" && args.len() == 3 {
                    return self.lower_collection_insert(args);
                }
                if target == "removeAt" && args.len() == 2 {
                    return self.lower_collection_remove_at(args);
                }
                if target == "set" && args.len() == 3 {
                    return self.lower_collection_set(args);
                }
                if target == "removeKey" && args.len() == 2 {
                    return self.lower_collection_remove_key(args);
                }
                if target == "hasKey" && args.len() == 2 {
                    return self.lower_collection_has_key(args);
                }
                if target == "keys" && args.len() == 1 {
                    return self.lower_collection_keys(args);
                }
                if target == "values" && args.len() == 1 {
                    return self.lower_collection_values_builtin(args);
                }
                if target == "sum" && args.len() == 1 {
                    return self.lower_collection_sum(args);
                }
                if target == "forEach" && args.len() == 2 {
                    return self.lower_collection_for_each_call(args);
                }
                if target == "transform" && args.len() == 2 {
                    return self.lower_collection_transform_call(args);
                }
                if target == "filter" && args.len() == 2 {
                    return self.lower_collection_filter_call(args);
                }
                if target == "reduce" && args.len() == 3 {
                    return self.lower_collection_reduce_call(args);
                }
                if target == "toString" && (args.len() == 1 || args.len() == 2) {
                    return self.lower_to_string(args);
                }
                if target == "typeName" && args.len() == 1 {
                    let type_name = self.static_type_name(&args[0]).ok_or_else(|| {
                        "native code cannot determine typeName argument type".to_string()
                    })?;
                    let register = self.load_string_constant(&type_name)?;
                    return Ok(ValueResult {
                        type_: "String".to_string(),
                        location: register,
                        text: format!("typeName({type_name})"),
                    });
                }
                if target == "toInt" && args.len() == 1 {
                    return self.lower_to_int(&args[0]);
                }
                if target == "toFloat" && args.len() == 1 {
                    return self.lower_to_float(&args[0]);
                }
                if target == "toFixed" && args.len() == 1 {
                    return self.lower_to_fixed(&args[0]);
                }
                if target == "toByte" && args.len() == 1 {
                    return self.lower_to_byte(&args[0]);
                }
                if target == "isNumeric" && args.len() == 1 {
                    return self.lower_is_numeric(&args[0]);
                }
                if let Some(function) = target.strip_prefix("math.") {
                    return self.lower_math_call(function, args);
                }
                if target == "isEven" && args.len() == 1 {
                    return self.lower_integer_parity_predicate("isEven", &args[0], false);
                }
                if target == "isOdd" && args.len() == 1 {
                    return self.lower_integer_parity_predicate("isOdd", &args[0], true);
                }
                if matches!(target.as_str(), "isPositive" | "isNegative" | "isZero")
                    && args.len() == 1
                {
                    return self.lower_numeric_filter_predicate(target, &args[0]);
                }
                if matches!(target.as_str(), "isEmpty" | "isNotEmpty") && args.len() == 1 {
                    return self.lower_empty_filter_predicate(target, &args[0]);
                }
                let symbol = self
                    .function_symbols
                    .get(target)
                    .cloned()
                    .unwrap_or_else(|| target.to_string());
                self.emit_call(target, &symbol, args, None)
            }
            NirValue::CallResult { target, args } => {
                if let Some(local) = self.locals.get(target).cloned() {
                    if local.type_.starts_with("FUNC(") {
                        let return_type = callable_return_type(&local.type_).ok_or_else(|| {
                            format!("native raw call through `{target}` has invalid callable type `{}`", local.type_)
                        })?;
                        let callable = ValueResult {
                            type_: local.type_,
                            location: {
                                let register = self.allocate_register()?;
                                self.emit(abi::load_u64(&register, abi::stack_pointer(), local.stack_offset));
                                register
                            },
                            text: target.clone(),
                        };
                        return self.emit_function_value_call(target, &callable, args, Some(&return_type))
                            .map(|result| ValueResult {
                                type_: format!("Result OF {return_type}"),
                                ..result
                            });
                    }
                }
                let symbol = self
                    .function_symbols
                    .get(target)
                    .cloned()
                    .unwrap_or_else(|| target.to_string());
                let success_type = self
                    .functions
                    .get(target)
                    .map(|function| function.returns.clone())
                    .or_else(|| self.package_return_types.get(target).cloned())
                    .or_else(|| builtins::call_return_type_name(target).map(str::to_string))
                    .ok_or_else(|| format!("native raw result call '{target}' has no return type"))?;
                let tag_slot = self.allocate_stack_object("raw_result_tag", 8);
                let value_slot = self.allocate_stack_object("raw_result_value", 8);
                let message_slot = self.allocate_stack_object("raw_result_message", 8);
                let payload_slot = self.allocate_stack_object("raw_result_payload", 8);
                let alloc_ok = self.label("result_construct_alloc_ok");
                let error_alloc_ok = self.label("result_error_alloc_ok");
                let wrap_error_label = self.label("result_wrap_error");
                let have_payload_label = self.label("result_have_payload");
                let result_slot = self.allocate_stack_object("raw_result", 8);
                self.emit_call(target, &symbol, args, Some(&success_type))?;
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
                self.emit(abi::load_u64("x9", abi::stack_pointer(), tag_slot));
                self.emit(abi::compare_immediate("x9", RESULT_OK_TAG));
                self.emit(abi::branch_ne(&wrap_error_label));
                self.emit(abi::load_u64("x9", abi::stack_pointer(), value_slot));
                self.emit(abi::store_u64("x9", abi::stack_pointer(), payload_slot));
                self.emit(abi::branch(&have_payload_label));
                self.emit(abi::label(&wrap_error_label));
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
                self.emit(abi::branch_eq(&error_alloc_ok));
                self.emit_allocation_error_return()?;
                self.emit(abi::label(&error_alloc_ok));
                self.emit(abi::load_u64("x9", abi::stack_pointer(), value_slot));
                self.emit(abi::store_u64("x9", "x1", 0));
                self.emit(abi::load_u64("x9", abi::stack_pointer(), message_slot));
                self.emit(abi::store_u64("x9", "x1", 8));
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
                    text: format!("callResult {target}"),
                })
            }
            NirValue::RuntimeCall {
                helper,
                target,
                args,
            } => {
                if let Some(result) = self.lower_fs_path_call(target, args)? {
                    return Ok(result);
                }
                if let Some(result) = self.lower_strings_package_call(target, args)? {
                    return Ok(result);
                }
                if target == "isEven" && args.len() == 1 {
                    return self.lower_integer_parity_predicate("isEven", &args[0], false);
                }
                if target == "isOdd" && args.len() == 1 {
                    return self.lower_integer_parity_predicate("isOdd", &args[0], true);
                }
                if matches!(target.as_str(), "isPositive" | "isNegative" | "isZero")
                    && args.len() == 1
                {
                    return self.lower_numeric_filter_predicate(target, &args[0]);
                }
                if matches!(target.as_str(), "isEmpty" | "isNotEmpty") && args.len() == 1 {
                    return self.lower_empty_filter_predicate(target, &args[0]);
                }
                if target == "typeName" && args.len() == 1 {
                    let type_name = self.static_type_name(&args[0]).ok_or_else(|| {
                        "native code cannot determine typeName argument type".to_string()
                    })?;
                    let register = self.load_string_constant(&type_name)?;
                    return Ok(ValueResult {
                        type_: "String".to_string(),
                        location: register,
                        text: format!("typeName({type_name})"),
                    });
                }
                let mut helper_args = args.clone();
                if target == "io.pollInput" && helper_args.is_empty() {
                    helper_args.push(NirValue::Const {
                        type_: "Integer".to_string(),
                        value: "0".to_string(),
                    });
                } else if target == "thread.start" {
                    while helper_args.len() < 4 {
                        helper_args.push(NirValue::Const {
                            type_: "Integer".to_string(),
                            value: "64".to_string(),
                        });
                    }
                } else if matches!(target.as_str(), "thread.send" | "thread.emit")
                    && helper_args.len() == 2
                {
                    helper_args.push(NirValue::Const {
                        type_: "Integer".to_string(),
                        value: "0".to_string(),
                    });
                } else if target == "thread.receive" && helper_args.len() == 1 {
                    helper_args.push(NirValue::Const {
                        type_: "Integer".to_string(),
                        value: "0".to_string(),
                    });
                }
                let result_type = self
                    .thread_runtime_return_type(target, &helper_args)
                    .or_else(|| builtins::call_return_type_name(target).map(str::to_string))
                    .ok_or_else(|| format!("native runtime call '{target}' has no return type"))?;
                self.emit_runtime_helper_call(
                    target,
                    &runtime::symbol_for_call(*helper, target),
                    &helper_args,
                    &result_type,
                )
            }
            NirValue::Constructor { type_, args } => {
                let mut arg_values = Vec::new();
                let mut arg_slots = Vec::new();
                for arg in args {
                    let value = self.lower_value(arg)?;
                    let slot = self.allocate_stack_object("constructor_arg", 8);
                    self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
                    arg_values.push(value);
                    arg_slots.push(slot);
                }
                let register = self.allocate_register()?;
                if type_ == "Error" {
                    let result_slot = self.allocate_stack_object("error_result", 8);
                    let alloc_ok = self.label("error_construct_alloc_ok");
                    self.emit(abi::move_immediate(
                        abi::return_register(),
                        "Integer",
                        "16",
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
                    for (index, slot) in arg_slots.iter().take(2).enumerate() {
                        self.emit(abi::load_u64("x9", abi::stack_pointer(), *slot));
                        self.emit(abi::store_u64("x9", "x1", 8 * index));
                    }
                    self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
                    return Ok(ValueResult {
                        type_: type_.clone(),
                        location: register,
                        text: format!("construct {type_}({})", join_texts(&arg_values)),
                    });
                }
                if self.type_model.record_fields.contains_key(type_) {
                    let result_slot = self.allocate_stack_object("record_result", 8);
                    let alloc_ok = self.label("record_construct_alloc_ok");
                    let object_size = 8 * arg_values.len();
                    self.emit(abi::move_immediate(
                        abi::return_register(),
                        "Integer",
                        &object_size.to_string(),
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
                    for (index, slot) in arg_slots.iter().enumerate() {
                        self.emit(abi::load_u64("x9", abi::stack_pointer(), *slot));
                        self.emit(abi::store_u64(
                            "x9",
                            "x1",
                            8 * index,
                        ));
                    }
                    self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
                    return Ok(ValueResult {
                        type_: type_.clone(),
                        location: register,
                        text: format!("construct {type_}({})", join_texts(&arg_values)),
                    });
                }
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(type_)
                    .copied()
                    .ok_or_else(|| {
                        format!("native code union variant '{type_}' does not resolve")
                    })?;
                let union_name = self
                    .type_model
                    .union_variants
                    .get(type_)
                    .cloned()
                    .unwrap_or_else(|| type_.clone());
                let union_size = self
                    .type_model
                    .union_variants
                    .iter()
                    .filter(|(_, candidate_union)| *candidate_union == &union_name)
                    .filter_map(|(variant, _)| self.type_model.union_variant_fields.get(variant))
                    .map(Vec::len)
                    .max()
                    .map(|max_fields| 8 * (1 + max_fields))
                    .unwrap_or(8 * (arg_values.len() + 1));
                let result_slot = self.allocate_stack_object("union_result", 8);
                let alloc_ok = self.label("union_construct_alloc_ok");
                self.emit(abi::move_immediate(
                    abi::return_register(),
                    "Integer",
                    &union_size.to_string(),
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
                let zero_register = self.allocate_register()?;
                self.emit(abi::move_immediate(&zero_register, "Integer", "0"));
                for offset in (0..union_size).step_by(8) {
                    self.emit(abi::store_u64(
                        &zero_register,
                        "x1",
                        offset,
                    ));
                }
                let tag_register = self.allocate_register()?;
                self.emit(abi::move_immediate(
                    &tag_register,
                    "UnionTag",
                    &tag.to_string(),
                ));
                self.emit(abi::store_u64(
                    &tag_register,
                    "x1",
                    0,
                ));
                for (index, slot) in arg_slots.iter().enumerate() {
                    self.emit(abi::load_u64("x9", abi::stack_pointer(), *slot));
                    self.emit(abi::store_u64(
                        "x9",
                        "x1",
                        8 * (index + 1),
                    ));
                }
                self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
                Ok(ValueResult {
                    type_: union_name,
                    location: register,
                    text: format!("construct {type_}({})", join_texts(&arg_values)),
                })
            }
            NirValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => {
                let wrapped = self.lower_value(value)?;
                let wrapped_slot = self.allocate_stack_object("union_wrap_source", 8);
                self.emit(abi::store_u64(
                    &wrapped.location,
                    abi::stack_pointer(),
                    wrapped_slot,
                ));
                let fields = self
                    .type_model
                    .record_fields
                    .get(member_type)
                    .cloned()
                    .ok_or_else(|| {
                        format!("native code union wrap member '{member_type}' is not a record")
                    })?;
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(member_type)
                    .copied()
                    .ok_or_else(|| {
                        format!("native code union variant '{member_type}' does not resolve")
                    })?;
                let union_size = self
                    .type_model
                    .union_variants
                    .iter()
                    .filter(|(_, candidate_union)| *candidate_union == union_type)
                    .filter_map(|(variant, _)| self.type_model.union_variant_fields.get(variant))
                    .map(Vec::len)
                    .max()
                    .map(|max_fields| 8 * (1 + max_fields))
                    .unwrap_or(8 * (fields.len() + 1));
                let result_slot = self.allocate_stack_object("union_result", 8);
                let alloc_ok = self.label("union_construct_alloc_ok");
                self.emit(abi::move_immediate(
                    abi::return_register(),
                    "Integer",
                    &union_size.to_string(),
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
                let zero_register = self.allocate_register()?;
                self.emit(abi::move_immediate(&zero_register, "Integer", "0"));
                for offset in (0..union_size).step_by(8) {
                    self.emit(abi::store_u64(&zero_register, "x1", offset));
                }
                let tag_register = self.allocate_register()?;
                self.emit(abi::move_immediate(
                    &tag_register,
                    "UnionTag",
                    &tag.to_string(),
                ));
                self.emit(abi::store_u64(&tag_register, "x1", 0));
                for (index, _) in fields.iter().enumerate() {
                    self.emit(abi::load_u64("x11", abi::stack_pointer(), wrapped_slot));
                    self.emit(abi::load_u64(
                        "x9",
                        "x11",
                        8 * index,
                    ));
                    self.emit(abi::load_u64("x10", abi::stack_pointer(), result_slot));
                    self.emit(abi::store_u64(
                        "x9",
                        "x10",
                        8 * (index + 1),
                    ));
                }
                let register = self.allocate_register()?;
                self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
                Ok(ValueResult {
                    type_: union_type.clone(),
                    location: register,
                    text: format!("wrap {member_type} as {union_type}"),
                })
            }
            NirValue::UnionExtract { type_, value } => {
                let fields = self
                    .type_model
                    .record_fields
                    .get(type_)
                    .cloned()
                    .ok_or_else(|| {
                        format!("native code union extract target '{type_}' is not a record")
                    })?;
                let source = self.lower_value(value)?;
                let source_slot = self.allocate_stack_object("union_extract_source", 8);
                self.emit(abi::store_u64(
                    &source.location,
                    abi::stack_pointer(),
                    source_slot,
                ));
                let result_slot = self.allocate_stack_object("union_extract_result", 8);
                let alloc_ok = self.label("union_extract_alloc_ok");
                self.emit(abi::move_immediate(
                    abi::return_register(),
                    "Integer",
                    &(8 * fields.len()).to_string(),
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
                for (index, _) in fields.iter().enumerate() {
                    self.emit(abi::load_u64("x11", abi::stack_pointer(), source_slot));
                    self.emit(abi::load_u64(
                        "x9",
                        "x11",
                        8 * (index + 1),
                    ));
                    self.emit(abi::store_u64(
                        "x9",
                        "x1",
                        8 * index,
                    ));
                }
                let register = self.allocate_register()?;
                self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
                Ok(ValueResult {
                    type_: type_.clone(),
                    location: register,
                    text: format!("extract {type_} from {}", source.text),
                })
            }
            NirValue::ResultIsOk { value } => {
                let result = self.lower_value(value)?;
                let register = self.allocate_register()?;
                let ok_label = self.label("result_is_ok_true");
                let end_label = self.label("result_is_ok_end");
                self.emit(abi::load_u64("x9", &result.location, 0));
                self.emit(abi::compare_immediate("x9", RESULT_OK_TAG));
                self.emit(abi::branch_eq(&ok_label));
                self.emit(abi::move_immediate(&register, "Boolean", "0"));
                self.emit(abi::branch(&end_label));
                self.emit(abi::label(&ok_label));
                self.emit(abi::move_immediate(&register, "Boolean", "1"));
                self.emit(abi::label(&end_label));
                Ok(ValueResult {
                    type_: "Boolean".to_string(),
                    location: register,
                    text: "resultIsOk".to_string(),
                })
            }
            NirValue::ResultValue { value } => {
                let result = self.lower_value(value)?;
                let type_ = result
                    .type_
                    .strip_prefix("Result OF ")
                    .ok_or_else(|| {
                        format!(
                            "native RESULT_VALUE requires raw Result input, got `{}`",
                            result.type_
                        )
                    })?
                    .to_string();
                let register = self.allocate_register()?;
                self.emit(abi::load_u64(&register, &result.location, 8));
                Ok(ValueResult {
                    type_,
                    location: register,
                    text: "resultValue".to_string(),
                })
            }
            NirValue::ResultError { value } => {
                let result = self.lower_value(value)?;
                let register = self.allocate_register()?;
                self.emit(abi::load_u64(&register, &result.location, 8));
                Ok(ValueResult {
                    type_: "Error".to_string(),
                    location: register,
                    text: "resultError".to_string(),
                })
            }
            NirValue::WithUpdate {
                type_,
                target,
                updates,
            } => self.lower_with_update(type_, target, updates),
            NirValue::MemberAccess { target, member } => match target.as_ref() {
                NirValue::Local(type_name) => {
                    if let Some(ordinal) = self
                        .type_model
                        .enum_members
                        .get(&(type_name.clone(), member.clone()))
                        .copied()
                    {
                        let register = self.allocate_register()?;
                        self.emit(abi::move_immediate(
                            &register,
                            "EnumOrdinal",
                            &ordinal.to_string(),
                        ));
                        return Ok(ValueResult {
                            type_: type_name.clone(),
                            location: register,
                            text: format!("{type_name}.{member}"),
                        });
                    }
                    self.lower_field_access(target, member)
                }
                _ => self.lower_field_access(target, member),
            },
            NirValue::Binary { op, left, right } => {
                if op == "&" {
                    return self.lower_string_concat(left, right);
                }
                if matches!(op.as_str(), "AND" | "OR" | "XOR") {
                    return self.lower_boolean_binary(op, left, right);
                }
                if matches!(op.as_str(), "=" | "<>" | "<" | ">" | "<=" | ">=") {
                    return self.lower_comparison_binary(op, left, right);
                }
                self.lower_arithmetic_binary(op, left, right)
            }
            NirValue::Unary { op, operand } => {
                let operand = self.lower_value(operand)?;
                if op == "NOT" && operand.type_ == "Boolean" {
                    let register = self.allocate_register()?;
                    let true_label = self.label("bool_not_true");
                    let done_label = self.label("bool_not_done");
                    self.emit(abi::compare_immediate(&operand.location, "0"));
                    self.emit(abi::branch_eq(&true_label));
                    self.emit(abi::move_immediate(&register, "Boolean", "false"));
                    self.emit(abi::branch(&done_label));
                    self.emit(abi::label(&true_label));
                    self.emit(abi::move_immediate(&register, "Boolean", "true"));
                    self.emit(abi::label(&done_label));
                    return Ok(ValueResult {
                        type_: "Boolean".to_string(),
                        location: register,
                        text: format!("(NOT {})", operand.text),
                    });
                }
                if op == "-" && operand.type_ == "Integer" {
                    let min_register = self.allocate_register()?;
                    let overflow_label = self.label("integer_unary_overflow");
                    let ok_label = self.label("integer_unary_ok");
                    self.emit(abi::move_immediate(
                        &min_register,
                        "Integer",
                        "9223372036854775808",
                    ));
                    self.emit(abi::compare_registers(&operand.location, &min_register));
                    self.emit(abi::branch_eq(&overflow_label));
                    let zero = self.allocate_register()?;
                    self.emit(abi::move_immediate(&zero, "Integer", "0"));
                    let register = self.allocate_register()?;
                    self.emit(abi::subtract_registers(&register, &zero, &operand.location));
                    self.emit(abi::branch(&ok_label));
                    self.emit(abi::label(&overflow_label));
                    self.emit_overflow_return()?;
                    self.emit(abi::label(&ok_label));
                    return Ok(ValueResult {
                        type_: "Integer".to_string(),
                        location: register,
                        text: format!("(-{})", operand.text),
                    });
                }
                Err(format!(
                    "native code plan does not lower unary operator '{op}' for {} yet while lowering native function '{}'",
                    operand.type_,
                    self.current_symbol
                ))
            }
            NirValue::ListLiteral { type_, values } => self.lower_list_literal(type_, values),
            NirValue::MapLiteral { type_, entries } => self.lower_map_literal(type_, entries),
        }
    }
}
