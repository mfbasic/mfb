use super::*;

pub(super) fn expanded_nir_union_variants<'a>(
    module: &'a NirModule,
    union_name: &str,
) -> Vec<&'a super::nir::NirVariant> {
    let Some(type_) = module
        .types
        .iter()
        .find(|candidate| candidate.kind == "union" && candidate.name == union_name)
    else {
        return Vec::new();
    };
    let mut variants = Vec::new();
    for include in &type_.includes {
        variants.extend(expanded_nir_union_variants(module, include));
    }
    variants.extend(type_.variants.iter());
    variants
}

pub(super) fn lower_function(
    function: &NirFunction,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    package_return_types: &HashMap<String, String>,
    platform_imports: &HashMap<String, String>,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let params = function
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let location = abi::argument_register(index)?;
            Ok(CodeParam {
                name: param.name.clone(),
                type_: param.type_.clone(),
                location,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut builder = CodeBuilder {
        current_symbol: nir::function_symbol(&function.name),
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        globals,
        type_model,
        string_symbols,
        locals: HashMap::new(),
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
        next_vreg: 0,
        vreg_eager: Vec::new(),
        regalloc_kind: regalloc::active_kind(),
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        pending_result_slots: None,
        error_arena_restore_slot: None,
        raw_result_capture: None,
        current_file: function.file.clone(),
        current_loc: NirSourceLoc::default(),
        owner_collections: function
            .resource_owners
            .values()
            .filter_map(|owner| match owner {
                crate::escape::ResOwner::Float(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        resource_owners: function.resource_owners.clone(),
        owned_list_heads: HashMap::new(),
        owned_value_slots: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        string_capacity_slots: HashMap::new(),
    };
    for param in &params {
        let stack_offset = builder.allocate_stack_object(&param.name, 8);
        builder.locals.insert(
            param.name.clone(),
            LocalValue {
                type_: param.type_.clone(),
                stack_offset,
                constant: None,
                by_ref: false,
            },
        );
        builder.emit(abi::store_u64(
            &param.location,
            abi::stack_pointer(),
            stack_offset,
        ));
        if CodeBuilder::is_thread_type(&param.type_) {
            builder
                .active_cleanups
                .push(ActiveCleanup::Thread(ThreadCleanup {
                    name: param.name.clone(),
                    symbol: CodeBuilder::thread_drop_symbol(),
                }));
        }
    }
    if let Some(name) = function.body.iter().find_map(|op| match op {
        NirOp::Trap { name, .. } => Some(name.clone()),
        _ => None,
    }) {
        let stack_offset = builder.allocate_stack_object(&name, 8);
        builder.locals.insert(
            name.clone(),
            LocalValue {
                type_: "Error".to_string(),
                stack_offset,
                constant: None,
                by_ref: false,
            },
        );
        let label = builder.label("trap");
        builder.trap = Some(TrapState {
            name,
            label,
            in_trap_body: false,
        });
    }
    // Pre-allocate the capacity shadow slot for every in-place string self-append
    // target so bind/assign sites can reset it and the prologue can zero it.
    builder.prescan_string_self_appends(&function.body);
    builder.lower_ops(&function.body)?;
    if !builder.current_block_returns() {
        builder.emit_return_exit(None)?;
    }
    // Color virtual registers to physical registers (plan-03 Stage A) before the
    // body is moved out for the peephole pass and finalize_frame.
    builder.run_register_allocation();
    let mut instructions = builder.instructions;
    // Zero every string self-append capacity shadow at function entry: the buffer a
    // parameter or first assignment hands the local is tight (no spare). Stores are
    // sp-relative with pre-prologue offsets; `finalize_frame` shifts them like every
    // other stack access. The shadow is reset on every later non-self-append
    // bind/assign, so it always reflects the live buffer's spare bytes.
    if !builder.string_capacity_slots.is_empty() {
        let mut zeroing = vec![abi::move_immediate("x9", "Integer", "0")];
        let mut slots: Vec<usize> = builder.string_capacity_slots.values().copied().collect();
        slots.sort_unstable();
        for slot in slots {
            zeroing.push(abi::store_u64("x9", abi::stack_pointer(), slot));
        }
        let insert_at = if instructions
            .first()
            .is_some_and(|instruction| instruction.op == CodeOp::Label)
        {
            1
        } else {
            0
        };
        instructions.splice(insert_at..insert_at, zeroing);
    }
    // In a trap function, an error can jump to the handler past a not-yet-run
    // `LET`; zero every owned freeable-flat slot at entry so the handler's
    // scope-drop skips any binding whose initializer never executed. The stores
    // are sp-relative with pre-prologue offsets, so `finalize_frame` shifts them
    // by the callee-save area like every other stack access.
    if builder.trap.is_some() && !builder.owned_value_slots.is_empty() {
        let mut zeroing = Vec::new();
        zeroing.push(abi::move_immediate("x9", "Integer", "0"));
        let mut slots = builder.owned_value_slots.clone();
        slots.sort_unstable();
        slots.dedup();
        for slot in slots {
            zeroing.push(abi::store_u64("x9", abi::stack_pointer(), slot));
        }
        let insert_at = if instructions
            .first()
            .is_some_and(|instruction| instruction.op == CodeOp::Label)
        {
            1
        } else {
            0
        };
        instructions.splice(insert_at..insert_at, zeroing);
    }
    // Store-to-load forwarding over the lowered stream (offsets are still
    // pre-prologue here, before finalize_frame shifts them).
    peephole::forward_stores_to_loads(&mut instructions);
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );

    Ok(CodeFunction {
        name: function.name.clone(),
        symbol: nir::function_symbol(&function.name),
        params,
        returns: function.returns.clone(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

pub(super) fn lower_builtin_function_wrapper(
    name: &str,
    type_: &str,
    symbol: &str,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    package_return_types: &HashMap<String, String>,
    platform_imports: &HashMap<String, String>,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let (params, returns) = function_type_parts(type_).ok_or_else(|| {
        format!("native built-in function wrapper has malformed function type '{type_}'")
    })?;
    if params.len() != 1 || returns != "Boolean" {
        return Err(format!(
            "native built-in function wrapper expects a unary Boolean function, got '{type_}'"
        ));
    }

    let param = CodeParam {
        name: "value".to_string(),
        type_: params[0].clone(),
        location: abi::argument_register(0)?,
    };
    let mut builder = CodeBuilder {
        current_symbol: symbol.to_string(),
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        globals,
        type_model,
        string_symbols,
        locals: HashMap::new(),
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
        next_vreg: 0,
        vreg_eager: Vec::new(),
        regalloc_kind: regalloc::active_kind(),
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        pending_result_slots: None,
        error_arena_restore_slot: None,
        raw_result_capture: None,
        current_file: String::new(),
        current_loc: NirSourceLoc::default(),
        resource_owners: HashMap::new(),
        owner_collections: HashSet::new(),
        owned_list_heads: HashMap::new(),
        owned_value_slots: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        string_capacity_slots: HashMap::new(),
    };

    let stack_offset = builder.allocate_stack_object("value", 8);
    builder.locals.insert(
        "value".to_string(),
        LocalValue {
            type_: param.type_.clone(),
            stack_offset,
            constant: None,
            by_ref: false,
        },
    );
    builder.emit(abi::store_u64(
        &param.location,
        abi::stack_pointer(),
        stack_offset,
    ));

    let result = builder.lower_value(&NirValue::Call {
        target: name.to_string(),
        args: vec![NirValue::Local("value".to_string())],
        loc: NirSourceLoc::default(),
    })?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    builder.run_register_allocation();
    let mut instructions = builder.instructions;
    peephole::forward_stores_to_loads(&mut instructions);
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );

    Ok(CodeFunction {
        name: format!("builtin.{name}.{type_}"),
        symbol: symbol.to_string(),
        params: vec![param],
        returns: returns,
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}
