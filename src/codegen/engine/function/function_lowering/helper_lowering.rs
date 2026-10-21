//! plan-176-U: the helper-function lowerings — builtin wrappers, ABI helpers, thread copy
//! functions and the helper builder. The function lowering itself (`lower_function`) lives in
//! `engine/function/function_lowering/mod.rs`.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_builtin_function_wrapper(
    name: &str,
    type_: &ParameterType,
    symbol: &str,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    package_return_types: &HashMap<String, ParameterType>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn crate::codegen::engine::types::CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let (params, returns) = match type_ {
        ParameterType::Func(params, returns, false) => (params, returns),
        _ => {
            return Err(format!(
                "native built-in function wrapper has malformed function type '{type_}'"
            ))
        }
    };
    if params.len() != 1 || !matches!(returns.as_ref(), ParameterType::Boolean) {
        return Err(format!(
            "native built-in function wrapper expects a unary Boolean function, got '{type_}'"
        ));
    }

    let param = CodeParam {
        name: "value".to_string(),
        type_: params[0].name().into_owned(),
        location: abi::argument_register(0)?.into(),
    };
    let mut builder = CodeBuilder::for_synthetic_function(
        symbol,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    );

    let stack_offset = builder.allocate_stack_object("value", 8);
    builder.locals.insert(
        "value".to_string(),
        LocalValue {
            type_: params[0].clone(),
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
        returns: returns.name().into_owned(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

/// The experimental `AbiFunction` wrapper — the unified successor to `OsLower`.
/// Builds a `CodeBuilder`, binds the incoming ABI argument registers as
/// **pre-lowered** `ValueResult`s (captured into vregs at entry, since the physical
/// arg registers are only live-in at the prologue), runs the registered
/// [`crate::codegen::registry::AbiFunction`] body, then wraps its returned
/// `ValueResult` in the shared runtime-helper fallible convention (value in
/// `RESULT_VALUE_REGISTER`, `RESULT_OK_TAG` in `RESULT_TAG_REGISTER`) and finalizes
/// into the `(frame, instructions, relocations, stack_slots)` tuple every runtime
/// helper produces. Reached from `lower_runtime_helper`'s `abi.*` arm. The demo
/// `abi::funcAddTwo` member routes here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_abi_function_helper(
    call: &str,
    symbol: &str,
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
    type_model: &TypeModel,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    string_symbols: &HashMap<String, String>,
    term_state_offset: Option<usize>,
    presentation_mode_offset: Option<usize>,
    mouse_state_offset: Option<usize>,
    input_state_offset: Option<usize>,
    arena_global_slots: usize,
    uses_rng: bool,
    debug_arena_registry: bool,
    arena_cap: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    let (lower, arity) = crate::codegen::registry::abi_function_lower(call)
        .ok_or_else(|| format!("native code plan has no AbiFunction lowering for '{call}'"))?;

    // A runtime helper is standalone: no user functions/globals/strings are in
    // scope. Empty tables suffice for a builder-driven body that only computes over
    // its argument registers (the demo). Kept as locals the builder borrows for the
    // duration of this call.
    let function_symbols: HashMap<String, String> = HashMap::new();
    let functions: HashMap<String, &NirFunction> = HashMap::new();
    let package_return_types: HashMap<String, ParameterType> = HashMap::new();
    let globals: HashMap<String, GlobalValue> = HashMap::new();

    let mut builder = CodeBuilder {
        current_symbol: symbol.to_string(),
        function_symbols: &function_symbols,
        functions: &functions,
        package_return_types: &package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals: &globals,
        type_model: type_model.clone(),
        string_symbols,
        locals: HashMap::new(),
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
        float_residents: HashMap::new(),
        promoted_float_locals: HashMap::new(),
        address_taken_locals: HashSet::new(),
        borrow_get_locals: HashSet::new(),
        element_updates: ElementUpdates::default(),
        record_temps: Default::default(),
        borrow_get_result: false,
        borrow_get_armed: false,
        append_headroom_binds: Default::default(),
        headroom_copy_result: false,
        headroom_copy_armed: false,
        map_settle_exempt_armed: false,
        record_member_read: false,
        value_discarded_armed: false,
        resource_temp_exempt: None,
        trap_call_results: HashSet::new(),
        resource_temps_closed_on_path: false,
        statement_temps: StatementTemps::default(),
        current_returns_param_borrow: false,
        resource_alias_sources: HashMap::new(),
        reclaiming_owned_lists: Vec::new(),
        reclaiming_resources: Vec::new(),
        store_floats: Default::default(),
        trap_ownership: Default::default(),
        trap_owner_flags: HashMap::new(),
        trap_default_records: HashMap::new(),
        live_resource_aliases: HashMap::new(),
        live_union_wraps: HashMap::new(),
        current_returns_fresh_string: false,
        callback_referenced_functions: HashSet::new(),
        // A helper body constructs nothing through the NIR arm.
        synthesized_constructors: HashSet::new(),
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        scope_token_slot: None,
        pending_result_slots: None,
        escaping_value_slot: None,
        error_return_pads: HashMap::new(),
        error_return_pad_code: Vec::new(),
        raw_result_capture: None,
        trap_discard_error_results: HashSet::new(),
        raw_result_discard_error: false,
        unboxed_trap_candidates: HashMap::new(),
        trap_delivery_moves: HashSet::new(),
        unboxed_trap_locals: HashMap::new(),
        unboxed_trap_pending: None,
        unboxed_trap_active: None,
        unboxed_trap_filled: false,
        suppress_resource_source_flag: false,
        emitting_error_route: false,
        building_error_block: false,
        current_file: String::new(),
        current_loc: NirSourceLoc::default(),
        resource_owners: HashMap::new(),
        owner_containers: HashSet::new(),
        owned_list_heads: HashMap::new(),
        owned_value_slots: Vec::new(),
        pending_temp_frees: Vec::new(),
        closure_env_layouts: Vec::new(),
        fresh_string_block: None,
        operand_snapshot_wanted: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        for_each_iterable_state_fields: Vec::new(),
        for_each_iterable_record_fields: Vec::new(),
        for_each_entry_blocks: Vec::new(),
        string_capacity_slots: HashMap::new(),
        self_update_scratch: None,
        self_update_scratch_env: None,
        field_pre_emit: None,
        field_pre_emitted: None,
        simd_result_into: None,
        math_pool_base_vreg: None,
        vector_natives: HashMap::new(),
        next_vector_native: 0,
        promoted_vector_locals: HashMap::new(),
        promotable_vector_locals: HashSet::new(),
        resource_containment: HashMap::new(),
        integer_lower_bounds: HashMap::new(),
        integer_strict_upper: std::collections::HashSet::new(),
        for_bound_expr: HashMap::new(),
        len_of_local: HashMap::new(),
        provable_index_locals: HashMap::new(),
        cold_instructions: Vec::new(),
        cold_blocks_enabled: false,
        enclosing_loop_reassigned: Vec::new(),
        graph_copy_walker: None,
        move_sites: None,
        handover: None,
        current_call_key: None,
        args_updated_in_place: Vec::new(),
        current_op_key: None,
        string_app_resource_base: None,
        string_shadow_env: std::collections::HashMap::new(),
        string_app_resource_base_env: None,
        module_name: String::new(),
    };

    // Hand the body its incoming ABI argument registers directly as `ValueResult`s
    // (a runtime helper is finalized as a vreg body, so an argument register is
    // live-in at entry; the body reads it before any call clobbers it, exactly like
    // an OS-seam helper does).
    let mut args = Vec::with_capacity(arity);
    for index in 0..arity {
        args.push(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: abi::argument_register(index)?.into(),
            text: format!("abiArg{index}"),
        });
    }

    let ctx = crate::codegen::registry::AbiCtx {
        platform_imports,
        platform,
        build_mode,
        module_name,
        term_state_offset,
        presentation_mode_offset,
        mouse_state_offset,
        input_state_offset,
        call,
        arena_global_slots,
        uses_rng,
        debug_arena_registry,
        arena_cap,
    };
    let result = lower(&mut builder, &args, &ctx)?;

    // A body that returns a `void`-location `ValueResult` has emitted its OWN
    // fallible ABI — the success value in `RESULT_VALUE_REGISTER` + `RESULT_OK_TAG`,
    // each error path setting its error + jumping to its own `ret`. The wrapper then
    // adds nothing (a keygen needs distinct `ErrUnknown`/`ErrOutOfMemory` exits it
    // can't express through the auto-epilogue). A body that returns a real value
    // gets the convenient epilogue: move it into the result register, tag OK, return.
    if result.location.render() != "void" {
        builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
        builder.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        builder.emit(abi::return_());
    }

    // Finalize as a runtime-helper vreg body (like OsLower) — this maps the
    // hand-picked `%v9`..`%v15` vregs the general marshallers use, which full
    // register allocation would reject. `stack_size` is the scratch the body
    // reserved via `allocate_stack_object`.
    let mut instructions = builder.instructions;
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], builder.stack_size);
    Ok((frame, instructions, builder.relocations, stack_slots))
}

/// The per-type thread-transfer deep-copy function (bug-391). Takes a pointer to
/// a value of `type_` (a recursive type) in the first argument register and
/// returns a pointer to a fresh, independent copy in the current arena.
///
/// plan-134-B: the copy itself is the module's one non-recursive walker,
/// `_mfb_rt_graph_copy` (`memory/arena/graph_copy.rs`); this entry point passes
/// `type_`'s `kind` index and returns what the walker returns. Its body used to be
/// `emit_thread_copy_real`, whose recursive sub-edges called these functions — a
/// native call per level of the data, which overflowed the stack on a chain about
/// 60 000 deep.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_thread_copy_function(
    type_: &ParameterType,
    kind: usize,
    symbol: &str,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    package_return_types: &HashMap<String, ParameterType>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn crate::codegen::engine::types::CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let param = CodeParam {
        name: "source".to_string(),
        type_: type_.clone().name().into_owned(),
        location: abi::argument_register(0)?.into(),
    };
    let mut builder = helper_code_builder(
        symbol,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    );

    // Hand the source to the walker with this type's kind, and return its copy.
    let source = builder.allocate_register();
    builder.emit(abi::move_register(&source, &param.location));
    builder.emit(abi::move_register(abi::c_arg(1), &source));
    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        &kind.to_string(),
    ));
    builder.emit_symbol_call(crate::codegen::memory::arena::graph_copy::GRAPH_COPY_SYMBOL);
    let result = builder.allocate_register();
    builder.emit(abi::move_register(&result, abi::return_register()));
    builder.emit(abi::move_register(abi::return_register(), &result));
    builder.emit(abi::return_());
    finish_helper_function(
        builder,
        &format!("thread_copy.{type_}"),
        vec![param],
        &type_.to_string(),
    )
}

/// A [`CodeBuilder`] for a runtime helper the codegen synthesizes rather than
/// lowers from NIR (bug-391's per-type copy entry, bug-806's module teardown):
/// no locals, no cleanups, no trap — a body the caller emits instruction by
/// instruction, with every builder service (register allocation, stack objects,
/// calls, raises) available.
#[allow(clippy::too_many_arguments)]
pub(crate) fn helper_code_builder<'a>(
    symbol: &str,
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, ParameterType>,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn crate::codegen::engine::types::CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &'a HashMap<String, GlobalValue>,
    string_symbols: &'a HashMap<String, String>,
    type_model: TypeModel,
) -> CodeBuilder<'a> {
    CodeBuilder {
        current_symbol: symbol.to_string(),
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
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
        float_residents: HashMap::new(),
        promoted_float_locals: HashMap::new(),
        address_taken_locals: HashSet::new(),
        borrow_get_locals: HashSet::new(),
        element_updates: ElementUpdates::default(),
        record_temps: Default::default(),
        borrow_get_result: false,
        borrow_get_armed: false,
        append_headroom_binds: Default::default(),
        headroom_copy_result: false,
        headroom_copy_armed: false,
        map_settle_exempt_armed: false,
        record_member_read: false,
        value_discarded_armed: false,
        resource_temp_exempt: None,
        trap_call_results: HashSet::new(),
        resource_temps_closed_on_path: false,
        statement_temps: StatementTemps::default(),
        current_returns_param_borrow: false,
        resource_alias_sources: HashMap::new(),
        reclaiming_owned_lists: Vec::new(),
        reclaiming_resources: Vec::new(),
        store_floats: Default::default(),
        trap_ownership: Default::default(),
        trap_owner_flags: HashMap::new(),
        trap_default_records: HashMap::new(),
        live_resource_aliases: HashMap::new(),
        live_union_wraps: HashMap::new(),
        current_returns_fresh_string: false,
        callback_referenced_functions: HashSet::new(),
        // A helper body constructs nothing through the NIR arm.
        synthesized_constructors: HashSet::new(),
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        scope_token_slot: None,
        pending_result_slots: None,
        escaping_value_slot: None,
        error_return_pads: HashMap::new(),
        error_return_pad_code: Vec::new(),
        raw_result_capture: None,
        trap_discard_error_results: HashSet::new(),
        raw_result_discard_error: false,
        unboxed_trap_candidates: HashMap::new(),
        trap_delivery_moves: HashSet::new(),
        unboxed_trap_locals: HashMap::new(),
        unboxed_trap_pending: None,
        unboxed_trap_active: None,
        unboxed_trap_filled: false,
        suppress_resource_source_flag: false,
        emitting_error_route: false,
        building_error_block: false,
        current_file: String::new(),
        current_loc: NirSourceLoc::default(),
        resource_owners: HashMap::new(),
        owner_containers: HashSet::new(),
        owned_list_heads: HashMap::new(),
        owned_value_slots: Vec::new(),
        pending_temp_frees: Vec::new(),
        closure_env_layouts: Vec::new(),
        fresh_string_block: None,
        operand_snapshot_wanted: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        for_each_iterable_state_fields: Vec::new(),
        for_each_iterable_record_fields: Vec::new(),
        for_each_entry_blocks: Vec::new(),
        string_capacity_slots: HashMap::new(),
        self_update_scratch: None,
        self_update_scratch_env: None,
        field_pre_emit: None,
        field_pre_emitted: None,
        simd_result_into: None,
        math_pool_base_vreg: None,
        vector_natives: HashMap::new(),
        next_vector_native: 0,
        promoted_vector_locals: HashMap::new(),
        promotable_vector_locals: HashSet::new(),
        resource_containment: HashMap::new(),
        integer_lower_bounds: HashMap::new(),
        integer_strict_upper: std::collections::HashSet::new(),
        for_bound_expr: HashMap::new(),
        len_of_local: HashMap::new(),
        provable_index_locals: HashMap::new(),
        cold_instructions: Vec::new(),
        cold_blocks_enabled: false,
        enclosing_loop_reassigned: Vec::new(),
        graph_copy_walker: None,
        move_sites: None,
        handover: None,
        current_call_key: None,
        args_updated_in_place: Vec::new(),
        current_op_key: None,
        string_app_resource_base: None,
        string_shadow_env: std::collections::HashMap::new(),
        string_app_resource_base_env: None,
        module_name: String::new(),
    }
}

/// Allocate registers, run the helper peepholes and lay out the frame of a
/// [`helper_code_builder`] body whose instructions end in a `return`.
pub(crate) fn finish_helper_function(
    mut builder: CodeBuilder<'_>,
    name: &str,
    params: Vec<CodeParam>,
    returns: &str,
) -> Result<CodeFunction, String> {
    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
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
        name: name.to_string(),
        symbol: builder.current_symbol,
        params,
        returns: returns.to_string(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}
