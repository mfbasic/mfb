use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_value(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        // Track the source location of the node being lowered so that any error
        // freshly created while lowering it (overflow, divide-by-zero, helper
        // failure, conversion failure) stamps a real `ErrorLoc`. The save/restore
        // ensures that after recursively lowering operands/arguments the outer
        // node's location is back in place before its own fallible emit runs.
        let saved_loc = self.current_loc;
        if let Some(loc) = value_loc(value) {
            self.current_loc = loc;
        }
        let result = self.lower_value_inner(value);
        self.current_loc = saved_loc;
        result
    }

    /// Lower a value that is being stored into a longer-lived or independently
    /// freed location (a `LET`/`MUT` binding, a global, a closure env, a returned
    /// value). plan-02 made every non-resource value a flat, pointer-free block,
    /// so a `memcpy` is a sound deep copy; this routine inserts that copy whenever
    /// the source is an **aliasing source** (a node that yields a pointer to an
    /// existing block rather than a fresh allocation). After copy-insertion every
    /// owned local owns an independent block, so plan-01 Phase 5 / plan-02 Phase 8
    /// can free each one exactly once at scope-drop with no double-free.
    ///
    /// Fresh-producing nodes (`Call`, `Constructor`, literals, `Binary`, …) and
    /// non-freeable types (scalars, resources, threads) are returned unchanged.
    pub(super) fn lower_value_owned(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let result = self.lower_value(value)?;
        if self.value_needs_owning_copy(value) && self.is_freeable_flat_value(&result.type_) {
            let copied = self.copy_flat_block(&result.type_, &result.location)?;
            return Ok(ValueResult {
                type_: result.type_,
                location: copied,
                text: result.text,
            });
        }
        Ok(result)
    }

    /// Whether lowering `value` yields a pointer this scope does **not** own — an
    /// alias/borrow into another value, or a *static* `String` constant in rodata
    /// (`static_string_value`). Either must be deep-copied into the arena before a
    /// binding/global/return can own it, so the eventual scope-drop `arena_free`
    /// reclaims a real arena block and never an aliased or static one.
    pub(super) fn value_needs_owning_copy(&self, value: &NirValue) -> bool {
        Self::value_is_aliasing_source(value) || self.static_string_value(value).is_some()
    }

    /// Whether lowering `value` yields a value whose lifetime is managed by the
    /// thread runtime, not by this scope: the result of a cross-thread data call
    /// (`thread::receive`/`read`/`waitFor`/`result`). Such a value lives in the
    /// thread's message plumbing and the worker arena that the runtime bulk-frees
    /// at teardown; scope-drop must not `arena_free` it (it may be a non-owning
    /// view, or already reclaimed on a cancel/timeout path), so its binding is not
    /// registered for an owned-value free — same exclusion principle as resources.
    pub(super) fn value_is_runtime_managed(value: &NirValue) -> bool {
        let target = match value {
            NirValue::Call { target, .. }
            | NirValue::CallResult { target, .. }
            | NirValue::RuntimeCall { target, .. } => target.as_str(),
            NirValue::MemberAccess { member, .. } if member == "result" => return true,
            _ => return false,
        };
        target.starts_with("thread.") || target.starts_with("thread::")
    }

    /// A NIR value node that yields a pointer to a **pre-existing** arena block
    /// (an alias / borrow) rather than a freshly allocated one. Storing such a
    /// value into an owned slot without copying would alias another owner, so
    /// [`lower_value_owned`](Self::lower_value_owned) deep-copies these.
    pub(super) fn value_is_aliasing_source(value: &NirValue) -> bool {
        matches!(
            value,
            NirValue::Local(_)
                | NirValue::Global { .. }
                | NirValue::Capture { .. }
                | NirValue::MemberAccess { .. }
                | NirValue::UnionExtract { .. }
                | NirValue::ResultValue { .. }
                | NirValue::ResultError { .. }
        )
    }

    /// Whether `type_` is a flat, arena-allocated value block that scope-drop
    /// frees own and `arena_free` reclaims in one call — `String`, a flat record,
    /// a flat data union, a flat collection, or a flat `Result`. Scalars (stored
    /// inline by value), resources, threads, and recursive/non-flat composites are
    /// excluded: they are never freed by the generic owned-value path.
    pub(super) fn is_freeable_flat_value(&self, type_: &str) -> bool {
        self.type_is_flat(type_)
            && (type_ == "String"
                || is_collection_type(type_)
                || type_.starts_with("Result OF ")
                || self.type_model.record_fields.contains_key(type_)
                || self.union_is_data(type_))
    }

    fn lower_value_inner(&mut self, value: &NirValue) -> Result<ValueResult, String> {
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
                let by_ref = local.by_ref;
                let register = self.allocate_register()?;
                self.emit(abi::load_u64(&register, abi::stack_pointer(), stack_offset));
                if by_ref {
                    // A reference local's slot holds a pointer to the parent
                    // binding's slot; deref it to read the live value/block
                    // pointer. For a scalar this yields the value; for a block it
                    // yields the block pointer (a borrow into the block).
                    self.emit(abi::load_u64(&register, &register, 0));
                }
                Ok(ValueResult {
                    type_,
                    location: register,
                    text: name.clone(),
                })
            }
            NirValue::LocalRef { name, type_ } => {
                // The address of the binding's slot (a borrow of the slot), used to
                // seed a non-escaping callback's env so the callback observes and
                // updates the live binding. The callback may
                // change the binding through this borrow, so any folded constant the
                // outer scope held for it is now stale and must be cleared, else a
                // later read folds to the pre-call value.
                let local = self
                    .locals
                    .get_mut(name)
                    .ok_or_else(|| format!("native code local ref '{name}' does not resolve"))?;
                let stack_offset = local.stack_offset;
                local.constant = None;
                let register = self.allocate_register()?;
                self.emit(abi::add_immediate(
                    &register,
                    abi::stack_pointer(),
                    stack_offset,
                ));
                Ok(ValueResult {
                    type_: type_.clone(),
                    location: register,
                    text: format!("&{name}"),
                })
            }
            NirValue::Global { name, type_ } => {
                let global = self.global_value(name)?;
                let value_type = if type_.is_empty() {
                    global.type_.clone()
                } else {
                    type_.clone()
                };
                let address = self.load_global_address(name)?;
                let register = self.allocate_register()?;
                self.emit(abi::load_u64(&register, &address, 0));
                Ok(ValueResult {
                    type_: value_type,
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
                self.emit(abi::add_page_offset(
                    &function_register,
                    &function_register,
                    &symbol,
                ));
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
                self.emit(abi::move_immediate(
                    abi::return_register(),
                    "Integer",
                    &CLOSURE_OBJECT_SIZE.to_string(),
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
                self.emit(abi::load_u64(
                    &function_register,
                    abi::stack_pointer(),
                    function_slot,
                ));
                self.emit(abi::store_u64(
                    &function_register,
                    "x1",
                    CLOSURE_OFFSET_CODE,
                ));
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
                self.emit(abi::add_page_offset(
                    &function_register,
                    &function_register,
                    &symbol,
                ));
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
                    self.emit(abi::move_immediate(
                        abi::return_register(),
                        "Integer",
                        &env_size,
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
                    self.emit(abi::move_register(&env_register, "x1"));
                    self.emit(abi::store_u64(
                        &env_register,
                        abi::stack_pointer(),
                        env_slot,
                    ));
                    for (index, capture) in captures.iter().enumerate() {
                        // The closure env outlives the capturing scope, so it must
                        // own each captured flat value independently (plan-02
                        // Phase 8). `lower_value_owned` deep-copies an aliasing
                        // source; its `arena_alloc` clobbers caller-saved scratch
                        // (incl. `env_register`), so reload the env from its slot.
                        let value = self.lower_value_owned(capture)?;
                        let env_register = self.allocate_register()?;
                        self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
                        self.emit(abi::store_u64(&value.location, &env_register, index * 8));
                    }
                    Some(env_slot)
                };
                let closure_register = self.allocate_register()?;
                let alloc_ok = self.label("closure_alloc_ok");
                self.emit(abi::move_immediate(
                    abi::return_register(),
                    "Integer",
                    &CLOSURE_OBJECT_SIZE.to_string(),
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
                self.emit(abi::load_u64(
                    &function_register,
                    abi::stack_pointer(),
                    function_slot,
                ));
                self.emit(abi::store_u64(
                    &function_register,
                    "x1",
                    CLOSURE_OFFSET_CODE,
                ));
                if let Some(env_slot) = env_slot {
                    let env_register = self.allocate_register()?;
                    self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
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
            NirValue::Capture { index, type_, .. } => {
                // Load the env slot's raw word. For a by-value capture this is the
                // value/block pointer; for a by-ref capture (`by_ref`) it is the
                // pointer to the parent binding's slot, which `Bind` installs into
                // a reference local that derefs on read/write.
                let register = self.allocate_register()?;
                self.emit(abi::load_u64(&register, CLOSURE_ENV_REGISTER, index * 8));
                Ok(ValueResult {
                    type_: type_.clone(),
                    location: register,
                    text: format!("capture[{index}]"),
                })
            }
            NirValue::Call { target, args, .. } => {
                if let Some(local) = self.locals.get(target).cloned() {
                    if local.type_.starts_with("FUNC(") {
                        let return_type = callable_return_type(&local.type_).ok_or_else(|| {
                            format!(
                                "native call through `{target}` has invalid callable type `{}`",
                                local.type_
                            )
                        })?;
                        let callable = ValueResult {
                            type_: local.type_,
                            location: {
                                let register = self.allocate_register()?;
                                self.emit(abi::load_u64(
                                    &register,
                                    abi::stack_pointer(),
                                    local.stack_offset,
                                ));
                                register
                            },
                            text: target.clone(),
                        };
                        return self.emit_function_value_call(
                            target,
                            &callable,
                            args,
                            Some(&return_type),
                        );
                    }
                }
                if let Some(result) = self.lower_fs_path_call(target, args)? {
                    return Ok(result);
                }
                if let Some(result) = self.lower_strings_package_call(target, args)? {
                    return Ok(result);
                }
                // Migrated `collections::`/`strings::` members arrive with their
                // qualified, dot-containing target (`collections.get`,
                // `strings.find`, ...). `native_builtin_target` maps these to the
                // shared bare lowering name and returns `None` for bare names, so a
                // user `FUNC get` is never hijacked by the native lowering
                // (plan-01-functions.md §5).
                let native = crate::builtins::native_builtin_target(target);
                if native == Some("contains") && args.len() == 2 {
                    return self.lower_collection_contains(args);
                }
                if native == Some("get") && args.len() == 2 {
                    return self.lower_collection_get(args);
                }
                if native == Some("getOr") && args.len() == 3 {
                    return self.lower_collection_get_or(args);
                }
                if native == Some("find") && (args.len() == 2 || args.len() == 3) {
                    return self.lower_find(args);
                }
                if target == "len" && args.len() == 1 {
                    return self.lower_len(&args[0]);
                }
                if native == Some("mid") && args.len() == 3 {
                    return self.lower_mid(args);
                }
                if native == Some("replace") && args.len() == 3 {
                    return self.lower_replace(args);
                }
                if native == Some("append") && args.len() == 2 {
                    return self.lower_collection_append(args);
                }
                if native == Some("prepend") && args.len() == 2 {
                    return self.lower_collection_prepend(args);
                }
                if native == Some("insert") && args.len() == 3 {
                    return self.lower_collection_insert(args);
                }
                if native == Some("removeAt") && args.len() == 2 {
                    return self.lower_collection_remove_at(args);
                }
                if native == Some("set") && args.len() == 3 {
                    return self.lower_collection_set(args);
                }
                if native == Some("removeKey") && args.len() == 2 {
                    return self.lower_collection_remove_key(args);
                }
                if native == Some("hasKey") && args.len() == 2 {
                    return self.lower_collection_has_key(args);
                }
                if native == Some("keys") && args.len() == 1 {
                    return self.lower_collection_keys(args);
                }
                if native == Some("values") && args.len() == 1 {
                    return self.lower_collection_values_builtin(args);
                }
                if native == Some("sum") && args.len() == 1 {
                    return self.lower_collection_sum(args);
                }
                if native == Some("forEach") && args.len() == 2 {
                    return self.lower_collection_for_each_call(args);
                }
                if native == Some("transform") && args.len() == 2 {
                    return self.lower_collection_transform_call(args);
                }
                if native == Some("filter") && args.len() == 2 {
                    return self.lower_collection_filter_call(args);
                }
                if native == Some("reduce") && args.len() == 3 {
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
                if let Some(function) = target.strip_prefix("bits.") {
                    return self.lower_bits_call(function, args);
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
            NirValue::CallResult { target, args, .. } => {
                if let Some(local) = self.locals.get(target).cloned() {
                    if local.type_.starts_with("FUNC(") {
                        let return_type = callable_return_type(&local.type_).ok_or_else(|| {
                            format!(
                                "native raw call through `{target}` has invalid callable type `{}`",
                                local.type_
                            )
                        })?;
                        let callable = ValueResult {
                            type_: local.type_,
                            location: {
                                let register = self.allocate_register()?;
                                self.emit(abi::load_u64(
                                    &register,
                                    abi::stack_pointer(),
                                    local.stack_offset,
                                ));
                                register
                            },
                            text: target.clone(),
                        };
                        return self
                            .emit_function_value_call(target, &callable, args, Some(&return_type))
                            .map(|result| ValueResult {
                                type_: format!("Result OF {return_type}"),
                                ..result
                            });
                    }
                }
                // An inline `TRAP` on an inline-lowered conversion built-in
                // (`toInt`, `toFloat`, `toFixed`, `toByte`) traps the raw
                // `Result`: lower the conversion inline but capture its error
                // instead of auto-propagating, then materialize the `Result`.
                if matches!(target.as_str(), "toInt" | "toFloat" | "toFixed" | "toByte")
                    && args.len() == 1
                {
                    return self.lower_inline_conversion_raw(target, &args[0]);
                }
                // An inline `TRAP` on a helper-backed built-in (`thread::waitFor`,
                // `fs::*`, …) traps the raw `Result`. The runtime helper leaves
                // that `Result` in the standard tag/value/error registers just
                // like a user-function call, so we invoke the helper without the
                // auto-propagate branch and materialize the raw `Result`.
                if let Some(helper) = runtime::helper_for_call(target) {
                    return self.lower_runtime_helper_call(helper, target, args, true);
                }
                // Backstop for the front-end gate (TYPE_INLINE_TRAP_ON_INLINED_BUILTIN):
                // an inline-lowered builtin has no standalone symbol, so the generic
                // raw path below would emit `bl <target>` to a non-existent symbol.
                // Typecheck rejects this case before codegen; if one still reaches
                // here, a future builtin was added without updating the gate — fail
                // loudly instead of miscompiling (plan-00-trap-fix.md §4.3).
                if crate::builtins::inline_trap_unsupported(target) {
                    return Err(format!(
                        "internal: inline TRAP reached inline-lowered builtin '{target}' \
                         without a raw lowering; front-end gate should have rejected it"
                    ));
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
                    .ok_or_else(|| {
                        format!("native raw result call '{target}' has no return type")
                    })?;
                let tag_slot = self.allocate_stack_object("raw_result_tag", 8);
                let value_slot = self.allocate_stack_object("raw_result_value", 8);
                let message_slot = self.allocate_stack_object("raw_result_message", 8);
                let source_slot = self.allocate_stack_object("raw_result_source", 8);
                let payload_slot = self.allocate_stack_object("raw_result_payload", 8);
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
                // Preserve the callee's error origin (x3) so an inline-trapped
                // error keeps its original source location.
                self.emit(abi::store_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    abi::stack_pointer(),
                    source_slot,
                ));
                self.emit(abi::load_u64("x9", abi::stack_pointer(), tag_slot));
                self.emit(abi::compare_immediate("x9", RESULT_OK_TAG));
                self.emit(abi::branch_ne(&wrap_error_label));
                self.emit(abi::load_u64("x9", abi::stack_pointer(), value_slot));
                self.emit(abi::store_u64("x9", abi::stack_pointer(), payload_slot));
                let ok_result =
                    self.emit_build_result_inline(tag_slot, &success_type, payload_slot)?;
                self.emit(abi::store_u64(
                    &ok_result,
                    abi::stack_pointer(),
                    result_slot,
                ));
                self.emit(abi::branch(&have_payload_label));
                self.emit(abi::label(&wrap_error_label));
                let error_register =
                    self.emit_build_error_inline(value_slot, message_slot, source_slot)?;
                self.emit(abi::store_u64(
                    &error_register,
                    abi::stack_pointer(),
                    payload_slot,
                ));
                let err_result = self.emit_build_result_inline(tag_slot, "Error", payload_slot)?;
                self.emit(abi::store_u64(
                    &err_result,
                    abi::stack_pointer(),
                    result_slot,
                ));
                self.emit(abi::label(&have_payload_label));
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
                ..
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
                self.lower_runtime_helper_call(*helper, target, args, false)
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
                if self.type_model.record_fields.contains_key(type_) {
                    // A record inlines its `String` fields into a trailing data
                    // region (the slot holds a block-relative offset); scalar and
                    // pointer fields stay inline at `8*index` (plan-02 §4.2).
                    let register = self.emit_build_inlined_record(type_, &arg_slots)?;
                    return Ok(ValueResult {
                        type_: type_.clone(),
                        location: register,
                        text: format!("construct {type_}({})", join_texts(&arg_values)),
                    });
                }
                let register = self.allocate_register()?;
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
                    .variants_for_union(&union_name)
                    .filter_map(|variant| self.type_model.union_variant_fields.get(variant))
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
                    self.emit(abi::store_u64(&zero_register, "x1", offset));
                }
                let tag_register = self.allocate_register()?;
                self.emit(abi::move_immediate(
                    &tag_register,
                    "UnionTag",
                    &tag.to_string(),
                ));
                self.emit(abi::store_u64(&tag_register, "x1", 0));
                for (index, slot) in arg_slots.iter().enumerate() {
                    self.emit(abi::load_u64("x9", abi::stack_pointer(), *slot));
                    self.emit(abi::store_u64("x9", "x1", 8 * (index + 1)));
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
                // A resource-union variant is a bare resource whose payload is
                // the resource pointer itself (one word at offset 8), not record
                // fields.
                let is_resource_variant = crate::builtins::is_resource_type(member_type);
                let fields = if is_resource_variant {
                    Vec::new()
                } else {
                    self.type_model
                        .record_fields
                        .get(member_type)
                        .cloned()
                        .ok_or_else(|| {
                            format!("native code union wrap member '{member_type}' is not a record")
                        })?
                };
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(member_type)
                    .copied()
                    .ok_or_else(|| {
                        format!("native code union variant '{member_type}' does not resolve")
                    })?;
                // Data variant: build a flat `{tag, size, variant-record-block}`
                // union, inlining the wrapped variant record at +16 (plan-02
                // §4.3). Resource variants fall through to the fixed
                // `{tag, resource-ptr}` layout below.
                if !is_resource_variant {
                    let _ = &fields;
                    let register =
                        self.emit_wrap_record_in_union(member_type, tag, wrapped_slot)?;
                    return Ok(ValueResult {
                        type_: union_type.clone(),
                        location: register,
                        text: format!("wrap {member_type} as {union_type}"),
                    });
                }
                // Payload words across all variants: a resource variant occupies
                // one word (the handle pointer); a record variant occupies its
                // field count.
                let max_payload = self
                    .type_model
                    .variants_for_union(union_type)
                    .map(|variant| {
                        if crate::builtins::is_resource_type(variant) {
                            1
                        } else {
                            self.type_model
                                .union_variant_fields
                                .get(variant)
                                .map(Vec::len)
                                .unwrap_or(0)
                        }
                    })
                    .max()
                    .unwrap_or(if is_resource_variant { 1 } else { fields.len() });
                let union_size = 8 * (1 + max_payload.max(1));
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
                if is_resource_variant || self.record_has_inline_data(member_type) {
                    // Resource variants store the handle pointer at +8. A record
                    // variant whose record has inlined String data is also stored
                    // as a single pointer at +8 (the inline-offset slots are
                    // meaningless once detached from the record's data region);
                    // the record stays a standalone flat block (plan-02 §4.2,
                    // union reshape deferred to Phase 4).
                    self.emit(abi::load_u64("x9", abi::stack_pointer(), wrapped_slot));
                    self.emit(abi::load_u64("x10", abi::stack_pointer(), result_slot));
                    self.emit(abi::store_u64("x9", "x10", 8));
                } else {
                    for (index, _) in fields.iter().enumerate() {
                        self.emit(abi::load_u64("x11", abi::stack_pointer(), wrapped_slot));
                        self.emit(abi::load_u64("x9", "x11", 8 * index));
                        self.emit(abi::load_u64("x10", abi::stack_pointer(), result_slot));
                        self.emit(abi::store_u64("x9", "x10", 8 * (index + 1)));
                    }
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
                // A resource-union variant's payload is the resource pointer
                // itself (offset 8): extracting it yields that pointer directly.
                if crate::builtins::is_resource_type(type_) {
                    let source = self.lower_value(value)?;
                    let register = self.allocate_register()?;
                    self.emit(abi::load_u64(&register, &source.location, 8));
                    return Ok(ValueResult {
                        type_: type_.clone(),
                        location: register,
                        text: format!("extract {type_} from {}", source.text),
                    });
                }
                // A data union inlines the active variant's flat record block at
                // +16 (plan-02 §4.3); the extracted record is a borrow into the
                // union at that offset.
                let source = self.lower_value(value)?;
                let register = self.allocate_register()?;
                self.emit(abi::add_immediate(&register, &source.location, 16));
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
                // The payload is inlined at +16 (plan-02 §4.3): a block payload
                // yields a borrow pointer into the Result; a scalar payload is the
                // 8-byte value.
                let register = self.allocate_register()?;
                if self.result_payload_is_block(&type_) {
                    self.emit(abi::add_immediate(&register, &result.location, 16));
                } else {
                    self.emit(abi::load_u64(&register, &result.location, 16));
                }
                Ok(ValueResult {
                    type_,
                    location: register,
                    text: "resultValue".to_string(),
                })
            }
            NirValue::ResultError { value } => {
                let result = self.lower_value(value)?;
                // The error payload (a flat Error block) is inlined at +16.
                let register = self.allocate_register()?;
                self.emit(abi::add_immediate(&register, &result.location, 16));
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
                _ if member == "result" => {
                    if let Some(output_type) = self.static_type_name(target).and_then(|type_| {
                        builtins::thread::parent_thread_output(&type_).map(str::to_string)
                    }) {
                        self.emit_raw_call(
                            &runtime::symbol_for_call(
                                runtime::RuntimeHelper::Thread,
                                "thread.waitFor",
                            ),
                            std::slice::from_ref(target.as_ref()),
                            "thread_result_arg",
                        )?;
                        return self.materialize_current_result(
                            &output_type,
                            "thread.result".to_string(),
                            true,
                        );
                    }
                    self.lower_field_access(target, member)
                }
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
            NirValue::Binary {
                op, left, right, ..
            } => {
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
            NirValue::Unary { op, operand, .. } => {
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
                if op == "-"
                    && matches!(
                        operand.type_.as_str(),
                        "Byte" | "Integer" | "Fixed" | "Float"
                    )
                {
                    return self.lower_numeric_unary_negation(operand);
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

    /// Lower an inline conversion built-in (`toInt`/`toFloat`/`toFixed`/`toByte`)
    /// for an inline `TRAP`: emit the normal inline conversion but capture its
    /// error return (which would otherwise auto-propagate) so the raw `Result`
    /// is left in the standard registers, then materialize it as a value.
    fn lower_inline_conversion_raw(
        &mut self,
        target: &str,
        arg: &NirValue,
    ) -> Result<ValueResult, String> {
        let success_type = builtins::call_return_type_name(target)
            .ok_or_else(|| format!("native raw conversion '{target}' has no return type"))?
            .to_string();
        let capture = self.label("raw_conversion_done");
        let previous = self.raw_result_capture.take();
        self.raw_result_capture = Some(capture.clone());
        let lowered = match target {
            "toInt" => self.lower_to_int(arg),
            "toFloat" => self.lower_to_float(arg),
            "toFixed" => self.lower_to_fixed(arg),
            "toByte" => self.lower_to_byte(arg),
            other => Err(format!("native raw conversion '{other}' is not supported")),
        };
        self.raw_result_capture = previous;
        let success = lowered?;
        // Success fall-through: tag the converted value as the `Ok` result.
        self.emit(abi::move_register(RESULT_VALUE_REGISTER, &success.location));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        self.emit(abi::label(&capture));
        self.materialize_current_result(&success_type, format!("callResult {target}"), false)
    }

    /// Lower a runtime-helper-backed call (`thread::*`, `fs::*`, `io::*`, …).
    /// With `raw = false` the call auto-unwraps/auto-propagates (the normal call
    /// site). With `raw = true` the call traps the raw `Result` for an inline
    /// `TRAP`: the helper outcome is materialized as a `Result OF <success>`
    /// value instead of propagating on error.
    /// `net::connectTcp` is overloaded on its first argument. The single-argument
    /// call is unambiguously the `Address` overload (typecheck rejects a lone
    /// host); the two-argument call is the `Address` overload only when the first
    /// argument is statically an `Address` (otherwise it is `host, port`).
    fn net_connect_is_address_form(&self, args: &[NirValue]) -> bool {
        match args.len() {
            1 => true,
            2 => {
                args.first()
                    .and_then(|arg| self.static_type_name(arg))
                    .as_deref()
                    == Some("Address")
            }
            _ => false,
        }
    }

    pub(super) fn lower_runtime_helper_call(
        &mut self,
        helper: runtime::RuntimeHelper,
        target: &str,
        args: &[NirValue],
        raw: bool,
    ) -> Result<ValueResult, String> {
        let mut helper_args = args.to_vec();
        if target == "io.input" && helper_args.is_empty() {
            helper_args.push(NirValue::Const {
                type_: "String".to_string(),
                value: String::new(),
            });
        } else if target == "io.pollInput" && helper_args.is_empty() {
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
        } else if target == "thread.send" && helper_args.len() == 2 {
            helper_args.push(NirValue::Const {
                type_: "Integer".to_string(),
                value: "0".to_string(),
            });
        } else if target == "thread.receive" && helper_args.len() == 1 {
            helper_args.push(NirValue::Const {
                type_: "Integer".to_string(),
                value: "0".to_string(),
            });
        } else if target == "net.lookup" && helper_args.len() == 1 {
            helper_args.push(NirValue::Const {
                type_: "Integer".to_string(),
                value: "0".to_string(),
            });
        } else if target == "net.connectTcp" {
            // `connectTcp(Address, timeoutMs?)` keeps its single Address argument
            // and is routed to the `net.connectTcpAddr` helper below; the
            // `connectTcp(host, port, timeoutMs?)` form fills the default timeout.
            let is_address = self.net_connect_is_address_form(args);
            let target_args = if is_address { 2 } else { 3 };
            while helper_args.len() < target_args {
                helper_args.push(NirValue::Const {
                    type_: "Integer".to_string(),
                    value: "0".to_string(),
                });
            }
        } else if target == "net.listenTcp" && helper_args.len() == 2 {
            helper_args.push(NirValue::Const {
                type_: "Integer".to_string(),
                value: "128".to_string(),
            });
        } else if matches!(target, "net.accept" | "net.poll") && helper_args.len() == 1 {
            helper_args.push(NirValue::Const {
                type_: "Integer".to_string(),
                value: "0".to_string(),
            });
        }
        let result_type = self
            .thread_runtime_return_type(target, &helper_args)
            .or_else(|| builtins::call_return_type_name(target).map(str::to_string))
            .ok_or_else(|| format!("native runtime call '{target}' has no return type"))?;
        let runtime_target = match target {
            "thread.send" => {
                let handle = self
                    .static_type_name(helper_args.first().ok_or_else(|| {
                        "native runtime thread.send missing handle argument".to_string()
                    })?)
                    .ok_or_else(|| {
                        "native runtime thread.send handle has unknown type".to_string()
                    })?;
                if builtins::thread::is_worker_thread_type(&handle) {
                    "thread.emit"
                } else {
                    "thread.send"
                }
            }
            "thread.receive" => {
                let handle = self
                    .static_type_name(helper_args.first().ok_or_else(|| {
                        "native runtime thread.receive missing handle argument".to_string()
                    })?)
                    .ok_or_else(|| {
                        "native runtime thread.receive handle has unknown type".to_string()
                    })?;
                if builtins::thread::is_worker_thread_type(&handle) {
                    "thread.receive"
                } else {
                    "thread.read"
                }
            }
            // Resource plane, split by direction like the data plane above. A
            // `thread::transfer` (lowered to `transferResource`) on a worker handle
            // writes the outbound resource queue (`emitResource`); on a parent
            // handle it writes the inbound queue (`transferResource`). A
            // `thread::accept` (lowered to `acceptResource`) on a worker handle
            // reads the inbound queue (`acceptResource`); on a parent handle it
            // reads the outbound queue (`readResource`).
            "thread.transferResource" => {
                let handle = self
                    .static_type_name(helper_args.first().ok_or_else(|| {
                        "native runtime thread.transfer missing handle argument".to_string()
                    })?)
                    .ok_or_else(|| {
                        "native runtime thread.transfer handle has unknown type".to_string()
                    })?;
                if builtins::thread::is_worker_thread_type(&handle) {
                    "thread.emitResource"
                } else {
                    "thread.transferResource"
                }
            }
            "thread.acceptResource" => {
                let handle = self
                    .static_type_name(helper_args.first().ok_or_else(|| {
                        "native runtime thread.accept missing handle argument".to_string()
                    })?)
                    .ok_or_else(|| {
                        "native runtime thread.accept handle has unknown type".to_string()
                    })?;
                if builtins::thread::is_worker_thread_type(&handle) {
                    "thread.acceptResource"
                } else {
                    "thread.readResource"
                }
            }
            "net.connectTcp" => {
                if self.net_connect_is_address_form(args) {
                    "net.connectTcpAddr"
                } else {
                    "net.connectTcp"
                }
            }
            _ => target,
        };
        self.emit_runtime_helper_call(
            runtime_target,
            &runtime::symbol_for_call(helper, runtime_target),
            &helper_args,
            &result_type,
            raw,
        )
    }
}

/// Extract the source location carried by an error-originating NIR value, if any.
fn value_loc(value: &NirValue) -> Option<NirSourceLoc> {
    match value {
        NirValue::Call { loc, .. }
        | NirValue::CallResult { loc, .. }
        | NirValue::RuntimeCall { loc, .. }
        | NirValue::Binary { loc, .. }
        | NirValue::Unary { loc, .. } => Some(*loc),
        _ => None,
    }
}
