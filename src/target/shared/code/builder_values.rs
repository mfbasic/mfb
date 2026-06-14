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
                Ok(ValueResult {
                    type_: type_.clone(),
                    location: register,
                    text: name.clone(),
                })
            }
            NirValue::Call { target, args } => {
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
            NirValue::RuntimeCall {
                helper,
                target,
                args,
            } => {
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
                let helper_args = if target == "io.pollInput" && args.is_empty() {
                    vec![NirValue::Const {
                        type_: "Integer".to_string(),
                        value: "0".to_string(),
                    }]
                } else {
                    args.clone()
                };
                let result_type = builtins::call_return_type_name(target)
                    .ok_or_else(|| format!("native runtime call '{target}' has no return type"))?;
                self.emit_runtime_helper_call(
                    target,
                    &runtime::symbol_for_call(*helper, target),
                    &helper_args,
                    result_type,
                )
            }
            NirValue::Constructor { type_, args } => {
                let arg_values = args
                    .iter()
                    .map(|arg| self.lower_value(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                let register = self.allocate_register()?;
                if self.type_model.record_fields.contains_key(type_) {
                    let object_offset = self.allocate_stack_object(type_, 8 * arg_values.len());
                    for (index, arg) in arg_values.iter().enumerate() {
                        self.emit(abi::store_u64(
                            &arg.location,
                            abi::stack_pointer(),
                            object_offset + 8 * index,
                        ));
                    }
                    self.emit(abi::add_immediate(
                        &register,
                        abi::stack_pointer(),
                        object_offset,
                    ));
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
                let object_offset = self.allocate_stack_object(type_, 8 * (arg_values.len() + 1));
                let tag_register = self.allocate_register()?;
                self.emit(abi::move_immediate(
                    &tag_register,
                    "UnionTag",
                    &tag.to_string(),
                ));
                self.emit(abi::store_u64(
                    &tag_register,
                    abi::stack_pointer(),
                    object_offset,
                ));
                for (index, arg) in arg_values.iter().enumerate() {
                    self.emit(abi::store_u64(
                        &arg.location,
                        abi::stack_pointer(),
                        object_offset + 8 * (index + 1),
                    ));
                }
                self.emit(abi::add_immediate(
                    &register,
                    abi::stack_pointer(),
                    object_offset,
                ));
                Ok(ValueResult {
                    type_: self
                        .type_model
                        .union_variants
                        .get(type_)
                        .cloned()
                        .unwrap_or_else(|| type_.clone()),
                    location: register,
                    text: format!("construct {type_}({})", join_texts(&arg_values)),
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
                if matches!(op.as_str(), "=" | "<>" | "<" | ">" | "<=" | ">=") {
                    return self.lower_comparison_binary(op, left, right);
                }
                self.lower_arithmetic_binary(op, left, right)
            }
            NirValue::Unary { op, operand } => {
                let operand = self.lower_value(operand)?;
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
                    "native code plan does not lower unary operator '{op}' for {} yet",
                    operand.type_
                ))
            }
            NirValue::ListLiteral { type_, values } => self.lower_list_literal(type_, values),
            NirValue::MapLiteral { type_, entries } => self.lower_map_literal(type_, entries),
        }
    }
}
