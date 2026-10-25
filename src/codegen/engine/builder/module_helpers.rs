//! plan-176-U: `lower_module_for_platform`'s graph/closure helpers, core runtime helpers, and
//! runtime helpers and threads. Called in order by `lower_module_for_platform`
//! (`engine/builder/lower_module.rs`).

use super::*;

use super::module_layout::ModuleLayout;
use crate::codegen::memory::arena::closure_ops::ClosureHelperTables;

/// What [`lower_core_runtime_helpers`] decided that the runtime helpers after it read.
#[derive(Clone, Copy)]
pub(super) struct CoreHelperFlags {
    pub(super) debug_arena: bool,
    pub(super) uses_stdout_buffer: bool,
    pub(super) uses_stdin: bool,
}

/// `lower_module_for_platform`'s graph and closure helpers: the recursive-copy shims, every closure
/// environment routine, the graph copy and drop walkers and the closure copy/drop.
pub(crate) fn lower_graph_and_closure_helpers(
    tables: &ClosureHelperTables<'_>,
    type_model: &TypeModel,
    closure_env_layouts: Vec<ClosureEnvLayout>,
    code_functions: &mut Vec<CodeFunction>,
    data_objects: &mut Vec<CodeDataObject>,
) -> Result<(), String> {
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
    // A per-type runtime deep-copy entry point for every recursive type, so a
    // recursive value (e.g. `dom::Node`) can be transferred out of a worker arena
    // — `copy_value_to_current_arena` routes such a value's copy to a call to
    // `thread_copy_symbol(type)` rather than recursing over the type inline
    // (bug-391). plan-134-B: each is a shim into the one non-recursive walker,
    // `_mfb_rt_graph_copy`, passing its index in this set's order as the kind.
    let recursive_copy_types = recursive_transfer_types(type_model);
    // Their `arena_alloc`-failure path builds an error with an empty message, so
    // the empty-string data object must exist even if the module's own code did
    // not otherwise require it.
    if !recursive_copy_types.is_empty()
        && !data_objects
            .iter()
            .any(|object| object.symbol == EMPTY_STRING_SYMBOL)
    {
        data_objects.push(string_data_object(EMPTY_STRING_SYMBOL, String::new()));
    }
    // plan-134-H: the drop walker's kinds are `recursive_transfer_types` in its order,
    // then the types that only reach a cycle (`TypeModel::graph_drop_kinds`), so the copy
    // walker's kinds are exactly their prefix. Fail closed if that ever stops holding: a
    // kind index is shared by the shims and both walkers.
    let copy_kind_count = recursive_copy_types.len();
    if type_model.graph_drop_kinds.len() < copy_kind_count
        || !recursive_copy_types
            .iter()
            .zip(&type_model.graph_drop_kinds)
            .all(|(copy_kind, drop_kind)| copy_kind == drop_kind)
    {
        return Err("the graph drop kinds do not start with the graph copy kinds".to_string());
    }
    let mut typed_kinds: Vec<(String, ParameterType)> = Vec::new();
    for (kind, name) in type_model.graph_drop_kinds.iter().enumerate() {
        // `recursive_transfer_types` returns rendered names (the emission ORDER
        // is observable in the `.ncode`); parse each back once, here, for the
        // typed emitters below — the per-type copy shims (cycle members only)
        // and both walkers.
        let type_ = ParameterType::declared(name);
        if kind < copy_kind_count {
            let symbol = thread_copy_symbol(&type_);
            code_functions.push(lower_thread_copy_function(
                &type_,
                kind,
                &symbol,
                function_symbols,
                functions,
                package_return_types,
                platform_imports,
                platform,
                tables.build_mode,
                globals,
                string_symbols,
                type_model.clone(),
            )?);
        }
        typed_kinds.push((name.clone(), type_));
    }
    // bug-781: each capturing `LAMBDA`'s environment routine, once. A `LAMBDA` created
    // in several functions (a cloned body) must have recorded one layout everywhere.
    {
        let mut emitted: std::collections::BTreeMap<String, ClosureEnvLayout> =
            std::collections::BTreeMap::new();
        for layout in closure_env_layouts {
            match emitted.get(&layout.symbol) {
                Some(known) if *known == layout => {}
                Some(known) => {
                    return Err(format!(
                        "closure environment `{}` is created with two layouts: {known:?} and \
                         {layout:?}",
                        layout.symbol
                    ))
                }
                None => {
                    emitted.insert(layout.symbol.clone(), layout);
                }
            }
        }
        for layout in emitted.values() {
            code_functions.push(
                crate::codegen::memory::arena::closure_ops::lower_closure_env_ops(
                    tables,
                    type_model.clone(),
                    layout,
                )?,
            );
        }
    }
    // plan-134-F: the drop walkers exist for a module with recursive types; bug-781: and
    // for one whose function-value-holding composites are dropped through them.
    let references = |functions: &[CodeFunction], symbols: &[&str]| {
        functions.iter().any(|function| {
            function
                .relocations
                .iter()
                .any(|relocation| symbols.contains(&relocation.to.as_str()))
        })
    };
    let drop_walkers_referenced = {
        use crate::codegen::memory::arena::graph_drop::{
            GRAPH_DROP_EDGES_SYMBOL, GRAPH_DROP_SYMBOL,
        };
        references(
            &*code_functions,
            &[GRAPH_DROP_SYMBOL, GRAPH_DROP_EDGES_SYMBOL],
        )
    };
    if !recursive_copy_types.is_empty() {
        code_functions.push(
            crate::codegen::memory::arena::graph_copy::lower_graph_copy_walker(
                &typed_kinds[..copy_kind_count],
                function_symbols,
                functions,
                package_return_types,
                platform_imports,
                platform,
                tables.build_mode,
                globals,
                string_symbols,
                type_model.clone(),
            )?,
        );
        code_functions.push(
            crate::codegen::memory::arena::graph_copy::lower_graph_stack_grow(
                function_symbols,
                functions,
                package_return_types,
                platform_imports,
                platform,
                tables.build_mode,
                globals,
                string_symbols,
                type_model.clone(),
            )?,
        );
    }
    if !recursive_copy_types.is_empty() || drop_walkers_referenced {
        // plan-134-F: the walker's inverse, over the same work stack. plan-134-H: its kinds
        // extend the copy walker's with the types that only reach a cycle, and a second
        // variant drops the graph of an element inlined in a collection's data region.
        use crate::codegen::memory::arena::graph_drop::{
            lower_graph_drop_walker, GRAPH_DROP_EDGES_SYMBOL, GRAPH_DROP_SYMBOL,
        };
        for (symbol, free_root) in [(GRAPH_DROP_SYMBOL, true), (GRAPH_DROP_EDGES_SYMBOL, false)] {
            code_functions.push(lower_graph_drop_walker(
                symbol,
                free_root,
                &typed_kinds,
                function_symbols,
                functions,
                package_return_types,
                platform_imports,
                platform,
                tables.build_mode,
                globals,
                string_symbols,
                type_model.clone(),
            )?);
        }
    }
    // bug-781: a function value's copy and drop, for a module that copies or drops one.
    if references(
        &*code_functions,
        &[CLOSURE_COPY_SYMBOL, CLOSURE_DROP_SYMBOL],
    ) {
        code_functions.push(
            crate::codegen::memory::arena::closure_ops::lower_closure_copy(
                tables,
                type_model.clone(),
            )?,
        );
        code_functions.push(
            crate::codegen::memory::arena::closure_ops::lower_closure_drop(
                tables,
                type_model.clone(),
            )?,
        );
    }
    Ok(())
}

/// `lower_module_for_platform`'s core runtime helpers: the arena, error, stdout/stdin/input/file
/// buffers, process reaper, shutdown, debug report, signal handler and audio callbacks.
pub(super) fn lower_core_runtime_helpers(
    module: &NirModule,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    layout: &ModuleLayout,
    runtime_symbols: &[String],
    code_functions: &mut Vec<CodeFunction>,
) -> Result<CoreHelperFlags, String> {
    let ModuleLayout {
        skip_entry_arena_destroy,
        uses_term,
        input_state_offset,
        register_signal_handlers,
        ..
    } = *layout;
    // plan-130-B: the arena hot path times itself exactly when the `--debug` perf
    // section is active for this module.
    let perf_arena =
        crate::codegen::debug::feature_active(module, crate::codegen::debug::PERF_SECTION);
    // plan-130-C: the allocator counts its events exactly when the `--debug` arena
    // section is active for this module.
    let debug_arena =
        crate::codegen::debug::feature_active(module, crate::codegen::debug::ARENA_SECTION);
    // plan-179-C: the arena memory cap's accounting rides the map and unmap sites.
    let arena_cap = crate::codegen::runtime::arena_cap::arena_cap_enabled(module);
    // plan-183-C: the region allocator — the dispatchers, the slice layout and commit
    // helpers, the small-page and medium/huge run helpers.
    code_functions.push(lower_arena_alloc(perf_arena, debug_arena));
    code_functions.push(crate::codegen::memory::arena::arena::lower_arena_geometry(
        debug_arena,
    ));
    code_functions.push(crate::codegen::memory::arena::arena::lower_arena_commit(
        platform,
        debug_arena,
        arena_cap,
        platform_imports,
    )?);
    code_functions.push(crate::codegen::memory::arena::arena::lower_arena_small_page(debug_arena));
    code_functions.push(crate::codegen::memory::arena::arena::lower_arena_span_alloc(false));
    code_functions.push(crate::codegen::memory::arena::arena::lower_arena_span_alloc(true));
    code_functions.push(crate::codegen::memory::arena::arena::lower_arena_span_free(
        debug_arena,
    ));
    // plan-183-E: memory the program stopped using goes back to the OS.
    code_functions.push(crate::codegen::memory::arena::arena::lower_arena_decommit(
        platform,
        debug_arena,
        arena_cap,
        platform_imports,
    )?);
    code_functions.push(crate::codegen::memory::arena::arena::lower_arena_sweep());
    // plan-183-D: collection growth tries to extend in place first.
    code_functions.push(crate::codegen::memory::arena::arena::lower_arena_extend(
        debug_arena,
    ));
    code_functions.push(lower_build_error_loc());
    code_functions.push(lower_make_error_result());
    code_functions.push(lower_simd_alloc_list());
    code_functions.push(lower_arena_free(perf_arena, debug_arena));
    // The arena's zero fill is always on: it scrubs freed chunks (plan-183-A: fresh
    // OS memory is already zero and is never filled).
    code_functions.push(lower_arena_fill_zero(platform.arch()));
    code_functions.push(lower_arena_destroy(
        platform,
        debug_arena,
        arena_cap,
        platform_imports,
    )?);
    // Opt-in stdout buffering (plan-14-A): the shared `_mfb_rt_io_stdout_drain`
    // helper is emitted whenever any stdout writer, stdin reader, or buffering
    // control is present — every point that references the drain. App mode has no
    // stdout buffer (transcript writes are synchronous), so it is excluded. When
    // present, `_mfb_shutdown` drains the buffer at exit as well.
    let uses_stdout_buffer = !module.build_mode.is_app()
        && runtime_symbols.iter().any(|symbol| {
            matches!(
                symbol.as_str(),
                "_mfb_rt_io_io_print"
                    | "_mfb_rt_io_io_write"
                    | "_mfb_rt_io_io_flush"
                    | "_mfb_rt_io_io_setBuffered"
                    | "_mfb_rt_io_io_readLine"
                    | "_mfb_rt_io_io_input"
                    | "_mfb_rt_io_io_readChar"
                    | "_mfb_rt_io_io_readByte"
                    | "_mfb_rt_io_io_readRaw"
                    | "_mfb_rt_io_io_readScalar"
                    | "_mfb_rt_io_io_readGrapheme"
                    | "_mfb_rt_io_io_readKey"
            )
        });
    if uses_stdout_buffer {
        code_functions.push(lower_stdout_drain(platform_imports, platform)?);
    }
    // Stdin broadcast log (plan-15): the shared reader/subscription helpers are
    // emitted whenever the module uses a stdin read builtin, polls the mouse
    // (bug-1099), or uses `thread::openStdIn`/`closeStdIn`.
    // `_mfb_rt_stdin_next_byte` replaces the per-byte `read(0,…)` in
    // every stdin read site; subscribe/unsubscribe/recompute back the broadcast
    // registry and the compat main-thread subscription. App mode reads a window
    // pipe, not fd 0, so it is excluded.
    let uses_stdin = !module.build_mode.is_app()
        && (crate::codegen::io::input::program_reads_mouse(runtime_symbols)
            || runtime_symbols.iter().any(|symbol| {
                matches!(
                    symbol.as_str(),
                    "_mfb_rt_io_io_readLine"
                        | "_mfb_rt_io_io_input"
                        | "_mfb_rt_io_io_readChar"
                        | "_mfb_rt_io_io_readByte"
                        | "_mfb_rt_io_io_readRaw"
                        | "_mfb_rt_io_io_readScalar"
                        | "_mfb_rt_io_io_readGrapheme"
                        | "_mfb_rt_io_io_readKey"
                        | "_mfb_rt_io_io_pollInput"
                        | "_mfb_rt_thread_thread_openStdIn"
                        | "_mfb_rt_thread_thread_closeStdIn"
                )
            }));
    if uses_stdin {
        code_functions.push(lower_stdin_recompute_base(platform_imports, platform)?);
        code_functions.push(lower_stdin_next_byte(
            platform_imports,
            platform,
            module.stdin_log_cap,
        )?);
        code_functions.push(lower_stdin_subscribe(platform_imports, platform)?);
        code_functions.push(lower_stdin_unsubscribe(platform_imports, platform)?);
    }
    // bug-699: the input decoder, for the readers that decode through the input
    // buffer. Reserved and emitted together, so a program without such a reader
    // carries neither.
    if input_state_offset.is_some() {
        code_functions.push(crate::codegen::io::input::decode::lower_input_decode(
            crate::codegen::io::input::program_uses_grapheme_clusters(runtime_symbols),
        ));
    }
    // bug-699: every `--app` window key handler encodes its events through one
    // shared encoder, so the window input pipe carries the kitty-protocol form a
    // terminal under `term::on` sends. The handlers are part of every app build.
    if module.build_mode.is_app() {
        code_functions.push(crate::codegen::io::input::encode::lower_input_encode_key()?);
    }
    // App-mode io members lower their transcript-write / prompt+read / present
    // sequences inline (plan-101 append shape): each io `abi_function` body calls
    // the platform's `emit_app_io_*` to append its GUI sequence directly, and the
    // member's own vreg finalizer builds the frame. There is no standalone app-io
    // helper to emit here any more.
    // Per-File output buffering (plan-14-B): the shared `_mfb_rt_fs_file_drain`
    // helper is referenced by fs.close (mandatory flush-on-close), the buffered
    // writeAll/writeAllBytes overflow paths, fs.flush, and fs.setBuffered(FALSE).
    let uses_file_buffer = runtime_symbols.iter().any(|symbol| {
        matches!(
            symbol.as_str(),
            "_mfb_rt_fs_fs_close"
                | "_mfb_rt_fs_fs_writeAll"
                | "_mfb_rt_fs_fs_writeAllBytes"
                | "_mfb_rt_fs_fs_flush"
                | "_mfb_rt_fs_fs_setBuffered"
                // bug-809: a seek drains pending output at the old position first.
                | "_mfb_rt_fs_fs_seek"
        )
    });
    if uses_file_buffer {
        code_functions.push(
            crate::codegen::builtins::fs::gen_handle::lower_fs_file_drain(
                platform_imports,
                platform,
            )?,
        );
    }
    // bug-701: the path classifier the `fs` host-error mapping calls, emitted
    // wherever a member that maps a host error is.
    if runtime_symbols.iter().any(|symbol| {
        symbol
            .strip_prefix("_mfb_rt_fs_fs_")
            .is_some_and(crate::codegen::builtins::fs::gen_classify::member_classifies_paths)
    }) {
        code_functions.push(
            crate::codegen::builtins::fs::gen_classify::lower_fs_path_class_helper(
                platform_imports,
                platform,
            )?,
        );
    }
    // bug-474: `process::detach` reaps its one child on a dedicated thread whose
    // start routine is `_mfb_rt_process_reaper`, so that helper must be emitted
    // wherever `process.detach` is (the detach body takes its address). It replaced
    // a process-wide `signal(SIGCHLD, SIG_IGN)`, which auto-reaped EVERY child and
    // left `process::waitFor` on an unrelated handle reporting `0`. Windows
    // `detach` only closes handles, so it has no reaper.
    if platform.family() != PlatformFamily::Windows
        && runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_process_process_detach")
    {
        code_functions.push(
            crate::codegen::builtins::process::lower_process_reaper_helper(
                platform_imports,
                platform,
            )?,
        );
    }
    // plan-179-B: the arena memory cap's startup check, which the program entry
    // calls before the first allocation.
    if crate::codegen::runtime::arena_cap::arena_cap_enabled(module) {
        code_functions.push(crate::codegen::runtime::arena_cap::lower_arena_cap_init(
            &module.arena_memory,
            platform_imports,
            platform,
        )?);
    }
    // plan-183-B: the startup reservation, which the entry calls right after the cap
    // check.
    if module.entry.is_some() {
        code_functions.push(crate::codegen::runtime::arena_cap::lower_arena_reserve(
            module,
            platform_imports,
            platform,
        )?);
    }
    if module.entry.is_some() {
        code_functions.push(lower_shutdown(
            uses_term,
            skip_entry_arena_destroy,
            uses_stdout_buffer,
            module_uses_call(module, "canvas.startGraphics"),
            platform_imports,
            platform,
            crate::codegen::debug::report_enabled(module),
            arena_cap,
        )?);
        // plan-130-A: `_mfb_debug_shutdown` and every report section it calls.
        code_functions.extend(crate::codegen::debug::code_functions(
            module,
            platform_imports,
            platform,
        )?);
    }
    if register_signal_handlers {
        code_functions.push(lower_signal_handler(platform)?);
    }
    // The macOS AudioQueue callbacks (plan-33-B §3.2): the output callback when an
    // output stream is built and the input callback when an input stream is,
    // since openOutput/openInput take their addresses. The backend owns the
    // platform decision and the symbol gate (bug-330).
    code_functions.extend(AudioBackend::select(platform).callback_functions(
        platform_imports,
        platform,
        runtime_symbols,
    )?);
    Ok(CoreHelperFlags {
        debug_arena,
        uses_stdout_buffer,
        uses_stdin,
    })
}

/// `lower_module_for_platform`'s runtime helpers and threads: the module drain, every runtime
/// helper the plan names, the RNG, UTF-8 validation, TLS trampolines, sort, the thread runtime and
/// every per-type thread value function.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_runtime_helpers_and_threads(
    module: &NirModule,
    tables: &ClosureHelperTables<'_>,
    type_model: &TypeModel,
    layout: &ModuleLayout,
    flags: CoreHelperFlags,
    runtime_symbols: &mut Vec<String>,
    code_functions: &mut Vec<CodeFunction>,
    data_objects: &mut Vec<CodeDataObject>,
) -> Result<(), String> {
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
    let ModuleLayout {
        module_teardown_symbol,
        ref global_initializer_symbol,
        uses_rng,
        term_state_offset,
        presentation_mode_offset,
        mouse_state_offset,
        input_state_offset,
        arena_global_slots,
        link_init_symbol,
        ..
    } = *layout;
    let CoreHelperFlags {
        debug_arena,
        uses_stdout_buffer,
        uses_stdin,
        ..
    } = flags;
    // bug-806: the module instance's teardown, lowered once every function has —
    // it closes only the resource kinds some store listed. A built-in close it
    // calls that no binding of the program named is emitted with the other
    // runtime helpers below.
    if module_teardown_symbol.is_some() {
        let drain = crate::codegen::resource::module_owner::lower_module_resource_drain(
            function_symbols,
            functions,
            package_return_types,
            platform_imports,
            platform,
            module.build_mode,
            globals,
            string_symbols,
            type_model.clone(),
        )?;
        for relocation in &drain.relocations {
            if runtime::spec_for_symbol(&relocation.to).is_some()
                && !runtime_symbols
                    .iter()
                    .any(|symbol| *symbol == relocation.to)
            {
                runtime_symbols.push(relocation.to.clone());
            }
        }
        code_functions.push(drain);
    }
    for symbol in &*runtime_symbols {
        // `-vv`: the hand-written runtime helpers are the other half of the
        // emitted code, and they scale with the *builtins the program touches*
        // rather than with its own source. Separating them from
        // `lower_function` is what tells a "my program is small but the build is
        // slow" report which half to look at.
        code_functions.push(crate::trace::timed_item(
            "lower_runtime_helper",
            || symbol.clone(),
            || {
                lower_runtime_helper(
                    symbol,
                    module.build_mode,
                    &module.project,
                    ArenaLayout {
                        term_state_offset,
                        presentation_mode_offset,
                        mouse_state_offset,
                        input_state_offset,
                        global_slots: arena_global_slots,
                        debug_arena_registry: crate::codegen::debug::feature_active(
                            module,
                            crate::codegen::debug::ARENA_SECTION,
                        ),
                        arena_cap: crate::codegen::runtime::arena_cap::arena_cap_enabled(module),
                    },
                    uses_rng,
                    type_model,
                    platform_imports,
                    platform,
                    // plan-98-C: the real string table, not an empty one — an
                    // `abi_function` body with a string literal needs its data object.
                    string_symbols,
                )
            },
        )?);
    }
    if uses_rng {
        code_functions.push(lower_rng_next());
        code_functions.push(lower_rng_seed_at());
    }
    // A wrapper copies a C string out when `RETURN` names the ABI return and that
    // return is a `CPtr` surfaced as an owned `String` (plan-50-H), OR when a
    // struct slot has a `CString` field (plan-50-F). This must agree EXACTLY with
    // `lower_link_thunk`'s `needs_encoding`, or the thunk references
    // `_mfb_rt_validate_utf8` and the helper is never emitted — a link error, not
    // a test failure.
    let link_returns_cstring = module.link_functions.iter().any(|function| {
        let returns_c_string = matches!(&function.result, Some(crate::ir::IrLinkExpr::Var(name)) if *name == function.abi_return_name)
            && function.abi_return_ctype.is_c_abi(CAbiType::Ptr)
            && matches!(function.return_type, ParameterType::String);
        let struct_has_cstring_field = function.abi_slots.iter().any(|slot| {
            module
                .link_cstructs
                .iter()
                .find(|c| c.alias == function.alias && slot.ctype.is_named(&c.name))
                .is_some_and(|c| c.fields.iter().any(|f| f.ctype.is_c_abi(CAbiType::Str)))
        });
        returns_c_string || struct_has_cstring_field
    });
    if link_returns_cstring
        || module_uses_any_call(module, &["os.args", "os.prog"])
        || runtime_symbols.iter().any(|symbol| {
            matches!(
                symbol.as_str(),
                "_mfb_rt_fs_fs_readText"
                    | "_mfb_rt_fs_fs_readAll"
                    | "_mfb_rt_fs_fs_readLine"
                    // bug-809: the raw line reader validates its line the same way.
                    | "_mfb_rt_fs_fs_readLineRaw"
                    | "_mfb_rt_net_net_readText"
                    | "_mfb_rt_net_net_receiveTextFrom"
                    | "_mfb_rt_process_process_receive"
                    | "_mfb_rt_process_process_receiveFrom"
            )
        })
    {
        code_functions.push(lower_validate_utf8_helper());
    }
    // The macOS TLS backend bridges Network.framework's async callbacks to a
    // semaphore via small emitted block-invoke functions. Their register layout
    // is the foreign-runtime callback ABI, so each backend emits its own
    // (per-(OS, ISA) machine floor); non-macOS / OpenSSL TLS returns none.
    if runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_tls_"))
    {
        let tls_server = runtime_symbols
            .iter()
            .any(|symbol| is_tls_server_symbol(symbol));
        code_functions.extend(platform.emit_tls_block_trampolines(tls_server));
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_fs_fs_listDirectory")
    {
        code_functions.push(lower_sort_string_list_helper());
    }
    let uses_thread_start = runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_thread_thread_start");
    if module.entry.is_some() {
        code_functions.push(crate::codegen::error::report::lower_error_report(
            uses_thread_start,
            platform_imports,
            platform,
        )?);
    }
    if uses_thread_start {
        code_functions.push(platform.emit_thread_trampoline(
            platform_imports,
            uses_stdin,
            ArenaInitSymbols {
                link_init: link_init_symbol,
                global_init: global_initializer_symbol.as_deref(),
                teardown: module_teardown_symbol,
            },
        )?);
        code_functions.push(crate::codegen::runtime::thread::lower_thread_finish(
            uses_stdout_buffer,
            // `_mfb_shutdown` is emitted exactly when the module has an entry.
            module.entry.is_some(),
            platform_imports,
            platform,
        )?);
    }
    // plan-156-A: the thread table, the control block's block arena and its release —
    // shared by every `thread` helper that starts or lets go of a thread. Gated on the
    // release being referenced, not on `thread::start` alone: the parent view's drop
    // (`thread.drop`) releases a block too, and a module can hold a `Thread` it did not
    // start (a helper binding one it was lent).
    let uses_block_release = uses_thread_start
        || code_functions.iter().any(|function| {
            function.relocations.iter().any(|relocation| {
                relocation.to == crate::codegen::runtime::thread::THREAD_BLOCK_RELEASE_SYMBOL
            })
        });
    if uses_block_release {
        use crate::codegen::runtime::thread::{
            lower_thread_block_arena_create, lower_thread_block_arena_destroy,
            lower_thread_block_release, lower_thread_table_acquire, lower_thread_table_release,
            thread_table_data_object,
        };
        code_functions.push(lower_thread_table_acquire(platform_imports, platform)?);
        // plan-183-B: the arena slices each thread takes and gives back.
        code_functions.push(
            crate::codegen::runtime::thread::lower_thread_slices_acquire(
                crate::codegen::runtime::arena_cap::arena_slice_count(module),
                crate::codegen::runtime::arena_cap::fixed_slice_count(module),
                platform_imports,
                platform,
            )?,
        );
        code_functions.push(
            crate::codegen::runtime::thread::lower_thread_slices_release(
                platform_imports,
                platform,
            )?,
        );
        data_objects.push(crate::codegen::runtime::thread::thread_limit_data_object(
            module.arena_memory.max_threads,
        ));
        code_functions.push(lower_thread_table_release(platform_imports, platform)?);
        code_functions.push(lower_thread_block_arena_create(
            debug_arena,
            platform_imports,
            platform,
        )?);
        code_functions.push(lower_thread_block_arena_destroy(
            debug_arena,
            platform_imports,
            platform,
        )?);
        code_functions.push(lower_thread_block_release(
            debug_arena,
            platform_imports,
            platform,
        )?);
        data_objects.push(thread_table_data_object(platform.family()));
    }
    // plan-156-A: every per-type copy/drop function a thread call site (or the
    // worker exit above) asked for. Unconditional: a module can send or receive
    // without starting a thread (a package's worker entry). Emitted before the
    // relocation-gated shared helpers below, which these functions call (a `String`
    // message's drop is `_mfb_rt_drop_owned_string`).
    for (symbol, kind, type_) in crate::codegen::runtime::thread::value_functions::take_requested()
    {
        code_functions.push(
            crate::codegen::runtime::thread::lower_thread_value_function(
                kind,
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
            )?,
        );
    }
    Ok(())
}
