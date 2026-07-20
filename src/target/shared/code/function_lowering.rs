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

/// Small-vector locals safe to keep in registers (their lanes) for their whole
/// lifetime, with no arena block (plan-01-vector). A candidate is a binding of a
/// vector type (`Float2/3/4`, `Fixed*`, `Integer*`) whose initializer produces a
/// register-native value — a vector construction or an inlined vector op — and
/// whose every use is *non-materializing* (a member read, or a direct argument to
/// an inlined vector op). Such a binding never needs a heap record. Excludes
/// address-taken and reassigned locals. Correctness does not hinge on precision
/// (`vector_value_as_block` materializes on demand); the analysis exists to avoid
/// promoting an *escaping* local, which would copy its block per use.
pub(super) fn promotable_vector_locals(
    ops: &[NirOp],
    address_taken: &HashSet<String>,
) -> HashSet<String> {
    let mut candidates = HashSet::new();
    collect_vector_native_bindings(ops, &mut candidates);
    let mut reassigned = HashSet::new();
    collect_assigned_locals(ops, &mut reassigned);
    let mut escaping = HashSet::new();
    mark_vector_escaping_ops(ops, &mut escaping);
    candidates
        .into_iter()
        .filter(|name| {
            !address_taken.contains(name) && !reassigned.contains(name) && !escaping.contains(name)
        })
        .collect()
}

/// Whether `value` lowers to a register-native small vector (a vector constructor
/// or an inlined vector op), so a binding of it starts life in lanes.
fn is_vector_native_producing(value: &NirValue) -> bool {
    match value {
        NirValue::Constructor { type_, .. } => vector_field_count(type_).is_some(),
        NirValue::Call { target, args, .. } => vector_call_is_inlined(target, args),
        _ => false,
    }
}

fn collect_vector_native_bindings(ops: &[NirOp], out: &mut HashSet<String>) {
    for op in ops {
        match op {
            NirOp::Bind {
                name,
                type_,
                value: Some(value),
                ..
            } if vector_field_count(type_).is_some() && is_vector_native_producing(value) => {
                out.insert(name.clone());
            }
            NirOp::Bind { .. }
            | NirOp::StoreGlobal { .. }
            | NirOp::Assign { .. }
            | NirOp::StateAssign { .. }
            | NirOp::Return { .. }
            | NirOp::Eval { .. }
            | NirOp::Fail { .. }
            | NirOp::ExitProgram { .. }
            | NirOp::ExitLoop { .. }
            | NirOp::ContinueLoop { .. } => {}
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                collect_vector_native_bindings(then_body, out);
                collect_vector_native_bindings(else_body, out);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    collect_vector_native_bindings(&case.body, out);
                }
            }
            NirOp::While { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::For { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => collect_vector_native_bindings(body, out),
        }
    }
}

fn collect_assigned_locals(ops: &[NirOp], out: &mut HashSet<String>) {
    for op in ops {
        match op {
            NirOp::Assign { name, .. } => {
                out.insert(name.clone());
            }
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                collect_assigned_locals(then_body, out);
                collect_assigned_locals(else_body, out);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    collect_assigned_locals(&case.body, out);
                }
            }
            NirOp::While { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::For { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => collect_assigned_locals(body, out),
            _ => {}
        }
    }
}

/// Mark every local read in a *materializing* position — anything other than a
/// member read of the local or a direct argument to an inlined vector op.
fn mark_vector_escaping_value(value: &NirValue, out: &mut HashSet<String>) {
    match value {
        NirValue::Local(name) => {
            out.insert(name.clone());
        }
        // `a.x` reads a lane and does not materialize `a`; a deeper target recurses.
        NirValue::MemberAccess { target, .. } => {
            if !matches!(target.as_ref(), NirValue::Local(_)) {
                mark_vector_escaping_value(target, out);
            }
        }
        NirValue::Call { target, args, .. } => {
            let inlined = vector_call_is_inlined(target, args);
            for arg in args {
                if inlined && matches!(arg, NirValue::Local(_)) {
                    continue; // a lane-read argument to an inlined op
                }
                mark_vector_escaping_value(arg, out);
            }
        }
        NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. }
        | NirValue::ListLiteral { values: args, .. } => {
            for arg in args {
                mark_vector_escaping_value(arg, out);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::Unary { operand: value, .. } => mark_vector_escaping_value(value, out),
        NirValue::Binary { left, right, .. } => {
            mark_vector_escaping_value(left, out);
            mark_vector_escaping_value(right, out);
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            mark_vector_escaping_value(target, out);
            for update in updates {
                mark_vector_escaping_value(&update.value, out);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, val) in entries {
                mark_vector_escaping_value(key, out);
                mark_vector_escaping_value(val, out);
            }
        }
        NirValue::Closure { captures, .. } => {
            for capture in captures {
                mark_vector_escaping_value(capture, out);
            }
        }
        NirValue::Const { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. }
        | NirValue::LocalRef { .. } => {}
    }
}

fn mark_vector_escaping_ops(ops: &[NirOp], out: &mut HashSet<String>) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. }
            | NirOp::StoreGlobal { value, .. }
            | NirOp::Return { value } => {
                if let Some(value) = value {
                    mark_vector_escaping_value(value, out);
                }
            }
            NirOp::Assign { value, .. }
            | NirOp::StateAssign { value, .. }
            | NirOp::Eval { value }
            | NirOp::ExitProgram { code: value }
            | NirOp::Fail { error: value } => mark_vector_escaping_value(value, out),
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                mark_vector_escaping_value(condition, out);
                mark_vector_escaping_ops(then_body, out);
                mark_vector_escaping_ops(else_body, out);
            }
            NirOp::Match { value, cases } => {
                mark_vector_escaping_value(value, out);
                for case in cases {
                    if let NirMatchPattern::Value(v) = &case.pattern {
                        mark_vector_escaping_value(v, out);
                    }
                    if let NirMatchPattern::OneOf(values) = &case.pattern {
                        for v in values {
                            mark_vector_escaping_value(v, out);
                        }
                    }
                    if let Some(guard) = &case.guard {
                        mark_vector_escaping_value(guard, out);
                    }
                    mark_vector_escaping_ops(&case.body, out);
                }
            }
            NirOp::While {
                condition, body, ..
            }
            | NirOp::DoUntil { body, condition } => {
                mark_vector_escaping_value(condition, out);
                mark_vector_escaping_ops(body, out);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                mark_vector_escaping_value(start, out);
                mark_vector_escaping_value(end, out);
                mark_vector_escaping_value(step, out);
                mark_vector_escaping_ops(body, out);
            }
            NirOp::ForEach { iterable, body, .. } => {
                mark_vector_escaping_value(iterable, out);
                mark_vector_escaping_ops(body, out);
            }
            NirOp::Trap { body, .. } => mark_vector_escaping_ops(body, out),
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
                // A nested loop's condition reads its own locals regardless of
                // depth (bug-70: the former if/else ran the same call in both
                // branches).
                collect_value_local_reads(condition, excluded);
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
            NirOp::Trap { body, .. } => scan_loop_locals(body, depth, top_assigns, excluded),
        }
    }
}

#[allow(clippy::too_many_arguments)]
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
            // Arguments 0..8 arrive in `x0`–`x7`; the rest arrive in the caller's
            // stack tail (bug-08). A stack parameter has no argument register, so
            // its `location` records the tail slot instead (never emitted as a
            // register — the prologue below loads it via `incoming_stack_arg_load`).
            let location = if index < abi::REGISTER_ARGUMENT_COUNT {
                abi::argument_register(index)?
            } else {
                format!("stack{}", index - abi::REGISTER_ARGUMENT_COUNT)
            };
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
        regalloc_error: None,
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        pending_result_slots: None,
        error_arena_restore_slot: None,
        raw_result_capture: None,
        emitting_error_route: false,
        building_error_block: false,
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
        pending_temp_frees: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        string_capacity_slots: HashMap::new(),
        math_pool_base_vreg: None,
        vector_natives: HashMap::new(),
        next_vector_native: 0,
        promoted_vector_locals: HashMap::new(),
        promotable_vector_locals: HashSet::new(),
        integer_lower_bounds: HashMap::new(),
        integer_strict_upper: std::collections::HashSet::new(),
    };
    for (index, param) in params.iter().enumerate() {
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
        if index < abi::REGISTER_ARGUMENT_COUNT {
            builder.emit(abi::store_u64(
                &param.location,
                abi::stack_pointer(),
                stack_offset,
            ));
        } else {
            // A stack parameter is loaded from the incoming stack tail (resolved
            // to an `sp`-relative offset in `finalize_frame`) and spilled into its
            // local slot like a register parameter (bug-08).
            let scratch = builder.temporary_vreg();
            builder.emit(abi::incoming_stack_arg_load(
                &scratch,
                index - abi::REGISTER_ARGUMENT_COUNT,
            ));
            builder.emit(abi::store_u64(&scratch, abi::stack_pointer(), stack_offset));
            builder.reset_temporary_registers();
        }
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
            stack_offset,
        });
    }
    // Pre-allocate the capacity shadow slot for every in-place string self-append
    // target so bind/assign sites can reset it and the prologue can zero it.
    builder.prescan_string_self_appends(&function.body);
    // Locals whose address is taken anywhere — never loop-promoted (plan-03 D2).
    collect_address_taken_locals(&function.body, &mut builder.address_taken_locals);
    // Small-vector locals that can live in registers for their whole lifetime with
    // no arena block (plan-01-vector).
    builder.promotable_vector_locals =
        promotable_vector_locals(&function.body, &builder.address_taken_locals);
    builder.lower_ops(&function.body)?;
    if !builder.current_block_returns() {
        builder.emit_return_exit(None)?;
    }
    // Fuse single-use `a*b ± c` float chains into one single-rounded fused op
    // (plan-02 Phase 3) before allocation, so the fused op's operands are colored
    // as a unit. A no-op unless the `d`-native FP virtual registers are present.
    fma_fusion::fuse_scalar_fma(&mut builder.instructions);
    // Color virtual registers to physical registers (plan-03 Stage A) before the
    // body is moved out for the peephole pass and finalize_frame.
    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
    // Zero every string self-append capacity shadow at function entry: the buffer a
    // parameter or first assignment hands the local is tight (no spare). Stores are
    // sp-relative with pre-prologue offsets; `finalize_frame` shifts them like every
    // other stack access. The shadow is reset on every later non-self-append
    // bind/assign, so it always reflects the live buffer's spare bytes.
    if !builder.string_capacity_slots.is_empty() {
        // Store the zero token (`xzr`) directly — no scratch register, no `mov`
        // (plan-34-C: shared lowering names no physical scratch).
        let mut zeroing = Vec::new();
        let mut slots: Vec<usize> = builder.string_capacity_slots.values().copied().collect();
        slots.sort_unstable();
        for slot in slots {
            zeroing.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
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
    // Zero every owned freeable-flat slot at entry so a scope-drop skips any
    // binding or temporary whose initializer never executed (its null guard sees
    // 0 instead of stack garbage). A trap handler can jump past a not-yet-run
    // `LET`, but the same hazard exists without a trap — a scope-drop over a
    // temporary that a given path leaves unwritten frees whatever the stack held
    // (benign on AArch64 where the slot happened to be zero, a wild free on x86).
    // The stores are sp-relative with pre-prologue offsets, so `finalize_frame`
    // shifts them by the callee-save area like every other stack access.
    if !builder.owned_value_slots.is_empty() {
        // Store the zero token (`xzr`) directly — no scratch register, no `mov`.
        let mut zeroing = Vec::new();
        let mut slots = builder.owned_value_slots.clone();
        slots.sort_unstable();
        slots.dedup();
        for slot in slots {
            zeroing.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
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
    // bug-284 C8: x86-64's mul/div/msub expansions clobber rdx:rax beyond their
    // named dst, so the forwarder must flush across them. Read the ISA the same
    // way `remove_fp_shuttles` does -- from the active backend's arena base --
    // rather than sniffing operand spellings.
    let is_x86 = mir::active_backend().register_model().arena_base()
        == crate::arch::x86_64::regmodel::ARENA_BASE_REGISTER;
    peephole::forward_stores_to_loads(&mut instructions, is_x86);
    // Drop the GP shuttle a checked float value round-trips through (plan-16). The
    // FP-shuttle liveness derives its call-clobber mask from the active backend's
    // register model, not from operand spellings (bug-350).
    peephole::remove_fp_shuttles(&mut instructions, mir::active_backend().register_model());
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

#[allow(clippy::too_many_arguments)]
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
        regalloc_error: None,
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        pending_result_slots: None,
        error_arena_restore_slot: None,
        raw_result_capture: None,
        emitting_error_route: false,
        building_error_block: false,
        current_file: String::new(),
        current_loc: NirSourceLoc::default(),
        resource_owners: HashMap::new(),
        owner_collections: HashSet::new(),
        owned_list_heads: HashMap::new(),
        owned_value_slots: Vec::new(),
        pending_temp_frees: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        string_capacity_slots: HashMap::new(),
        math_pool_base_vreg: None,
        vector_natives: HashMap::new(),
        next_vector_native: 0,
        promoted_vector_locals: HashMap::new(),
        promotable_vector_locals: HashSet::new(),
        integer_lower_bounds: HashMap::new(),
        integer_strict_upper: std::collections::HashSet::new(),
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

    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
    // bug-284 C8: x86-64's mul/div/msub expansions clobber rdx:rax beyond their
    // named dst, so the forwarder must flush across them. Read the ISA the same
    // way `remove_fp_shuttles` does -- from the active backend's arena base --
    // rather than sniffing operand spellings.
    let is_x86 = mir::active_backend().register_model().arena_base()
        == crate::arch::x86_64::regmodel::ARENA_BASE_REGISTER;
    peephole::forward_stores_to_loads(&mut instructions, is_x86);
    peephole::remove_fp_shuttles(&mut instructions, mir::active_backend().register_model());
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
        returns,
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}
