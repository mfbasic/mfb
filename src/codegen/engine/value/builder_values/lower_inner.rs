//! plan-176-U: `lower_value_inner`, the per-`NirValue` lowering. The entry points and shared
//! types live in `engine/value/builder_values/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_value_inner(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let scratch9_reg = self.temporary_vreg();
        let scratch10_reg = self.temporary_vreg();
        let scratch9 = &scratch9_reg;
        let scratch10 = &scratch10_reg;
        if let Some(string_value) = self.static_string_value(value) {
            let register = self.load_string_constant(&string_value)?;
            return Ok(ValueResult {
                origin: None,
                type_: ParameterType::String,
                location: Operand::from(register),
                text: format!("String({string_value})"),
            });
        }
        match value {
            NirValue::Const { type_, value } => {
                // A Const String is always intercepted by `static_string_value`
                // above (`static_string_value`, builder_value_semantics/static_eval.rs, returns Some for it), so this
                // arm only reaches non-String scalar constants. bug-175 C: the dead
                // `*type_ == ParameterType::String` branch was removed.
                let register = self.allocate_register();
                let immediate = native_immediate_value(&type_, value)?;
                self.emit(abi::move_immediate(
                    &register,
                    &abi::immediate_class(&type_),
                    &immediate,
                ));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: format!("{type_}({value})"),
                })
            }
            NirValue::Local(name) => {
                if self
                    .type_model
                    .union_variants
                    .contains_key(&ParameterType::declared(name))
                {
                    return Ok(ValueResult {
                        origin: None,
                        type_: ParameterType::named("VariantTag"),
                        location: Operand::from(name.clone()),
                        text: name.clone(),
                    });
                }
                // A promoted small-vector local lives in its lanes, not a slot
                // (plan-01-vector): reconstruct a register-native view from them.
                // The lanes are shared register values (vectors are immutable), so
                // every read and any later materialization sees the same value.
                if let Some((type_, lanes)) = self.promoted_vector_locals.get(name).cloned() {
                    return Ok(self.make_vector_native(&type_, lanes));
                }
                // A loop-promoted float local lives in an FP register, not its
                // slot (plan-03 Stage D part 2). Its FP register *is* the value's
                // home under the `d`-native value model, so return it directly —
                // no GPR materialization (plan-01 float-dnative).
                if let Some(d) = self.promoted_float_locals.get(name).cloned() {
                    return Ok(ValueResult {
                        origin: None,
                        type_: ParameterType::Float,
                        location: Operand::from(d),
                        text: name.clone(),
                    });
                }
                let local = self
                    .locals
                    .get(name)
                    .ok_or_else(|| format!("native code local '{name}' does not resolve"))?;
                let type_ = local.type_.clone();
                let type_name = type_.clone();
                let stack_offset = local.stack_offset;
                let by_ref = local.by_ref;
                // A non-aliased `Float` local loads straight into an FP register
                // (`ldr d`) under the `d`-native model, so it feeds float
                // arithmetic with no `ldr x` + `fmov` shuttle (plan-01
                // float-dnative). A `by_ref` local needs a pointer deref first, so
                // it stays on the GPR path.
                if matches!(type_, ParameterType::Float) && !by_ref {
                    let d = self.allocate_fp_register();
                    self.emit(abi::load_double(&d, abi::stack_pointer(), stack_offset));
                    return Ok(ValueResult {
                        origin: None,
                        type_: type_name.clone(),
                        location: Operand::from(d),
                        text: name.clone(),
                    });
                }
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, abi::stack_pointer(), stack_offset));
                if by_ref {
                    // A reference local's slot holds a pointer to the parent
                    // binding's slot; deref it to read the live value/block
                    // pointer. For a scalar this yields the value; for a block it
                    // yields the block pointer (an alias into the block).
                    self.emit(abi::load_u64(&register, &register, 0));
                }
                Ok(ValueResult {
                    origin: None,
                    type_: type_name.clone(),
                    location: Operand::from(register),
                    text: name.clone(),
                })
            }
            NirValue::LocalRef { name, type_ } => {
                // The address of the binding's slot (a reference to the slot), used to
                // seed a non-escaping callback's env so the callback observes and
                // updates the live binding. The callback may
                // change the binding through this reference, so any folded constant the
                // outer scope held for it is now stale and must be cleared, else a
                // later read folds to the pre-call value.
                let local = self
                    .locals
                    .get_mut(name)
                    .ok_or_else(|| format!("native code local ref '{name}' does not resolve"))?;
                let stack_offset = local.stack_offset;
                local.constant = None;
                let register = self.allocate_register();
                self.emit(abi::add_immediate(
                    &register,
                    abi::stack_pointer(),
                    stack_offset,
                ));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: format!("&{name}"),
                })
            }
            NirValue::Global { name, type_ } => {
                let global = self.global_value(name)?;
                let value_type = if type_.name().is_empty() {
                    global.type_.clone()
                } else {
                    type_.clone()
                };
                let address = self.load_global_address(name)?;
                // A `Float` global loads straight into an FP register under the
                // `d`-native model (plan-01 float-dnative), mirroring the local
                // load path.
                if matches!(value_type, ParameterType::Float) {
                    let d = self.allocate_fp_register();
                    self.emit(abi::load_double(&d, &address, 0));
                    return Ok(ValueResult {
                        origin: None,
                        type_: value_type.clone(),
                        location: Operand::from(d),
                        text: name.clone(),
                    });
                }
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, &address, 0));
                Ok(ValueResult {
                    origin: None,
                    type_: value_type.clone(),
                    location: Operand::from(register),
                    text: name.clone(),
                })
            }
            NirValue::FunctionRef { name, type_ } => {
                // A no-capture function value is the address of a STATIC closure
                // descriptor (`{code = &func, env = 0}`) — one per function, in BSS,
                // populated once at startup (see `collect_function_value_refs` +
                // the entry). Load its address instead of arena-allocating a fresh
                // descriptor on every evaluation, so a lambda in a loop no longer
                // grows the arena (bug-78). All indirect-call/env-access consumers
                // read `{code, env}` off this pointer exactly as before.
                let symbol = builtin_function_symbol_for_type(name, &type_)
                    .or_else(|| self.function_symbols.get(name).cloned())
                    .unwrap_or_else(|| name.clone());
                let desc_symbol = closure_descriptor_symbol(&symbol);
                let closure_register = self.allocate_register();
                self.emit(abi::load_page_address(&closure_register, &desc_symbol));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: desc_symbol.clone(),
                    kind: RelocIntent::DataAddrHi,
                    binding: "data".to_string(),
                    library: None,
                });
                self.emit(abi::add_page_offset(
                    &closure_register,
                    &closure_register,
                    &desc_symbol,
                ));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: desc_symbol,
                    kind: RelocIntent::DataAddrLo,
                    binding: "data".to_string(),
                    library: None,
                });
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(closure_register),
                    text: name.clone(),
                })
            }
            NirValue::Closure {
                name,
                type_,
                captures,
            } if captures.is_empty() => {
                // bug-781: a capture-less closure owns nothing, so it is the static
                // descriptor a `FunctionRef` loads — `env = 0` then marks exactly
                // the function values no holder frees or copies.
                self.lower_value_inner(&NirValue::FunctionRef {
                    name: name.clone(),
                    type_: type_.clone(),
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
                let ops_symbol = closure_env_ops_symbol(&symbol);
                let function_register = self.allocate_register();
                self.emit(abi::load_page_address(&function_register, &symbol));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: symbol.clone(),
                    kind: RelocIntent::DataAddrHi,
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
                    kind: RelocIntent::DataAddrLo,
                    binding: "data".to_string(),
                    library: None,
                });
                let function_slot = self.allocate_stack_object("closure_code", 8);
                self.emit(abi::store_u64(
                    &function_register,
                    abi::stack_pointer(),
                    function_slot,
                ));
                // plan-142-G: a lambda that borrows this frame's self-update
                // scratch finds its slot's address one word past the captures. The
                // prescan gave this frame a scratch of its own for it; a frame that
                // only borrows one cannot lend it on.
                let borrowed_scratch = match self.scratch_closure_captures(name) {
                    None => None,
                    Some(_) => match (self.self_update_scratch, self.self_update_scratch_env) {
                        (Some(slot), None) => Some(slot),
                        _ => {
                            return Err(format!(
                                "native closure `{name}` borrows a self-update scratch its \
                                 creator does not own"
                            ))
                        }
                    },
                };
                // plan-146-G: a lambda that self-updates a by-ref-captured `String`
                // works on the OWNER's block, so it shares the owner's capacity
                // shadow through one further environment word per such capture.
                let shared_shadows: Vec<(String, usize)> =
                    crate::codegen::collection::assign::string_self_update::string_shadow_captures(
                        self.functions,
                        name,
                    );
                let env_register = self.allocate_register();
                let env_slot = self.allocate_stack_object("closure_env", 8);
                let alloc_ok = self.label("closure_env_alloc_ok");
                let shares_resource_base =
                    crate::codegen::collection::assign::string_self_update::lambda_shares_resource_base(
                        self.functions,
                        name,
                    );
                let auxiliary_words = usize::from(borrowed_scratch.is_some())
                    + shared_shadows.len()
                    + usize::from(shares_resource_base);
                let env_size =
                    (closure_env_word_offset(captures.len() + auxiliary_words)).to_string();
                self.emit(abi::move_immediate(
                    abi::return_register(),
                    "Integer",
                    &env_size,
                ));
                self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
                self.emit_arena_alloc_call();
                self.emit(abi::branch_eq(&alloc_ok));
                self.raise_error_bare("ErrOutOfMemory")?;
                self.emit(abi::label(&alloc_ok));
                self.emit(abi::move_register(&env_register, abi::mfb_return(1)));
                self.emit(abi::store_u64(
                    &env_register,
                    abi::stack_pointer(),
                    env_slot,
                ));
                // bug-781: word 0 names this `LAMBDA`'s environment routine, so a
                // holder that knows only the `FUNC` type can copy or drop it.
                let ops_register = self.allocate_register();
                self.emit(abi::load_page_address(&ops_register, &ops_symbol));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: ops_symbol.clone(),
                    kind: RelocIntent::DataAddrHi,
                    binding: "data".to_string(),
                    library: None,
                });
                self.emit(abi::add_page_offset(
                    &ops_register,
                    &ops_register,
                    &ops_symbol,
                ));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: ops_symbol.clone(),
                    kind: RelocIntent::DataAddrLo,
                    binding: "data".to_string(),
                    library: None,
                });
                let env_register = self.allocate_register();
                self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
                self.emit(abi::store_u64(
                    &ops_register,
                    &env_register,
                    CLOSURE_ENV_OFFSET_OPS,
                ));
                let mut words = Vec::with_capacity(captures.len());
                let mut resource_words = Vec::new();
                for (index, capture) in captures.iter().enumerate() {
                    // The closure env outlives the capturing scope, so it must
                    // own each captured value independently (plan-02 Phase 8).
                    // `lower_value_owned` deep-copies an aliasing source; its
                    // `arena_alloc` clobbers caller-saved scratch (incl.
                    // `env_register`), so reload the env from its slot.
                    let value = self.lower_value_owned(capture)?;
                    let word = self.closure_env_word(capture, &value.type_);
                    // bug-843: the environment owns its own copy of a captured
                    // resource-union box (`ClosureEnvWord::ResourceUnionBox`).
                    let value = if word == ClosureEnvWord::ResourceUnionBox {
                        let box_slot =
                            self.spill_to_slot("closure_capture_union_box", &value.location);
                        let copied = self.emit_copy_resource_union_box(box_slot)?;
                        ValueResult {
                            location: Operand::from(copied),
                            ..value
                        }
                    } else {
                        value
                    };
                    words.push(word);
                    // bug-843: a by-value capture that reaches a resource is a
                    // reference the closure carries, walked by the routine's
                    // resource operations (`closure_carry`).
                    let by_ref = matches!(
                        capture,
                        NirValue::LocalRef { .. } | NirValue::Capture { by_ref: true, .. }
                    );
                    if !by_ref && self.type_carries_resources(&value.type_) {
                        resource_words.push((index, value.type_.clone()));
                    }
                    // Observation boundary: a `Float` captured into the
                    // closure env is read back when the closure runs, so it
                    // must be finite (plan-17).
                    self.observe_float(capture, &value)?;
                    // Materialize a `d`-native float before storing it into
                    // the closure env (plan-01 float-dnative).
                    let value = self.materialize_float(value)?;
                    let env_register = self.allocate_register();
                    self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
                    self.emit(abi::store_u64(
                        &value.location,
                        &env_register,
                        closure_env_word_offset(index),
                    ));
                }
                if let Some(scratch_slot) = borrowed_scratch {
                    let holder = self.allocate_register();
                    self.emit(abi::add_immediate(
                        &holder,
                        abi::stack_pointer(),
                        scratch_slot,
                    ));
                    let env_register = self.allocate_register();
                    self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
                    self.emit(abi::store_u64(
                        &holder,
                        &env_register,
                        closure_env_word_offset(captures.len()),
                    ));
                }
                for (position, (local, _)) in shared_shadows.iter().enumerate() {
                    // The owner's shadow slot, which its own prescan allocated
                    // for exactly this capture (`prescan_string_self_appends`).
                    let Some(shadow_slot) = self.string_capacity_slots.get(local).copied() else {
                        return Err(format!(
                            "native closure `{name}` shares the capacity shadow of `{local}`, \
                             which its creator does not have"
                        ));
                    };
                    let holder = self.allocate_register();
                    self.emit(abi::add_immediate(
                        &holder,
                        abi::stack_pointer(),
                        shadow_slot,
                    ));
                    let env_register = self.allocate_register();
                    self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
                    let index = captures.len() + usize::from(borrowed_scratch.is_some()) + position;
                    self.emit(abi::store_u64(
                        &holder,
                        &env_register,
                        closure_env_word_offset(index),
                    ));
                }
                if shares_resource_base {
                    // plan-146-G: the address of this frame's `os::appResourcePath`
                    // base cache, which the lambda fills on its first call and
                    // this frame owns and frees.
                    let Some(base_slot) = self.string_app_resource_base else {
                        return Err(format!(
                            "native closure `{name}` shares an os.appResourcePath base cache \
                             its creator does not have"
                        ));
                    };
                    let holder = self.allocate_register();
                    self.emit(abi::add_immediate(&holder, abi::stack_pointer(), base_slot));
                    let env_register = self.allocate_register();
                    self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
                    let index = captures.len()
                        + usize::from(borrowed_scratch.is_some())
                        + shared_shadows.len();
                    self.emit(abi::store_u64(
                        &holder,
                        &env_register,
                        closure_env_word_offset(index),
                    ));
                }
                self.record_closure_env_layout(ClosureEnvLayout {
                    symbol: ops_symbol,
                    words,
                    auxiliary_words,
                    resource_words,
                })?;
                let closure_register = self.allocate_register();
                let alloc_ok = self.label("closure_alloc_ok");
                // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not return_register().
                self.emit(abi::move_immediate(
                    abi::c_arg(0),
                    "Integer",
                    &CLOSURE_OBJECT_SIZE.to_string(),
                ));
                self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
                self.emit_arena_alloc_call();
                self.emit(abi::branch_eq(&alloc_ok));
                self.raise_error_bare("ErrOutOfMemory")?;
                self.emit(abi::label(&alloc_ok));
                self.emit(abi::load_u64(
                    &function_register,
                    abi::stack_pointer(),
                    function_slot,
                ));
                self.emit(abi::store_u64(
                    &function_register,
                    abi::mfb_return(1),
                    CLOSURE_OFFSET_CODE,
                ));
                let env_register = self.allocate_register();
                self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
                self.emit(abi::store_u64(
                    &env_register,
                    abi::mfb_return(1),
                    CLOSURE_OFFSET_ENV,
                ));
                self.emit(abi::move_register(&closure_register, abi::mfb_return(1)));
                // bug-781: the object is a fresh value its consumer owns — a binding,
                // a field, an element, a return claims it; an argument leaves it to
                // the statement-scope free (`register_pending_temp`, as for any
                // owned value).
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(closure_register),
                    text: name.clone(),
                })
            }
            NirValue::Capture { index, type_, .. } => {
                // Load the env slot's raw word. For a by-value capture this is the
                // value/block pointer; for a by-ref capture (`by_ref`) it is the
                // pointer to the parent binding's slot, which `Bind` installs into
                // a reference local that derefs on read/write.
                let register = self.allocate_register();
                self.emit(abi::load_u64(
                    &register,
                    CLOSURE_ENV_REGISTER,
                    closure_env_word_offset(*index),
                ));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: format!("capture[{index}]"),
                })
            }
            NirValue::Call { target, args, loc } => {
                // plan-01-vector: inline the pure-arithmetic `vector::` ops
                // (`scale`, `dot`) over Float2/3/4 as their equivalent constructor
                // / sum expression, eliminating the out-of-line FUNC call. The
                // rewrite reproduces the `vector_package.mfb` body's exact
                // expression tree, so the result and its finiteness observation are
                // bit-identical; a non-simple (re-evaluation-unsafe) operand or any
                // un-inlined op falls back to the package FUNC call below.
                if let Some(result) = self.try_inline_vector_op(target, args, *loc)? {
                    return Ok(result);
                }
                // plan-39 A4: lower the internal `#collections_slice$T` helper
                // (window/chunks) as a native contiguous-range copy instead of the
                // element-by-element FUNC. Non-list / unsupported types fall back.
                if let Some(result) = self.try_inline_slice_op(target, args)? {
                    return Ok(result);
                }
                // zip migrated to Implementation::Mfb.fast_path (func_zip.rs),
                // consulted by try_mfb_fast_path below.
                if let Some(local) = self.locals.get(target).cloned() {
                    // Any callable, `ISOLATED` included: an isolated function value
                    // is the same capture-free code pointer, and it is called the
                    // same way. Matching only the plain form sent a call through an
                    // `ISOLATED FUNC` local down the direct-call path, which emitted
                    // a relocation to a function named like the local (found with
                    // bug-822).
                    if matches!(local.type_, ParameterType::Func(..)) {
                        let return_type_typed = typed_callable_return_type(&local.type_)
                            .expect("guarded by the Func match above")
                            .clone();
                        let return_type = return_type_typed.clone();
                        let callable = ValueResult {
                            origin: None,
                            type_: local.type_.clone(),
                            location: {
                                let register = self.allocate_register();
                                self.emit(abi::load_u64(
                                    &register,
                                    abi::stack_pointer(),
                                    local.stack_offset,
                                ));
                                Operand::from(register)
                            },
                            text: target.clone(),
                        };
                        return self.emit_function_value_call(
                            target,
                            &callable,
                            args,
                            Some(&return_type.name()),
                        );
                    }
                }
                // A top-level (global) binding holding a function value is called
                // indirectly too (bug-198): load the function pointer from the
                // global's arena slot, mirroring the local FUNC-value path above.
                if let Some(global) = self.globals.get(target).cloned() {
                    if matches!(global.type_, ParameterType::Func(..)) {
                        let return_type = typed_callable_return_type(&global.type_)
                            .expect("guarded by the Func match above")
                            .name()
                            .into_owned();
                        let address = self.load_global_address(target)?;
                        let register = self.allocate_register();
                        self.emit(abi::load_u64(&register, &address, 0));
                        let callable = ValueResult {
                            origin: None,
                            type_: global.type_.clone(),
                            location: Operand::from(register),
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
                // `strings::`/`astrings::` native members migrated to the clean-room
                // registry, reached below through the inline dual-path (plan-99 PART
                // B/C). `strings::` lowers per member in its own `func_*.rs` (the
                // `builder_strings_*` carrier + `lower_strings_package_call` dispatcher
                // were collapsed to the func_/gen_ shape); `astrings::` members are
                // `Body::abi_inline`, reached through `try_abi_inline_lower`,
                // still delegating to the shared `lower_astrings_package_call` carrier
                // in `gen_astrings.rs`.
                // Migrated `collections::`/`strings::` members arrive with their
                // qualified, dot-containing target (`collections.get`,
                // `strings.find`, ...). `native_builtin_target` maps these to the
                // shared bare lowering name and returns `None` for bare names, so a
                // user `FUNC get` is never hijacked by the native lowering
                // (plan-01-functions.md §5).
                // plan-95: prefer a migrated function's own `abi_inline` lowering
                // over the legacy ladder below.
                if let Some(result) = self.try_abi_inline_lower(target, args) {
                    return result;
                }
                let native = crate::codegen::builtins::native_builtin_target(target);
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
                // Mfb dual-path: a migrated source-generic member's native fast path
                // (`Implementation::Mfb.fast_path`, owned by its `func_*.rs`) is
                // consulted here. It self-gates on the monomorph instantiation and
                // either lowers (returns here) or declines, falling through to the
                // un-migrated per-member arms below and then the monomorphized `.mfb`
                // body. As each member migrates, its per-member arm below is deleted.
                if let Some(result) = self.try_mfb_fast_path(target, args) {
                    return result;
                }
                // findLastIndex migrated to Implementation::Mfb.fast_path
                // (func_find_last_index.rs), consulted by try_mfb_fast_path above.
                if target == "toString" && (args.len() == 1 || args.len() == 2) {
                    return self.lower_to_string(args);
                }
                if target == "typeName" && args.len() == 1 {
                    let type_name = self.static_type_name_for_fold(&args[0]).ok_or_else(|| {
                        "native code cannot determine typeName argument type".to_string()
                    })?;
                    let register = self.load_string_constant(&type_name.name())?;
                    return Ok(ValueResult {
                        origin: None,
                        type_: ParameterType::String,
                        location: Operand::from(register),
                        text: format!("typeName({type_name})"),
                    });
                }
                if target == "toInt" && (1..=2).contains(&args.len()) {
                    return self.lower_to_int(args);
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
                if target == "toMoney" && args.len() == 1 {
                    return self.lower_to_money(&args[0]);
                }
                if target == "toScalar" && args.len() == 1 {
                    return self.lower_to_scalar(&args[0]);
                }
                if target == "isNumeric" && args.len() == 1 {
                    return self.lower_is_numeric(&args[0]);
                }
                // `math.*` migrated to the clean-room registry (`Body::abi_inline`),
                // reached above through `try_abi_inline_lower` — no `math.` name
                // predicate here. The shared `lower_math_call` carrier stays; each
                // member's `func_*.rs` shim calls it, and `builder_vector_inline` reaches
                // the scalar `math.sqrt`/`math.clamp` it emits as `NirValue::Call` through
                // the same self-lowering dual-path.
                // `money.*` migrated to the clean-room registry (`Body::abi_inline`),
                // reached on the `Call` path through `try_abi_inline_lower`; the
                // explicit intrinsics below cover this `RuntimeCall` arm.
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
                // plan-147-D: name the call node so `emit_call` can ask `handover`
                // whether this SITE hands anything over. Restored afterwards so a
                // nested call cannot inherit an outer site's answer.
                let previous_call = self
                    .current_call_key
                    .replace(crate::codegen::engine::analysis::handover::call_key(value));
                let result = self.emit_call(target, &symbol, args, None);
                self.current_call_key = previous_call;
                result
            }
            NirValue::CallResult { target, args, .. } => {
                if let Some(local) = self.locals.get(target).cloned() {
                    if matches!(local.type_, ParameterType::Func(..)) {
                        let return_type_typed = typed_callable_return_type(&local.type_)
                            .expect("guarded by the Func match above")
                            .clone();
                        let return_type = return_type_typed.clone();
                        let callable = ValueResult {
                            origin: None,
                            type_: local.type_.clone(),
                            location: {
                                let register = self.allocate_register();
                                self.emit(abi::load_u64(
                                    &register,
                                    abi::stack_pointer(),
                                    local.stack_offset,
                                ));
                                Operand::from(register)
                            },
                            text: target.clone(),
                        };
                        // bug-448: the inline-TRAP machinery consumes a boxed
                        // `Result` (tag/value/error), so materialize one from the
                        // call's standard result registers exactly as the direct
                        // user-function raw path below does — the previous code
                        // returned the raw success *value*, which the machinery
                        // then dereferenced as a `Result`-object pointer and
                        // segfaulted.
                        let tag_slot = self.allocate_stack_object("raw_result_tag", 8);
                        let value_slot = self.allocate_stack_object("raw_result_value", 8);
                        let message_slot = self.allocate_stack_object("raw_result_message", 8);
                        let source_slot = self.allocate_stack_object("raw_result_source", 8);
                        let unboxed = self.unboxed_trap_active.take();
                        self.emit_function_value_call_raw(&callable, args)?;
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
                            source_slot,
                        ));
                        // bug-841: `EXIT PROGRAM` is not the handler's to catch.
                        self.emit_inline_trap_program_exit_passthrough(tag_slot, value_slot)?;
                        // bug-778: an unboxed temporary keeps the callee's block —
                        // an owning `Bind` of the untrapped call owns it too (a
                        // `FUNC` value is never a param borrow, bug-569).
                        if let Some(unboxed) = unboxed {
                            return self.finish_unboxed_trap_result(
                                unboxed,
                                tag_slot,
                                value_slot,
                                message_slot,
                                source_slot,
                                UnboxedPayloadSource::Owned,
                                TrappedErrorSource::CalleeRegister,
                                format!("callResult {target}"),
                            );
                        }
                        let payload_slot = self.allocate_stack_object("raw_result_payload", 8);
                        let wrap_error_label = self.label("result_wrap_error");
                        let have_payload_label = self.label("result_have_payload");
                        let result_slot = self.allocate_stack_object("raw_result", 8);
                        self.emit(abi::load_u64(scratch9, abi::stack_pointer(), tag_slot));
                        self.emit(abi::compare_immediate(scratch9, RESULT_OK_TAG));
                        self.emit(abi::branch_ne(&wrap_error_label));
                        self.emit(abi::load_u64(scratch9, abi::stack_pointer(), value_slot));
                        self.emit(abi::store_u64(scratch9, abi::stack_pointer(), payload_slot));
                        let ok_result =
                            self.emit_build_result_inline(tag_slot, &return_type, payload_slot)?;
                        self.emit(abi::store_u64(
                            &ok_result,
                            abi::stack_pointer(),
                            result_slot,
                        ));
                        self.emit(abi::branch(&have_payload_label));
                        self.emit(abi::label(&wrap_error_label));
                        // bug-565: adopt a parked `ERR_BLOCK` instead of orphaning
                        // it, and free the payload the `Result` copied. Shared with
                        // the direct-callee path below and with
                        // `materialize_current_result`.
                        self.emit_trapped_error_result(
                            scratch9,
                            tag_slot,
                            value_slot,
                            message_slot,
                            source_slot,
                            payload_slot,
                            result_slot,
                            TrappedErrorSource::CalleeRegister,
                        )?;
                        self.emit(abi::label(&have_payload_label));
                        return Ok(self.fresh_trapped_result_value(
                            result_slot,
                            return_type_typed.clone(),
                            format!("callResult {target}"),
                        ));
                    }
                }
                // An inline `TRAP` on an inline-lowered conversion built-in
                // (`toInt`, `toFloat`, `toFixed`, `toByte`) traps the raw
                // `Result`: lower the conversion inline but capture its error
                // instead of auto-propagating, then materialize the `Result`.
                if matches!(
                    target.as_str(),
                    "toInt" | "toFloat" | "toFixed" | "toByte" | "toMoney" | "toScalar"
                ) && (args.len() == 1 || (target == "toInt" && args.len() == 2))
                {
                    return self.lower_inline_conversion_raw(target, args);
                }
                // bug-486: the fallibility census is name-keyed except for the
                // overloads whose argument type decides it — today only
                // `toString(<List OF Byte>)`, whose UTF-8 decode raises
                // `ErrEncoding`. `static_type_name_for_fold` is the widest arg-type
                // oracle the builder has (it falls back to the registry's typed
                // return-type resolver for a nested call); this is a read-only
                // question with none of the fast-path coupling that keeps
                // `static_type_name` itself narrow. An argument it cannot type
                // answers `Unknown`, which lands on the name-keyed verdict.
                let inline_arg_types: Vec<ParameterType> =
                    if crate::codegen::builtins::inline_builtin_fallibility_depends_on_args(target)
                    {
                        args.iter()
                            .map(|arg| {
                                self.static_type_name_for_fold(arg)
                                    .unwrap_or(ParameterType::Unknown)
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                // An inline `TRAP` on a fallible inline member (`collections::get`,
                // `strings::mid`, …) traps the raw `Result` the same way (plan-21-B):
                // run the member's normal inline lowering under a raw capture so its
                // domain error redirects to the capture point instead of propagating,
                // then materialize the `Result OF <success>`.
                if crate::codegen::builtins::inline_builtin_raw_supported(target, &inline_arg_types)
                {
                    return self.lower_inline_builtin_raw(target, args);
                }
                // An inline `TRAP` on a provably-infallible inline built-in
                // (`len`, `toString`, `typeName`, `bits::*`, the pure-query/growth
                // collection members) is uniform surface (plan-26-A): the member
                // emits no error exit, so lower it normally and wrap the success as
                // an always-`Ok` `Result` for the inline-TRAP machinery. The handler
                // is dead code (front-end warns `TYPE_INLINE_TRAP_DEAD_HANDLER`).
                // bug-679: `NoTypeKinds` deliberately, and unreachable for the one
                // name that would care. Every `toInt` form returned above through
                // `lower_inline_conversion_raw`, so the enum overload never
                // arrives here; and codegen holds no declaration table anyway, so
                // the oracle's `false` is both correct and the over-approximating
                // side for anything that did.
                if crate::codegen::builtins::inline_builtin_is_infallible(
                    target,
                    &inline_arg_types,
                    &crate::codegen::builtins::NoTypeKinds,
                ) {
                    return self.lower_inline_infallible_raw(target, args);
                }
                // An inline `TRAP` on a helper-backed built-in (`thread::waitFor`,
                // `fs::*`, …) traps the raw `Result`. The runtime helper leaves
                // that `Result` in the standard tag/value/error registers just
                // like a user-function call, so we invoke the helper without the
                // auto-propagate branch and materialize the raw `Result`.
                if let Some(helper) = runtime::helper_for_call(target) {
                    return self.lower_runtime_helper_call(helper, target, args, true);
                }
                // Future-proofing backstop: an inline-lowered builtin has no
                // standalone symbol, so the generic raw path below would emit
                // `bl <target>` to a non-existent symbol. After plan-26 every inline
                // builtin is either raw-supported or infallible (both handled above),
                // so this never fires today; it fails loudly if a *future* inline
                // builtin is added to `native_builtin_target` without a raw or
                // infallible lowering, instead of miscompiling.
                if crate::codegen::builtins::inline_trap_unsupported(
                    target,
                    &inline_arg_types,
                    &crate::codegen::builtins::NoTypeKinds,
                ) {
                    return Err(format!(
                        "internal: inline TRAP reached inline-lowered builtin '{target}' \
                         without a raw or infallible lowering; add one to \
                         lower_inline_builtin_raw / lower_inline_infallible_raw"
                    ));
                }
                let symbol = self
                    .function_symbols
                    .get(target)
                    .cloned()
                    .unwrap_or_else(|| target.to_string());
                let success_type_typed = self
                    .functions
                    .get(target)
                    .map(|function| function.returns.clone())
                    .or_else(|| self.package_return_types.get(target).cloned())
                    .or_else(|| {
                        // A static descriptor name — a registry-literal boundary.
                        builtins::call_return_type(target)
                    })
                    .ok_or_else(|| {
                        format!("native raw result call '{target}' has no return type")
                    })?;
                let success_type = success_type_typed.clone();
                let tag_slot = self.allocate_stack_object("raw_result_tag", 8);
                let value_slot = self.allocate_stack_object("raw_result_value", 8);
                let message_slot = self.allocate_stack_object("raw_result_message", 8);
                let source_slot = self.allocate_stack_object("raw_result_source", 8);
                let unboxed = self.unboxed_trap_active.take();
                // plan-147-D: the trapped call form is an approved site like any other.
                let previous_call = self
                    .current_call_key
                    .replace(crate::codegen::engine::analysis::handover::call_key(value));
                let call = self.emit_call(target, &symbol, args, Some(&success_type.name()));
                self.current_call_key = previous_call;
                call?;
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
                // bug-841: `EXIT PROGRAM` is not the handler's to catch.
                self.emit_inline_trap_program_exit_passthrough(tag_slot, value_slot)?;
                // bug-778: an unboxed temporary owns the callee's block exactly when
                // an owning `Bind` of the untrapped call would (`lower_value_owned`):
                // a param borrow or a rodata `String` is copied, anything else is
                // taken. Nothing is registered for the statement-scope free — the
                // temporary, then its reader's binding, is the owner.
                if let Some(unboxed) = unboxed {
                    let owner = if self.value_needs_owning_copy(value) {
                        UnboxedPayloadSource::Copy
                    } else {
                        UnboxedPayloadSource::Owned
                    };
                    return self.finish_unboxed_trap_result(
                        unboxed,
                        tag_slot,
                        value_slot,
                        message_slot,
                        source_slot,
                        owner,
                        TrappedErrorSource::CalleeRegister,
                        format!("callResult {target}"),
                    );
                }
                let payload_slot = self.allocate_stack_object("raw_result_payload", 8);
                let wrap_error_label = self.label("result_wrap_error");
                let have_payload_label = self.label("result_have_payload");
                let result_slot = self.allocate_stack_object("raw_result", 8);
                self.emit(abi::load_u64(scratch9, abi::stack_pointer(), tag_slot));
                self.emit(abi::compare_immediate(scratch9, RESULT_OK_TAG));
                self.emit(abi::branch_ne(&wrap_error_label));
                self.emit(abi::load_u64(scratch9, abi::stack_pointer(), value_slot));
                self.emit(abi::store_u64(scratch9, abi::stack_pointer(), payload_slot));
                let ok_result =
                    self.emit_build_result_inline(tag_slot, &success_type, payload_slot)?;
                self.emit(abi::store_u64(
                    &ok_result,
                    abi::stack_pointer(),
                    result_slot,
                ));
                // bug-561: the `Result` above owns a COPY of the callee's block;
                // the callee's own block is dead from here. Ok path only —
                // `value_slot` holds an error code on the other branch.
                self.register_call_result_payload_temp(value, &success_type, payload_slot);
                self.emit(abi::branch(&have_payload_label));
                self.emit(abi::label(&wrap_error_label));
                // bug-565: adopt a parked `ERR_BLOCK` instead of orphaning it, and
                // free the payload block the `Result` copied.
                self.emit_trapped_error_result(
                    scratch9,
                    tag_slot,
                    value_slot,
                    message_slot,
                    source_slot,
                    payload_slot,
                    result_slot,
                    TrappedErrorSource::CalleeRegister,
                )?;
                self.emit(abi::label(&have_payload_label));
                Ok(self.fresh_trapped_result_value(
                    result_slot,
                    success_type_typed.clone(),
                    format!("callResult {target}"),
                ))
            }
            NirValue::RuntimeCall {
                helper,
                target,
                args,
                ..
            } => {
                // plan-116-J: a consuming parameter takes over closing the resources
                // reachable from its argument, so this scope stops closing them.
                //
                // THIS is where a `Body::abi_function` member's call site arrives, and
                // finding that took three wrong guesses worth recording: not
                // `lower_value`'s `Call` arm, not its `CallResult` arm, and not
                // `emit_call`. `canvas::setGroup` lowers to
                // `{"kind": "runtimeCall", "helper": "canvas", "target":
                // "canvas.setGroup"}` — a NIR node of its own, which none of those three
                // see. The `--nir` dump is what settled it; a trace of 798 lowered calls
                // through `emit_call` did not contain the one call the program makes.
                self.deactivate_consumed_cleanups(target, args);
                // `strings::`/`astrings::` native members migrated to the clean-room
                // registry are `Body::abi_inline`, so they arrive on the `Call` path
                // (reached through `try_abi_inline_lower` there), not this `RuntimeCall`
                // arm — which handles the explicit intrinsics below (plan-99 PART B/C).
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
                    let type_name = self.static_type_name_for_fold(&args[0]).ok_or_else(|| {
                        "native code cannot determine typeName argument type".to_string()
                    })?;
                    let register = self.load_string_constant(&type_name.name())?;
                    return Ok(ValueResult {
                        origin: None,
                        type_: ParameterType::String,
                        location: Operand::from(register),
                        text: format!("typeName({type_name})"),
                    });
                }
                self.lower_runtime_helper_call(*helper, target, args, false)
            }
            NirValue::Constructor { type_, args } => {
                // plan-01-vector: a small Float vector is constructed register-native
                // — its lanes stay in per-lane scalar-Float carriers with no
                // arena_alloc, materializing to the block only at a storage boundary
                // (`vector_value_as_block`). Each lane is finiteness-observed exactly
                // as the record-field boundary would (plan-17), so behavior is
                // bit-identical to the heap-record constructor.
                if let Some(count) = vector_field_count(type_) {
                    if args.len() == count {
                        let mut lanes = Vec::with_capacity(count);
                        for arg in args {
                            let value = self.lower_value(arg)?;
                            self.observe_float(arg, &value)?;
                            lanes.push(value);
                        }
                        return Ok(self.make_vector_native(type_, lanes));
                    }
                }
                // A fresh nested owned block passed as a field (e.g. the `ErrorLoc`
                // inside `error(...)`) is registered as a pending temp while lowering
                // the arg, then byte-INLINED (copied) into this record — so the
                // standalone arg block is dead the moment the record is built. On a
                // normal statement it is reclaimed by the statement-scope drop, but a
                // `FAIL` (and other control transfers) CLEARS pending temps instead of
                // freeing them, orphaning it (a per-caught-error leak). Free those
                // consumed arg temps right here so the record is self-contained on
                // every path (plan-25 comment: a record temp is a single arena_free).
                // plan-118-D: the per-TYPE census behind the synthesis threshold.
                // `val:Constructor` in the expansion tally is the aggregate; this
                // splits it by record/variant type so "which types are constructed
                // often enough for a shared function to amortize" is a measurement
                // rather than an assumption. Inclusive of the arguments' own
                // lowering (the aggregate row is the exclusive one), which is the
                // right reading here: the question is what a construction site
                // costs in total. Counted under `-vv` only.
                let census_start = self.instructions.len();
                let arg_temp_watermark = self.pending_temp_frees.len();
                let mut arg_values = Vec::new();
                let mut arg_slots = Vec::new();
                for arg in args {
                    // plan-134-E: a recursive argument is stored into the new record.
                    let value = self.lower_value_stored_field(arg)?;
                    // Observation boundary: a `Float` record/union field must be
                    // finite (plan-17).
                    self.observe_float(arg, &value)?;
                    // A register-native vector field or a `d`-native float is
                    // materialized before the field-payload spill (plan-01).
                    let value = self.materialize_value(value)?;
                    let slot = self.allocate_stack_object("constructor_arg", 8);
                    self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
                    arg_values.push(value);
                    arg_slots.push(slot);
                }
                if self.type_model.record_fields.contains_key(type_) {
                    // A record inlines its `String` fields into a trailing data
                    // region (the slot holds a block-relative offset); scalar and
                    // pointer fields stay inline at `8*index` (plan-02 §4.2).
                    //
                    // plan-118-D: for a type constructed often enough to amortize
                    // it, all of that — the allocation, the failure block, the
                    // field stores, a byte-copy loop per String field — is in
                    // `construct.T` and this site marshals and calls. The helper
                    // is built by replaying THIS emitter, so the layout law is
                    // not forked.
                    let register = if self.synthesized_constructors.contains(type_) {
                        self.emit_construct_helper_call(type_, &arg_slots)?
                    } else {
                        self.emit_build_inlined_record(type_, &arg_slots)?
                    };
                    // The record now owns byte-inlined copies of every field, so the
                    // consumed nested arg blocks are dead — free them (the record
                    // register is live across these frees and preserved by the vreg
                    // allocator). Spill/reload the record pointer so the `arena_free`
                    // calls cannot clobber it.
                    let result_slot = self.allocate_stack_object("constructor_result", 8);
                    self.emit(abi::store_u64(&register, abi::stack_pointer(), result_slot));
                    self.drop_arg_block_temps_to(arg_temp_watermark)?;
                    let result = self.temporary_vreg();
                    self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
                    crate::trace::count_tally(
                        "constructor type",
                        || format!("record {type_}"),
                        (self.instructions.len() - census_start) as u64,
                    );
                    return Ok(ValueResult {
                        origin: None,
                        type_: type_.clone(),
                        location: Operand::from(result),
                        text: format!("construct {type_}({})", join_texts(&arg_values)),
                    });
                }
                // Not a record, and there is nothing else a `Constructor` can
                // name. A union is all-data or all-resource
                // (`TYPE_MIXED_RESOURCE_UNION`): every variant of an all-data
                // union is a `TYPE ... END TYPE` record and is therefore in
                // `record_fields` above, and a resource has no constructor
                // syntax at all — it comes from a function. So a `Constructor`
                // whose type is missing from `record_fields` is a lowering bug,
                // and this fails CLOSED rather than guessing a layout for it.
                //
                // bug-548: ~85 lines of union-variant construction stood here,
                // building a tagged block the way `UnionWrap` does. Nothing
                // reached it. Four sweeps agreed — every unit test, every one of
                // ~700 committed projects, and then, with the instrumentation
                // still in, an imported package's variant (`json::JsonNum[1.5]`),
                // a LOCAL union over imported record variants, and a mixed
                // resource/data union, which the type checker refuses outright.
                // That refusal is what turned "no program reaches it" into "no
                // program can": it removes the one variant kind that is not a
                // record.
                Err(format!(
                    "native code constructor type '{type_}' is not a record; a \
                     union variant is either a record (constructed above) or a \
                     resource (which has no constructor)"
                ))
            }
            NirValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => {
                // plan-134-E: the variant is stored into the new union block.
                let wrapped = self.lower_value_stored_inline(value)?;
                let wrapped_slot = self.allocate_stack_object("union_wrap_source", 8);
                self.emit(abi::store_u64(
                    &wrapped.location,
                    abi::stack_pointer(),
                    wrapped_slot,
                ));
                // A resource-union variant is a bare resource whose payload is
                // the resource pointer itself (one word at offset 8), not record
                // fields.
                // bug-851: a `RESOURCE`-declared variant is a bare handle as well.
                let is_resource_variant = self.is_resource_nominal(member_type);
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
                        origin: None,
                        type_: union_type.clone(),
                        location: Operand::from(register),
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
                        if self.is_resource_nominal(variant) {
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
                self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
                self.emit_arena_alloc_call();
                self.emit(abi::branch_eq(&alloc_ok));
                self.raise_error_bare("ErrOutOfMemory")?;
                self.emit(abi::label(&alloc_ok));
                self.emit(abi::store_u64(
                    abi::mfb_return(1),
                    abi::stack_pointer(),
                    result_slot,
                ));
                let zero_register = self.allocate_register();
                self.emit(abi::move_immediate(&zero_register, "Integer", "0"));
                for offset in (0..union_size).step_by(8) {
                    self.emit(abi::store_u64(&zero_register, abi::mfb_return(1), offset));
                }
                let tag_register = self.allocate_register();
                self.emit(abi::move_immediate(
                    &tag_register,
                    abi::IMMEDIATE_CLASS_UNION_TAG,
                    &tag.to_string(),
                ));
                self.emit(abi::store_u64(&tag_register, abi::mfb_return(1), 0));
                // Only resource variants reach here — every data variant returned
                // early above via `emit_wrap_record_in_union`. A resource variant
                // stores its handle pointer as a single word at +8 (plan-02 §4.2).
                self.emit(abi::load_u64(scratch9, abi::stack_pointer(), wrapped_slot));
                self.emit(abi::load_u64(scratch10, abi::stack_pointer(), result_slot));
                self.emit(abi::store_u64(scratch9, scratch10, 8));
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
                Ok(ValueResult {
                    origin: None,
                    type_: union_type.clone(),
                    location: Operand::from(register),
                    text: format!("wrap {member_type} as {union_type}"),
                })
            }
            NirValue::UnionExtract { type_, value } => {
                // A resource-union variant's payload is the resource pointer
                // itself (offset 8): extracting it yields that pointer directly.
                if self.is_resource_nominal(type_) {
                    let source = self.lower_value(value)?;
                    let register = self.allocate_register();
                    self.emit(abi::load_u64(&register, &source.location, 8));
                    return Ok(ValueResult {
                        origin: None,
                        type_: type_.clone(),
                        location: Operand::from(register),
                        text: format!("extract {type_} from {}", source.text),
                    });
                }
                // A data union inlines the active variant's flat record block at
                // +16 (plan-02 §4.3); the extracted record is an alias into the
                // union at that offset.
                let source = self.lower_value(value)?;
                let register = self.allocate_register();
                self.emit(abi::add_immediate(&register, &source.location, 16));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: format!("extract {type_} from {}", source.text),
                })
            }
            NirValue::ResultIsOk { value } => {
                if let Some(result) = self.lower_unboxed_result_is_ok(value) {
                    return result;
                }
                let result = self.lower_value(value)?;
                let register = self.allocate_register();
                let ok_label = self.label("result_is_ok_true");
                let end_label = self.label("result_is_ok_end");
                self.emit(abi::load_u64(scratch9, &result.location, 0));
                self.emit(abi::compare_immediate(scratch9, RESULT_OK_TAG));
                self.emit(abi::branch_eq(&ok_label));
                self.emit(abi::move_immediate(&register, "Boolean", "0"));
                self.emit(abi::branch(&end_label));
                self.emit(abi::label(&ok_label));
                self.emit(abi::move_immediate(&register, "Boolean", "1"));
                self.emit(abi::label(&end_label));
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::Boolean,
                    location: Operand::from(register),
                    text: "resultIsOk".to_string(),
                })
            }
            NirValue::ResultValue { value } => {
                if let Some(result) = self.lower_unboxed_result_value(value) {
                    return result;
                }
                let result = self.lower_value(value)?;
                let crate::types::ParameterType::ResultOf(payload) = &result.type_ else {
                    return Err(format!(
                        "native RESULT_VALUE requires raw Result input, got `{}`",
                        result.type_
                    ));
                };
                let type_ = (**payload).clone();
                // The payload is inlined at +16 (plan-02 §4.3): a block payload
                // yields an alias pointer into the Result; a scalar payload is the
                // 8-byte value.
                let register = self.allocate_register();
                if type_ == ParameterType::Nothing {
                    // bug-744: a `Nothing` callee leaves `RESULT_VALUE_REGISTER`
                    // unset, so the payload `materialize_current_result` boxed from
                    // it is whatever that register last held. The one `Nothing`
                    // value is `NOTHING` (0); produce it rather than read the slot.
                    self.emit(abi::move_immediate(&register, "Integer", "0"));
                } else if self.result_payload_is_block(&type_) {
                    self.emit(abi::add_immediate(&register, &result.location, 16));
                } else {
                    self.emit(abi::load_u64(&register, &result.location, 16));
                }
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: "resultValue".to_string(),
                })
            }
            NirValue::ResultError { value } => {
                if let Some(result) = self.lower_unboxed_result_error(value) {
                    return result;
                }
                let result = self.lower_value(value)?;
                // The error payload (a flat Error block) is inlined at +16.
                let register = self.allocate_register();
                self.emit(abi::add_immediate(&register, &result.location, 16));
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::named("Error"),
                    location: Operand::from(register),
                    text: "resultError".to_string(),
                })
            }
            NirValue::Checked {
                type_,
                value,
                observe,
            } => self.lower_checked_value(&type_, &value, *observe),
            NirValue::WithUpdate {
                type_,
                target,
                updates,
            } => {
                // bug-766 B: the item a `set`/`append` self-update copies, built in
                // the record-temp scratch.
                let node = value as *const NirValue as usize;
                if self.record_temps.with_item_armed == Some(node) {
                    self.record_temps.with_item_armed = None;
                    let result = self.lower_with_update_in_record_temp(type_, target, updates)?;
                    self.record_temps.placed = Some(node);
                    return Ok(result);
                }
                self.lower_with_update(&type_, target, updates)
            }
            NirValue::MemberAccess { target, member } => match target.as_ref() {
                _ if member == "result" => {
                    if let Some(output_type) =
                        self.static_type_name(target).and_then(|type_| match type_ {
                            ParameterType::ThreadHandle {
                                worker: false, out, ..
                            } => Some((*out).clone()),
                            _ => None,
                        })
                    {
                        self.emit_raw_call(
                            &runtime::symbol_for_call(
                                runtime::RuntimeHelper::Thread,
                                "thread.waitFor",
                            ),
                            std::slice::from_ref(target.as_ref()),
                            "thread_result_arg",
                        )?;
                        // bug-566: the worker allocated this block in ITS arena
                        // (`x19` is per-thread). Freeing it here is not a leak fix,
                        // it is a cross-arena wild free — the counter-example that
                        // makes the runtime-helper question an audit.
                        return self.materialize_current_result(
                            &output_type,
                            "thread.result".to_string(),
                            true,
                            RawSuccessBlock::OwnedElsewhere,
                        );
                    }
                    self.lower_field_access(target, member)
                }
                NirValue::Local(type_name) => {
                    if let Some(ordinal) = self
                        .type_model
                        .enum_members
                        .get(&(ParameterType::declared(type_name), member.clone()))
                        .copied()
                    {
                        let register = self.allocate_register();
                        self.emit(abi::move_immediate(
                            &register,
                            abi::IMMEDIATE_CLASS_ENUM_ORDINAL,
                            &ordinal.to_string(),
                        ));
                        return Ok(ValueResult {
                            origin: None,
                            type_: ParameterType::declared(type_name),
                            location: Operand::from(register),
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
                if *op == BinaryOp::Concat {
                    // String concat / rope fusion (Level 3): `&` is
                    // left-associative, so `a & b & c` arrives as
                    // `(a & b) & c` and the pairwise lowering would allocate
                    // and fill a whole intermediate for `a & b` only to copy
                    // it again. Flatten the left spine and lower the chain
                    // into one pre-sized allocation instead. Two operands are
                    // left to the pairwise path verbatim, so ordinary `a & b`
                    // is untouched at every level.
                    let mut parts: Vec<&NirValue> = Vec::new();
                    flatten_concat_spine(left, &mut parts);
                    parts.push(right);
                    if parts.len() > 2 && crate::optimizer::level_enabled(3) {
                        let fused = parts.len() as u64 - 1;
                        let result = self.lower_string_concat_chain(&parts)?;
                        crate::optimizer::stats::count_string_concats_fused(fused);
                        return Ok(result);
                    }
                    return self.lower_string_concat(left, right);
                }
                if matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor) {
                    return self.lower_boolean_binary(*op, left, right);
                }
                if op.is_comparison() {
                    return self.lower_comparison_binary(*op, left, right);
                }
                self.lower_arithmetic_binary(*op, left, right)
            }
            NirValue::Unary { op, operand, .. } => {
                let operand = self.lower_value(operand)?;
                if *op == UnaryOp::Not && operand.type_ == ParameterType::Boolean {
                    let register = self.allocate_register();
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
                        origin: None,
                        type_: ParameterType::Boolean,
                        location: Operand::from(register),
                        text: format!("(NOT {})", operand.text),
                    });
                }
                if *op == UnaryOp::Negate
                    && matches!(
                        operand.type_.name().as_ref(),
                        "Byte" | "Integer" | "Fixed" | "Float" | "Money"
                    )
                {
                    return self.lower_numeric_unary_negation(operand);
                }
                // `NOT` on a non-Boolean and `-` on a non-numeric are both
                // rejected by `ir::verify` before lowering, and `SIZEOF` folds
                // during LINK lowering; a node reaching here is malformed IR,
                // not an unlowered operator.
                Err(format!(
                    "native code plan does not lower unary operator '{}' for {} yet while lowering native function '{}'",
                    op.name(),
                    operand.type_,
                    self.current_symbol
                ))
            }
            NirValue::ListLiteral { type_, values } => self.lower_list_literal(&type_, values),
            NirValue::SetLiteral { type_, values } => self.lower_set_literal(&type_, values),
            NirValue::MapLiteral { type_, entries } => self.lower_map_literal(&type_, entries),
        }
    }
}
