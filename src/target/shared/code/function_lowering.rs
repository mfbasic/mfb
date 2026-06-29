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

/// Collect the names of every local whose address is taken (`LocalRef`) anywhere
/// in `ops`. A loop-promoted local must have *no* such borrow, since a callback
/// holding the borrow could read or mutate the slot while the value lives only in
/// a register (plan-03 Stage D part 2). The matches are exhaustive on purpose:
/// missing a `LocalRef` would be unsound, so the compiler must force every
/// variant to be handled.
pub(super) fn collect_address_taken_locals(ops: &[NirOp], out: &mut HashSet<String>) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } => {
                if let Some(v) = value {
                    collect_value_local_refs(v, out);
                }
            }
            NirOp::Assign { value, .. }
            | NirOp::StateAssign { value, .. }
            | NirOp::Eval { value }
            | NirOp::ExitProgram { code: value }
            | NirOp::Fail { error: value } => collect_value_local_refs(value, out),
            NirOp::Return { value } => {
                if let Some(v) = value {
                    collect_value_local_refs(v, out);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_value_local_refs(condition, out);
                collect_address_taken_locals(then_body, out);
                collect_address_taken_locals(else_body, out);
            }
            NirOp::Match { value, cases } => {
                collect_value_local_refs(value, out);
                for case in cases {
                    if let NirMatchPattern::Value(v) = &case.pattern {
                        collect_value_local_refs(v, out);
                    }
                    if let NirMatchPattern::OneOf(values) = &case.pattern {
                        for v in values {
                            collect_value_local_refs(v, out);
                        }
                    }
                    if let Some(guard) = &case.guard {
                        collect_value_local_refs(guard, out);
                    }
                    collect_address_taken_locals(&case.body, out);
                }
            }
            NirOp::While {
                condition, body, ..
            }
            | NirOp::DoUntil { body, condition } => {
                collect_value_local_refs(condition, out);
                collect_address_taken_locals(body, out);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_value_local_refs(start, out);
                collect_value_local_refs(end, out);
                collect_value_local_refs(step, out);
                collect_address_taken_locals(body, out);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_value_local_refs(iterable, out);
                collect_address_taken_locals(body, out);
            }
            NirOp::Trap { body, .. } => collect_address_taken_locals(body, out),
        }
    }
}

fn collect_value_local_refs(value: &NirValue, out: &mut HashSet<String>) {
    match value {
        NirValue::LocalRef { name, .. } => {
            out.insert(name.clone());
        }
        NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => {}
        NirValue::Closure { captures, .. } => {
            for v in captures {
                collect_value_local_refs(v, out);
            }
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. }
        | NirValue::ListLiteral { values: args, .. } => {
            for v in args {
                collect_value_local_refs(v, out);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::MemberAccess { target: value, .. }
        | NirValue::Unary { operand: value, .. } => collect_value_local_refs(value, out),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_value_local_refs(target, out);
            for update in updates {
                collect_value_local_refs(&update.value, out);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_value_local_refs(k, out);
                collect_value_local_refs(v, out);
            }
        }
        NirValue::Binary { left, right, .. } => {
            collect_value_local_refs(left, out);
            collect_value_local_refs(right, out);
        }
    }
}

/// Collect every local *read* (`Local`) in `value`. Exhaustive on purpose.
fn collect_value_local_reads(value: &NirValue, out: &mut HashSet<String>) {
    match value {
        NirValue::Local(name) => {
            out.insert(name.clone());
        }
        NirValue::Const { .. }
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => {}
        NirValue::Closure { captures, .. } => {
            for v in captures {
                collect_value_local_reads(v, out);
            }
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. }
        | NirValue::ListLiteral { values: args, .. } => {
            for v in args {
                collect_value_local_reads(v, out);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::MemberAccess { target: value, .. }
        | NirValue::Unary { operand: value, .. } => collect_value_local_reads(value, out),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_value_local_reads(target, out);
            for update in updates {
                collect_value_local_reads(&update.value, out);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_value_local_reads(k, out);
                collect_value_local_reads(v, out);
            }
        }
        NirValue::Binary { left, right, .. } => {
            collect_value_local_reads(left, out);
            collect_value_local_reads(right, out);
        }
    }
}

/// Walk a loop body collecting, at `depth` 0 (this loop's own level), the locals
/// directly assigned (`top_assigns` — the loop-carried-accumulator candidates),
/// and into `excluded` every local that is bound inside the body, is a loop
/// induction variable, or is read/assigned inside a *nested* loop (depth ≥ 1).
/// A candidate that is excluded is never promoted, so a nested loop always sees
/// the authoritative stack slot (plan-03 Stage D part 2).
pub(super) fn scan_loop_locals(
    ops: &[NirOp],
    depth: u32,
    top_assigns: &mut HashSet<String>,
    excluded: &mut HashSet<String>,
) {
    let reads = |v: &NirValue, excluded: &mut HashSet<String>| {
        if depth >= 1 {
            collect_value_local_reads(v, excluded);
        }
    };
    for op in ops {
        match op {
            NirOp::Bind { name, value, .. } => {
                excluded.insert(name.clone());
                if let Some(v) = value {
                    reads(v, excluded);
                }
            }
            NirOp::Assign { name, value } => {
                if depth == 0 {
                    top_assigns.insert(name.clone());
                } else {
                    excluded.insert(name.clone());
                }
                reads(value, excluded);
            }
            NirOp::StoreGlobal { value, .. } => {
                if let Some(v) = value {
                    reads(v, excluded);
                }
            }
            NirOp::StateAssign { value, .. }
            | NirOp::Eval { value }
            | NirOp::ExitProgram { code: value }
            | NirOp::Fail { error: value } => reads(value, excluded),
            NirOp::Return { value } => {
                if let Some(v) = value {
                    reads(v, excluded);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                reads(condition, excluded);
                scan_loop_locals(then_body, depth, top_assigns, excluded);
                scan_loop_locals(else_body, depth, top_assigns, excluded);
            }
            NirOp::Match { value, cases } => {
                reads(value, excluded);
                for case in cases {
                    if let Some(guard) = &case.guard {
                        reads(guard, excluded);
                    }
                    scan_loop_locals(&case.body, depth, top_assigns, excluded);
                }
            }
            NirOp::While {
                condition, body, ..
            }
            | NirOp::DoUntil { body, condition } => {
                if depth >= 1 {
                    collect_value_local_reads(condition, excluded);
                } else {
                    // The nested loop's condition is at depth+1.
                    collect_value_local_reads(condition, excluded);
                }
                scan_loop_locals(body, depth + 1, top_assigns, excluded);
            }
            NirOp::For {
                name,
                start,
                end,
                step,
                body,
                ..
            } => {
                excluded.insert(name.clone());
                reads(start, excluded);
                reads(end, excluded);
                reads(step, excluded);
                scan_loop_locals(body, depth + 1, top_assigns, excluded);
            }
            NirOp::ForEach {
                name,
                iterable,
                body,
                ..
            } => {
                excluded.insert(name.clone());
                reads(iterable, excluded);
                scan_loop_locals(body, depth + 1, top_assigns, excluded);
            }
            NirOp::Trap { body, .. } => {
                scan_loop_locals(body, depth, top_assigns, excluded)
            }
        }
    }
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
        next_fp_register: 0,
        next_fp_vreg: 0,
        fp_vreg_eager: Vec::new(),
        float_residents: HashMap::new(),
        promoted_float_locals: HashMap::new(),
        address_taken_locals: HashSet::new(),
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
    // Locals whose address is taken anywhere — never loop-promoted (plan-03 D2).
    collect_address_taken_locals(&function.body, &mut builder.address_taken_locals);
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
        next_fp_register: 0,
        next_fp_vreg: 0,
        fp_vreg_eager: Vec::new(),
        float_residents: HashMap::new(),
        promoted_float_locals: HashMap::new(),
        address_taken_locals: HashSet::new(),
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
