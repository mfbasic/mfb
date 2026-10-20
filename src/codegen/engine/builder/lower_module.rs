//! plan-176-U: the module lowering — the `lower_module_for_platform` orchestrator, its function
//! lowering (`lower_module_functions`), its MIR variant and `variant_demand`. The regions it
//! calls in order live in `module_layout.rs`, `module_helpers.rs` and `module_finish.rs`; the
//! builder types and shared imports in `engine/builder/mod.rs`.

use super::*;

use crate::codegen::memory::arena::closure_ops::ClosureHelperTables;

pub(crate) fn lower_module_for_platform(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    platform: &dyn CodegenPlatform,
) -> Result<NativeCodePlan, String> {
    // One virtual-register id sequence per module (bug-721): the ids — and so the
    // allocation — depend only on what this module lowers.
    crate::codegen::engine::regalloc::reset_vreg_ids();
    if module.target != platform.target() {
        return Err(format!(
            "native code platform '{}' cannot lower module target '{}'",
            platform.target(),
            module.target
        ));
    }
    // Install this platform's backend so the builders, helper routing, and helper
    // finalization dispatch selection + register allocation through it instead of
    // naming AArch64 directly (plan-00-H/I additivity).
    mir::set_backend(platform.backend());
    // plan-156-A: the per-type thread value functions this module's call sites
    // request are collected while its functions lower; start from none.
    crate::codegen::runtime::thread::value_functions::reset();
    // bug-806: likewise the module-owned resource kinds its stores list.
    crate::codegen::resource::module_owner::reset_requested_kinds();
    // Imported packages are now decoded and merged into the project IR upstream
    // (see `lower::lower_project`) and lowered as ordinary functions through this
    // same codegen. The legacy flat binary_repr -> native package bridge is no
    // longer used: there are no separate package exports to lower here — the one
    // thing still read straight off the `.mfp` is each binding's native library
    // locator table (plan-46-C), which is per-target and so cannot be resolved
    // upstream of this per-flavor pass.
    let link_libraries = crate::trace::timed("resolve link libraries", || {
        resolve_link_libraries(module, packages, platform)
    })?;
    let mut function_symbols = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), nir::function_symbol(&function.name)))
        .collect::<HashMap<_, _>>();
    for import in &module.imports {
        function_symbols.insert(import.name.clone(), import.symbol.clone());
    }
    let mut globals = module
        .globals
        .iter()
        .enumerate()
        .map(|(index, global)| {
            (
                global.name.clone(),
                GlobalValue {
                    type_: global.type_.clone(),
                    offset: ENTRY_GLOBALS_OFFSET + index * 8,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let functions = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), function))
        .collect::<HashMap<_, _>>();
    let platform_imports = native_plan
        .platform_imports
        .iter()
        .map(|import| (import.symbol.clone(), import.library.clone()))
        .collect::<HashMap<_, _>>();
    let imports = native_plan
        .platform_imports
        .iter()
        .map(|import| CodeImport {
            library: import.library.clone(),
            symbol: import.symbol.clone(),
        })
        .collect::<Vec<_>>();
    // Native `LINK` function return types (keyed `alias.func`) so calls used in
    // expressions resolve their result type (plan-linker.md §12).
    // The LINK block's C-boundary type strings are a genuine string source;
    // parse once here (plan-104-B census: type-valued map).
    let mut package_return_types: HashMap<String, ParameterType> = HashMap::new();
    for function in &module.link_functions {
        package_return_types.insert(
            format!("{}.{}", function.alias, function.name),
            function.return_type.clone(),
        );
    }
    for import in &module.imports {
        // Re-export aliases route to a thunk; map the alias call name to the
        // thunk's return type too (plan-link-update.md §5a).
        if let Some(returns) = module
            .link_functions
            .iter()
            .find(|function| {
                nir::link_thunk_symbol(&function.alias, &function.name) == import.symbol
            })
            .map(|function| function.return_type.clone())
        {
            package_return_types
                .entry(import.name.clone())
                .or_insert(returns);
        }
    }
    let package_global_count = 0usize;
    // The module-level steps below each walk every function, so `-vv` gives
    // each a row: without them their cost lands in this stage's self time, which
    // is exactly the "where did the rest of the minute go" gap the profiler
    // exists to close.
    // plan-140-B: the type model is built before the string pre-pass, which reads
    // its enum table (the two must agree on which types are enums).
    let type_model = crate::trace::timed("type model", || {
        TypeModel::from_module_and_packages(module, packages)
    })?;
    // bug-806: a module whose globals can hold a resource reserves one more
    // writable slot, just past its own globals, for the head of the instance's
    // module-owned resource list (`resource::module_owner`), and emits the teardown
    // that closes them. Every other program keeps its exact layout and code.
    let module_resource_slots = usize::from(module.globals.iter().any(|global| {
        crate::codegen::resource::module_owner::model_type_reaches_resource(
            &type_model,
            &global.type_,
        )
    }));
    if module_resource_slots > 0 {
        globals.insert(
            crate::codegen::resource::module_owner::MODULE_RESOURCES_GLOBAL.to_string(),
            GlobalValue {
                type_: ParameterType::Integer,
                offset: ENTRY_GLOBALS_OFFSET + module.globals.len() * 8,
            },
        );
    }
    let module_teardown_symbol = (module_resource_slots > 0)
        .then_some(crate::codegen::resource::module_owner::MODULE_RESOURCE_DRAIN_SYMBOL);
    let string_symbols =
        crate::trace::timed("string symbols", || string_symbols(module, &type_model));
    // plan-176-U: the eight tables every lowering below threads through; none changes after this.
    let tables = ClosureHelperTables {
        function_symbols: &function_symbols,
        functions: &functions,
        package_return_types: &package_return_types,
        platform_imports: &platform_imports,
        platform,
        build_mode: module.build_mode,
        globals: &globals,
        string_symbols: &string_symbols,
    };
    let mut data_objects =
        module_layout::module_data_objects(module, native_plan, platform, &string_symbols);
    // bug-377: the close thunks, by symbol. Every consumer of a resource's
    // registered close op resolves it through `resolve_closer_symbol`, so the
    // scope-drop call site and the thunk's own "am I a close op?" test cannot
    // disagree about which function is the closer — which is exactly how an
    // imported resource ended up closed by the drop path while its thunk never
    // set `RESOURCE_CLOSED_BIT`.
    let close_op_symbols: HashSet<String> = type_model
        .resource_closers
        .values()
        .filter_map(|close| resolve_closer_symbol(close, &function_symbols))
        .collect();
    let mut code_functions = Vec::new();
    let mut runtime_symbols = native_plan.runtime_symbols.clone();
    let layout = module_layout::plan_arena_layout(
        module,
        platform,
        &function_symbols,
        package_global_count,
        module_resource_slots,
        module_teardown_symbol,
        &mut runtime_symbols,
        &mut data_objects,
        &mut code_functions,
    );
    module_layout::emit_program_entries(
        module,
        platform,
        &platform_imports,
        &layout,
        &mut code_functions,
        &mut data_objects,
    )?;
    let (constructor_types, closure_env_layouts) =
        lower_module_functions(module, &tables, &type_model, &mut code_functions)?;
    module_helpers::lower_graph_and_closure_helpers(
        &tables,
        &type_model,
        closure_env_layouts,
        &mut code_functions,
        &mut data_objects,
    )?;
    let flags = module_helpers::lower_core_runtime_helpers(
        module,
        platform,
        &platform_imports,
        &layout,
        &runtime_symbols,
        &mut code_functions,
    )?;
    module_finish::add_companion_runtime_symbols(module, &mut runtime_symbols);
    module_helpers::lower_runtime_helpers_and_threads(
        module,
        &tables,
        &type_model,
        &layout,
        flags,
        &mut runtime_symbols,
        &mut code_functions,
        &mut data_objects,
    )?;
    module_finish::lower_relocation_gated_helpers(
        &tables,
        &type_model,
        &constructor_types,
        &mut code_functions,
    )?;
    module_finish::lower_canvas_and_link_support(
        module,
        platform,
        &platform_imports,
        &layout,
        &type_model,
        &link_libraries,
        &close_op_symbols,
        &mut code_functions,
        &mut data_objects,
    )?;
    module_finish::add_relocation_gated_data_objects(module, &code_functions, &mut data_objects)?;
    // MIR seam for the hand-written runtime helpers (plan-00-F). Builder-emitted
    // functions already round-trip through the neutral MIR at their
    // pre-allocation seam (`run_register_allocation`); the helpers do not pass
    // through the builder, so they enter the MIR pipeline here — the entry
    // sequence, the arena allocator, the error path, the PCG64 RNG, the kernels,
    // and the thread trampoline all flow through the neutral MIR (plan-00-G: the
    // sole code path).
    {
        let _span = crate::trace::span("route helpers through MIR");
        for function in &mut code_functions {
            mir::route_function_through_mir(function);
        }
    }

    // plan-56-A §4.2: bind any relocation whose `library` was deferred at emit
    // time (the empty-string placeholder) to the library the platform import
    // list declares for that symbol.
    //
    // Done here, over EVERY assembled function, rather than at each emitter:
    // the Linux/GTK app-mode helpers are produced by half a dozen separate
    // `CodegenPlatform` hooks, and binding per-hook means a hook added later
    // silently ships relocations labelled with no library at all — which is
    // exactly what happened when this was first written per-entry-point.
    crate::trace::timed("bind relocation libraries", || {
        bind_deferred_relocation_libraries(&mut code_functions, &platform_imports)
    })?;

    let plan = NativeCodePlan {
        target: module.target.clone(),
        build_mode: module.build_mode,
        arch: platform.arch().to_string(),
        project: module.project.clone(),
        entry_symbol: module.entry.as_ref().map(|_| "_main".to_string()),
        imports,
        data_objects,
        functions: code_functions,
    };
    // `validate` opens its own `-vv` span, so both this call and the backend's
    // second one on the returned plan are attributed to the same row.
    plan.validate()?;
    Ok(plan)
}

/// What [`lower_module_functions`] hands back: the `construct.T` census (type, arity) and every
/// capturing `LAMBDA`'s environment layout.
type ModuleFunctions = (Vec<(ParameterType, usize)>, Vec<ClosureEnvLayout>);

/// `lower_module_for_platform`'s function lowering: every NIR function, its owned and destination
/// variants, and the builtin function wrappers. Returns the `construct.T` census and every
/// capturing `LAMBDA`'s environment layout.
fn lower_module_functions(
    module: &NirModule,
    tables: &ClosureHelperTables<'_>,
    type_model: &TypeModel,
    code_functions: &mut Vec<CodeFunction>,
) -> Result<ModuleFunctions, String> {
    let ClosureHelperTables {
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        globals,
        string_symbols,
        ..
    } = *tables;
    // plan-86 K1: functions used as callbacks (FunctionRef) are invoked through an
    // owning ABI, so they are excluded from the parameter-passthrough borrow elision.
    // Computed once for the whole module and shared by every function's lowering.
    let callback_referenced_functions = crate::trace::timed("collect function refs", || {
        collect_function_ref_names(module)
    });
    // plan-118-D: which record types get their own `construct.T`. Decided ONCE
    // for the module, before any function lowers, so a construction site and the
    // synthesis below cannot disagree about whether the function exists.
    let constructor_types = crate::trace::timed("census constructor types", || {
        synthesized_constructor_types(module, type_model)
    });
    let synthesized_constructors = synthesized_constructor_set(&constructor_types);
    crate::trace::count(
        "synthesized construct.T",
        synthesized_constructors.len() as u64,
    );
    // bug-781: every capturing `LAMBDA`'s environment layout, as the functions that
    // create one record it.
    let mut closure_env_layouts: Vec<ClosureEnvLayout> = Vec::new();
    for function in &module.functions {
        // `-vv` (`crate::trace`): the per-function codegen cost, recorded twice —
        // aggregated into the span tree (so its sub-stages nest underneath) and
        // as a leaderboard row keyed by the function's name. Whether the module
        // is slow because one function is pathological or because there are
        // thousands of ordinary ones is the first question a slow native build
        // raises, and only the leaderboard answers it.
        code_functions.push(crate::trace::timed_item(
            "lower_function",
            || function.name.clone(),
            || {
                lower_function(
                    function,
                    function_symbols,
                    functions,
                    package_return_types,
                    platform_imports,
                    platform,
                    module.build_mode,
                    globals,
                    string_symbols,
                    &callback_referenced_functions,
                    &synthesized_constructors,
                    type_model.clone(),
                    &module.project,
                    None,
                    &mut closure_env_layouts,
                )
            },
        )?);
    }
    // plan-147-D: the owned variants. A function lowers once above as its base
    // symbol with every parameter lent; here it lowers once more for each distinct
    // owned-parameter mask an approved call site asks for.
    //
    // The demand is a FIXPOINT: a variant's own body has call sites, and those sites
    // can approve hand-overs of their own, asking for further variants. The set of
    // `(function, mask)` pairs is finite (one function has at most `2^params` masks,
    // and only masks a real site names are ever requested), so the loop terminates.
    // The base lowerings above are never revisited, which is what keeps a function's
    // own symbol and ABI unchanged (plan-147-A §2.3 row S13).
    //
    // Letter D Phase 1 lands the machinery with NO demand: `variant_demand` returns
    // an empty set until Phase 2 wires the caller side up, so codegen here is
    // byte-identical.
    {
        let mut emitted: HashSet<(String, u64)> = HashSet::new();
        let mut pending: Vec<(String, u64)> = variant_demand(
            module,
            functions,
            type_model,
            &callback_referenced_functions,
        )
        .into_iter()
        .collect();
        pending.sort();
        while let Some((name, mask)) = pending.pop() {
            if !emitted.insert((name.clone(), mask)) {
                continue;
            }
            let Some(function) = functions.get(name.as_str()) else {
                return Err(format!(
                    "plan-147-D: an owned variant was demanded for `{name}`, which is \
                     not a function of this module"
                ));
            };
            let base = nir::function_symbol(&function.name);
            let symbol =
                crate::codegen::engine::function::function_lowering::OwnedVariant::symbol_for(
                    &base, mask,
                );
            // The collision guard the naming scheme deliberately does NOT assume away:
            // a mangled segment can be lowercase (a package prefix is), so `own1f` is
            // not structurally impossible as a type segment. Fail loudly rather than
            // alias two functions.
            if function_symbols
                .values()
                .any(|existing| *existing == symbol)
            {
                return Err(format!(
                    "plan-147-D: the owned-variant symbol `{symbol}` already names a \
                     function of this module"
                ));
            }
            let variant = crate::codegen::engine::function::function_lowering::OwnedVariant {
                mask,
                into: None,
                symbol,
            };
            code_functions.push(lower_function(
                function,
                function_symbols,
                functions,
                package_return_types,
                platform_imports,
                platform,
                module.build_mode,
                globals,
                string_symbols,
                &callback_referenced_functions,
                &synthesized_constructors,
                type_model.clone(),
                &module.project,
                Some(&variant),
                &mut closure_env_layouts,
            )?);
        }
        crate::trace::count("owned variants", emitted.len() as u64);
    }
    // bug-784: the destination variants (`analysis::into_dest`). The sites are a
    // function of each function's NIR alone, which every lowering of it — base,
    // owned or destination variant — shares, so this one pass covers them all.
    {
        let mut demand: Vec<(String, usize)> =
            crate::codegen::engine::analysis::into_dest::demanded_into_variants(
                &module.functions,
                type_model,
                functions,
            )
            .into_iter()
            .collect();
        demand.sort();
        for (name, param) in &demand {
            let Some(function) = functions.get(name.as_str()) else {
                return Err(format!(
                    "bug-784: a destination variant was demanded for `{name}`, which is \
                     not a function of this module"
                ));
            };
            let base = nir::function_symbol(&function.name);
            let symbol =
                crate::codegen::engine::function::function_lowering::OwnedVariant::into_symbol_for(
                    &base, *param,
                );
            if function_symbols
                .values()
                .any(|existing| *existing == symbol)
            {
                return Err(format!(
                    "bug-784: the destination-variant symbol `{symbol}` already names a \
                     function of this module"
                ));
            }
            let variant = crate::codegen::engine::function::function_lowering::OwnedVariant {
                mask: 0,
                into: Some(*param),
                symbol,
            };
            code_functions.push(lower_function(
                function,
                function_symbols,
                functions,
                package_return_types,
                platform_imports,
                platform,
                module.build_mode,
                globals,
                string_symbols,
                &callback_referenced_functions,
                &synthesized_constructors,
                type_model.clone(),
                &module.project,
                Some(&variant),
                &mut closure_env_layouts,
            )?);
        }
        crate::trace::count("destination variants", demand.len() as u64);
    }
    for (name, type_, symbol) in builtin_function_refs(module) {
        code_functions.push(lower_builtin_function_wrapper(
            &name,
            &type_,
            &symbol,
            function_symbols,
            functions,
            package_return_types,
            platform_imports,
            platform,
            module.build_mode,
            globals,
            string_symbols,
            type_model.clone(),
        )?);
    }
    Ok((constructor_types, closure_env_layouts))
}

/// Lower a module to its **MIR** (`-mir` dump, `mir.md §12a`). Runs the same
/// lowering as [`lower_module_for_platform`] with MIR capture armed, then
/// assembles the captured per-function MIR (virtual registers, pre-allocation)
/// into a [`MirPlan`]. Used by the `-mir` build output; the resulting code plan
/// itself is discarded.
pub(crate) fn lower_module_mir_for_platform(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    platform: &dyn CodegenPlatform,
) -> Result<MirPlan, String> {
    mir::begin_capture();
    let result = lower_module_for_platform(module, native_plan, packages, platform);
    let captured = mir::take_capture();
    let plan = result?;
    Ok(mir::build_mir_plan(&plan, captured))
}

/// plan-147-D: the `(function, owned-parameter mask)` pairs the module's approved
/// call sites ask for.
///
/// The SAME `collect_handover_args` the caller side consults in `emit_call`, run
/// over every function of the module. Asking one analysis twice — rather than
/// deriving the demand some other way — is what guarantees that every variant symbol
/// a caller emits is a variant this loop actually lowers; a mismatch would be an
/// undefined symbol at link time.
fn variant_demand(
    module: &NirModule,
    functions: &HashMap<String, &NirFunction>,
    type_model: &TypeModel,
    callback_referenced: &HashSet<String>,
) -> HashSet<(String, u64)> {
    // A FIXPOINT over `(function, mask)`: a variant's body owns parameters the base
    // does not, so it can approve hand-overs the base cannot and ask for further
    // variants. `fill(collections::append(xs, n), n - 1)` is the shape that needs it —
    // only inside `fill$own1` is `xs` owned, and only there is the recursive call's
    // argument handed on. The set is finite (one function has at most `2^params`
    // masks, and only masks a real site names are requested), so this terminates.
    let empty = crate::codegen::engine::analysis::handover::ParamSet::new();
    let mut demand: HashSet<(String, u64)> = HashSet::new();
    let mut pending: Vec<(String, u64)> = Vec::new();
    for function in &module.functions {
        for (name, mask) in crate::codegen::engine::analysis::handover::collect_handover_args(
            function,
            type_model,
            functions,
            &empty,
            callback_referenced,
        )
        .demanded_variants()
        {
            if demand.insert((name.clone(), mask)) {
                pending.push((name, mask));
            }
        }
    }
    while let Some((name, mask)) = pending.pop() {
        let Some(function) = functions.get(name.as_str()) else {
            continue;
        };
        let owned: crate::codegen::engine::analysis::handover::ParamSet = function
            .params
            .iter()
            .enumerate()
            .filter(|(index, _)| *index < 64 && mask & (1u64 << index) != 0)
            .map(|(_, param)| param.name.clone())
            .collect();
        for (next, next_mask) in crate::codegen::engine::analysis::handover::collect_handover_args(
            function,
            type_model,
            functions,
            &owned,
            callback_referenced,
        )
        .demanded_variants()
        {
            if demand.insert((next.clone(), next_mask)) {
                pending.push((next, next_mask));
            }
        }
    }
    demand
}
