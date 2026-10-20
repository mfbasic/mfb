// --- codegen tier imports (migration) ---
use crate::arch::ops::CodeOp;
use crate::codegen::builtins::vector::vector_call_is_inlined;
use crate::codegen::builtins::vector::vector_field_count;
use crate::codegen::compiler::opt::fma_fusion;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::mir;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::optimizer::opt2::peephole;
use crate::target::shared::abi;
use crate::target::shared::nir;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
use std::collections::HashMap;
use std::collections::HashSet;

mod helper_lowering;
mod local_analysis;
mod promotion_analysis;
mod trap_analysis;

pub(crate) use helper_lowering::*;
pub(crate) use local_analysis::*;
pub(crate) use promotion_analysis::*;
use trap_analysis::{
    trap_delivery_moves, trap_discard_error_results, unboxed_trap_result_candidates,
};

#[allow(clippy::too_many_arguments)]
/// plan-147-D: which parameters a lowering OWNS, and the symbol it emits under.
///
/// A function lowers once as its base symbol with every parameter lent (the caller
/// keeps the block, `None` here), and then once more per distinct owned-parameter
/// mask that an approved call site asks for. A variant is a second lowering of the
/// **same `NirFunction`**, so every span, `Error.source` stamp and line-table entry
/// is identical to the base's — which is what keeps plan-147-A §2.3 row S11 true.
///
/// Inside a variant an owned parameter is an ordinary owned local: it gets an
/// `ActiveCleanup::OwnedValue`, so it is freed on every exit unless moved out, and
/// letter B's S11 can update it in place and hand the block on.
#[derive(Clone, Debug)]
pub(crate) struct OwnedVariant {
    /// Bit `i` set = parameter `i` is owned by this lowering.
    pub(crate) mask: u64,
    /// bug-784: `Some(j)` for a destination variant (`<base>$into<j>`, mask 0):
    /// parameter `j` is the caller's destination, and `RETURN WITH <param j> { … }`
    /// writes its result there when the size is unchanged
    /// (`analysis::into_dest`).
    pub(crate) into: Option<usize>,
    /// The symbol this lowering emits under.
    pub(crate) symbol: String,
}

impl OwnedVariant {
    /// The symbol a variant of `base` with `mask` emits under.
    ///
    /// `$` is the only separator monomorphization uses (`mangle_name`), and a
    /// mangled segment is `[A-Za-z0-9_$]*` (`sanitize_type_name` maps everything
    /// else to `$`). A lowercase segment IS reachable — a package prefix is
    /// lowercase, so `own1f` is not structurally impossible as a type segment — so
    /// this name is NOT assumed unique. `variant_symbol_is_fresh` asserts it against
    /// the module's real symbol set instead, which fails loudly rather than
    /// silently aliasing two functions.
    pub(crate) fn symbol_for(base: &str, mask: u64) -> String {
        format!("{base}$own{mask:x}")
    }

    /// bug-784: the symbol a destination variant of `base` for parameter `param`
    /// emits under. Checked against the module's symbols like
    /// [`Self::symbol_for`]'s.
    pub(crate) fn into_symbol_for(base: &str, param: usize) -> String {
        format!("{base}$into{param}")
    }

    /// Whether parameter `index` is owned by this lowering.
    pub(crate) fn owns(&self, index: usize) -> bool {
        index < 64 && self.mask & (1u64 << index) != 0
    }
}

pub(crate) fn lower_function(
    function: &NirFunction,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    package_return_types: &HashMap<String, ParameterType>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn crate::codegen::engine::types::CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    callback_referenced_functions: &HashSet<String>,
    // plan-118-D: the record types with their own `construct.T` function.
    synthesized_constructors: &HashSet<ParameterType>,
    type_model: TypeModel,
    module_name: &str,
    // plan-147-D: `None` is the base lowering (every parameter lent).
    variant: Option<&OwnedVariant>,
    // bug-781: receives the environment layout of every capturing `LAMBDA` the
    // function creates, for the module to emit each one's environment routine.
    closure_env_layouts: &mut Vec<ClosureEnvLayout>,
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
                // Typed convention token — a parameter's register location stays
                // Operand::Abi so it is realized by each backend's typed handler
                // (never a Raw convention string). plan-85-D Phase 3.
                abi::argument_register(index)?.into()
            } else {
                // Stack-tail sentinel (bug-08); a Raw marker the prologue interprets.
                Operand::from(format!("stack{}", index - abi::REGISTER_ARGUMENT_COUNT))
            };
            Ok(CodeParam {
                name: param.name.clone(),
                type_: param.type_.clone().name().into_owned(),
                location,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let base_symbol = nir::function_symbol(&function.name);
    let emitted_symbol = match variant {
        Some(variant) => variant.symbol.clone(),
        None => base_symbol.clone(),
    };
    let mut builder = CodeBuilder {
        current_symbol: emitted_symbol.clone(),
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
        synthesized_constructors: synthesized_constructors.clone(),
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
        current_file: function.file.clone(),
        current_loc: NirSourceLoc::default(),
        owner_containers: function
            .resource_owners
            .values()
            .filter_map(|owner| match owner {
                crate::ir::resource_escape::ResOwner::Float(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        resource_owners: function.resource_owners.clone(),
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
        cold_blocks_enabled: true,
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
        module_name: module_name.to_string(),
    };
    // bug-789: a function that can recurse checks the stack at entry, before its
    // parameters are spilled: the check's two temporaries then never share a
    // register with a parameter it would have to reload. The raise is a cold block
    // after the body.
    let stack_overflow =
        if crate::codegen::runtime::stack_limit::function_needs_stack_check(function) {
            let label = builder.label("stack_overflow");
            let start = builder.instructions.len();
            builder.emit_stack_check(&label);
            Some((label, start..builder.instructions.len()))
        } else {
            None
        };
    for (index, param) in params.iter().enumerate() {
        let stack_offset = builder.allocate_stack_object(&param.name, 8);
        builder.locals.insert(
            param.name.clone(),
            LocalValue {
                // The typed NIR param, not the rendered `CodeParam` string —
                // `params[i]` was built 1:1 from `function.params[i]` above.
                type_: function.params[index].type_.clone(),
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
        // plan-147-D: an owned parameter is an ordinary owned local. Registering the
        // cleanup here — after the spill, so the slot genuinely holds the block —
        // frees it on every exit (normal, error, trap route) unless it is moved out,
        // and takes it out of `excluded_roots`'s "never bound" set so letter B's S11
        // and `plan_returned_move` treat it as owned.
        if variant.is_some_and(|variant| variant.owns(index)) {
            builder
                .active_cleanups
                .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                    type_: function.params[index].type_.clone(),
                    stack_offset,
                    capacity_slot: None,
                    loop_alias_slot: None,
                    result_wrapper: None,
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
                type_: ParameterType::named("Error"),
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
    // Locals whose address is taken anywhere — never loop-promoted (plan-03 D2),
    // and never given a string capacity shadow (below).
    collect_address_taken_locals(&function.body, &mut builder.address_taken_locals);
    // Pre-allocate the capacity shadow slot for every in-place string self-append
    // target so bind/assign sites can reset it and the prologue can zero it.
    // plan-146-G: which by-ref captures share their owner's capacity shadow —
    // before the self-append prescan, which skips those (they need no slot here).
    builder.prescan_string_shadow_env(&function.name);
    builder.prescan_string_app_resource_base_env(&function.name);
    builder.prescan_string_self_appends(&function.body);
    // plan-142-B: the scratch block the in-place shrink arms keep marks in.
    builder.prescan_self_update_scratch(&function.name, &function.body);
    builder.prescan_string_app_resource_base(&function.body);
    // plan-134-D: the owning stores that read their source for the last time
    // (plan-134-C), so a recursive value's store can move instead of copying.
    builder.move_sites = Some(
        crate::codegen::engine::analysis::last_use::collect_last_use_moves(
            function,
            &builder.type_model,
        ),
    );
    // plan-147-D: which call arguments this function may hand to the callee instead
    // of lending (plan-147-C). Computed once here, like `move_sites` above, so the
    // answer cannot depend on the order the builder reaches the ops in.
    let owned_param_names: std::collections::HashSet<String> = function
        .params
        .iter()
        .enumerate()
        .filter(|(index, _)| variant.is_some_and(|variant| variant.owns(*index)))
        .map(|(_, param)| param.name.clone())
        .collect();
    builder.handover = Some(
        crate::codegen::engine::analysis::handover::collect_handover_args(
            function,
            &builder.type_model,
            functions,
            &owned_param_names,
            callback_referenced_functions,
        ),
    );
    // Small-vector locals that can live in registers for their whole lifetime with
    // no arena block (plan-01-vector).
    builder.promotable_vector_locals =
        promotable_vector_locals(&function.body, &builder.address_taken_locals);
    // plan-86 E, bug-689: `get` bindings that hold an alias into their container
    // (needs address_taken above, and — for the element-bound ones — the promoted
    // vectors, which never hold a block to alias).
    let borrow_gets = {
        let global_written = |ops: &[NirOp], global: &str| {
            builder.ops_reach_store(
                ops,
                crate::codegen::engine::value::store_reach::StoreLeaf::Global(global),
            )
        };
        crate::codegen::engine::analysis::borrow_get::collect_borrow_gets(
            &function.body,
            &builder.address_taken_locals,
            &global_written,
        )
    };
    builder.install_borrow_gets(borrow_gets);
    // bug-784: the destination-passing call sites (after `install_borrow_gets`,
    // which resets the element-update state), and — in a destination variant —
    // the destination parameter.
    builder.element_updates.into_sites =
        crate::codegen::engine::analysis::into_dest::collect_into_sites(
            function,
            &builder.type_model,
            functions,
        );
    builder.element_updates.into_param = variant
        .and_then(|variant| variant.into)
        .and_then(|index| function.params.get(index))
        .map(|param| param.name.clone());
    // bug-770 C: `MUT x = get/getOr(…)` binds whose next statement appends to `x`
    // in place get their owned copy with growth headroom.
    builder.append_headroom_binds =
        crate::codegen::engine::analysis::append_headroom::collect_append_headroom_binds(
            &function.body,
        );
    // bug-766 B: the fixed-size record temporaries kept in the record-temp scratch.
    builder.install_record_temps(&function.body);
    // plan-116-J: what each local holds, for the consuming-parameter deactivation. A
    // whole-function pre-pass rather than an incremental map built during lowering, so
    // it does not depend on the order the builder happens to reach the ops in.
    builder.resource_containment = crate::codegen::resource_containment(&function.body);
    // The alias chains a returned union follows to its owner, and the assignments
    // that must raise what they store (rule 4). A whole-function pre-pass.
    let ownership = {
        let container = |type_: &ParameterType| builder.is_resource_owning_container(type_);
        crate::codegen::resource::cleanup::record_ownership::record_ownership(
            function, functions, &container,
        )
    };
    builder.resource_alias_sources = ownership.alias_sources;
    builder.statement_temps.ref_cells = builder.by_ref_resource_captures(&function.body);
    // A container that a store raises an inner scope's value into lists what the
    // store hands it, so it needs its owned-lists from its bind, like a float
    // target.
    builder
        .owner_containers
        .extend(ownership.store_floats.containers.iter().cloned());
    builder.store_floats = ownership.store_floats;
    // bug-805: a container local later ASSIGNED a call result adopts what the call
    // hands out, so it needs its owned-lists from its bind, like a float target.
    let call_assigned = builder.call_assigned_owner_containers(&function.body);
    builder.owner_containers.extend(call_assigned);
    // bug-842: and the inline-`TRAP` deliveries of such a call's success value.
    builder.trap_call_results =
        crate::codegen::cleanup::owned::builder_owned_cleanup::trap_call_result_locals(
            &function.body,
        );
    // bug-648: which inline-`TRAP` temps are handed borrowed values.
    builder.trap_ownership =
        crate::codegen::resource::cleanup::trap_ownership::collect_trap_ownership(&function.body);
    // plan-64-I: inline-conversion `CallResult` Result-locals whose trapped error
    // is provably unused, so the error path builds only a tag (no ErrorLoc/Error).
    builder.trap_discard_error_results = trap_discard_error_results(&function.body);
    // bug-778: the inline-`TRAP` temporaries that need no heap `Result` block.
    builder.unboxed_trap_candidates = unboxed_trap_result_candidates(&function.body);
    builder.trap_delivery_moves = trap_delivery_moves(&function.body);
    // plan-86 K1: a pure parameter-passthrough function returns its argument uncopied;
    // the caller copies at its ownership boundary. Only the user-function path sets
    // this — builtin/thread-copy wrappers default false (never param-borrow, and no
    // caller looks them up in `functions`), keeping callee-skip and caller-classify
    // consistent.
    builder.callback_referenced_functions = callback_referenced_functions.clone();
    builder.current_returns_param_borrow =
        function_returns_param_borrow(function, callback_referenced_functions);
    // bug-536 shape B-2: the callee half of "a caller may free this `String`". Set
    // from the SAME predicate every call site consults, so the obligation this
    // function takes on (return a solely-owned block on every path) and the licence
    // the caller takes (free the result at statement end) can never disagree.
    builder.current_returns_fresh_string =
        function_returns_fresh_string(function, callback_referenced_functions);
    {
        // `-vv` span (`crate::trace`): emission is one of the four big
        // per-function costs, and separating it from allocation is what says
        // whether a slow function is slow to *lower* or slow to *color*.
        let _span = crate::trace::span("emit ops");
        // plan-156-E: a parameter that declares a thread view's STATE default-builds it
        // when the view has none yet — the worker entry's own view arrives without one
        // (the runtime builds the view, not its STATE); a view that already carries
        // STATE is left alone. The same lazy init a `RES … STATE` binding runs.
        for param in &function.params {
            if let ParameterType::Stateful { base, state } = &param.type_ {
                if matches!(**base, ParameterType::ThreadHandle { .. }) {
                    if let Some(slot) = builder.locals.get(&param.name).map(|l| l.stack_offset) {
                        builder.emit_resource_state_init(slot, state, &param.type_)?;
                    }
                }
            }
        }
        builder.lower_ops(&function.body)?;
        if !builder.current_block_returns() {
            builder.emit_return_exit(None, None)?;
        }
        if let Some((label, check)) = stack_overflow {
            // A body that lowered to no call at all (its NIR calls were inlined
            // builtins) cannot recurse: drop the check, which would otherwise cost
            // a leaf its leaf frame through the cold block's call.
            let calls = builder.instructions[check.end..].iter().any(|instruction| {
                matches!(
                    instruction.op,
                    crate::arch::ops::CodeOp::BranchLink
                        | crate::arch::ops::CodeOp::BranchLinkRegister
                )
            });
            if calls {
                builder.emit_stack_overflow_block(&label)?;
            } else {
                builder.instructions.drain(check);
            }
        }
    }
    closure_env_layouts.append(&mut builder.closure_env_layouts);
    // Fuse single-use `a*b ± c` float chains into one single-rounded fused op
    // (plan-02 Phase 3) before allocation, so the fused op's operands are colored
    // as a unit. A no-op unless the `d`-native FP virtual registers are present.
    {
        let _span = crate::trace::span("fma fusion");
        fma_fusion::fuse_scalar_fma(&mut builder.instructions);
    }
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
    {
        let _span = crate::trace::span("peephole: store-to-load");
        peephole::forward_stores_to_loads(&mut instructions, is_x86);
    }
    // Drop the GP shuttle a checked float value round-trips through (plan-16). The
    // FP-shuttle liveness derives its call-clobber mask from the active backend's
    // register model, not from operand spellings (bug-350).
    {
        let _span = crate::trace::span("peephole: fp shuttles");
        peephole::remove_fp_shuttles(&mut instructions, mir::active_backend().register_model());
    }
    let mut stack_slots = builder.stack_slots;
    let frame = {
        let _span = crate::trace::span("finalize frame");
        finalize_frame(
            &mut instructions,
            &mut stack_slots,
            builder.stack_size,
            builder.used_callee_saved,
        )
    };
    crate::trace::count("machine instructions", instructions.len() as u64);
    // plan-118-A: the size twin of the "slowest lower_function" leaderboard a
    // few lines of instrumentation up the stack. Lowering time and emitted size
    // are different axes — a function can be slow to lower because it is
    // quadratic in something small, or fast to lower and still be 6% of the
    // module — so "is the expansion one pathological function or all of them?"
    // is only answerable from this one.
    crate::trace::size_item(
        "lower_function",
        || function.name.clone(),
        instructions.len() as u64,
    );

    Ok(CodeFunction {
        name: match variant {
            Some(OwnedVariant {
                into: Some(param), ..
            }) => format!("{}$into{param}", function.name),
            Some(variant) => format!("{}$own{:x}", function.name, variant.mask),
            None => function.name.clone(),
        },
        symbol: emitted_symbol,
        params,
        returns: function.returns.name().into_owned(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}
