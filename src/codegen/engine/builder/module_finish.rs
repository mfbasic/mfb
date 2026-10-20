//! plan-176-U: `lower_module_for_platform`'s companion runtime symbols, relocation-gated
//! helpers, canvas and `LINK` support, and relocation-gated data objects. Called in order by
//! `lower_module_for_platform` (`engine/builder/lower_module.rs`).

use super::*;

use super::module_layout::ModuleLayout;
use crate::codegen::memory::arena::closure_ops::ClosureHelperTables;

/// `lower_module_for_platform`'s companion runtime symbols: the helpers a planned runtime symbol
/// calls that the plan does not list itself.
pub(super) fn add_companion_runtime_symbols(module: &NirModule, runtime_symbols: &mut Vec<String>) {
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_fs_fs_readBytes")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_fs_fs_readAllBytes")
    {
        runtime_symbols.push("_mfb_rt_fs_fs_readAllBytes".to_string());
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_fs_fs_writeTextAtomic")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_fs_fs_writeText")
    {
        runtime_symbols.push("_mfb_rt_fs_fs_writeText".to_string());
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_fs_fs_writeBytesAtomic")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_fs_fs_writeBytes")
    {
        runtime_symbols.push("_mfb_rt_fs_fs_writeBytes".to_string());
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_thread_thread_receive")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_thread_thread_read")
    {
        runtime_symbols.push("_mfb_rt_thread_thread_read".to_string());
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_thread_thread_send")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_thread_thread_emit")
    {
        runtime_symbols.push("_mfb_rt_thread_thread_emit".to_string());
    }
    // The resource plane mirrors the data plane's direction split: the NIR carries
    // the pre-split `transferResource`/`acceptResource` target, while codegen may
    // route a worker-handle call to `emitResource` (outbound write) or a
    // parent-handle call to `readResource` (outbound read). Emit the companion so
    // whichever direction codegen selects has a defined helper.
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_thread_thread_transferResource")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_thread_thread_emitResource")
    {
        runtime_symbols.push("_mfb_rt_thread_thread_emitResource".to_string());
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_thread_thread_acceptResource")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_thread_thread_readResource")
    {
        runtime_symbols.push("_mfb_rt_thread_thread_readResource".to_string());
    }
    // The `connectTcp(Address)` overload routes to a paired helper that shares
    // `connectTcp`'s libc imports; emit it whenever `connectTcp` is present.
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_net_net_connectTcp")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_net_net_connectTcpAddr")
    {
        runtime_symbols.push("_mfb_rt_net_net_connectTcpAddr".to_string());
    }
    // plan-110-A: `ping(Address, …)` routes to `net.pingAddr` the same way — the
    // NIR names only `net.ping`, so without this the call site relocates against a
    // symbol nothing defines. It shares `ping`'s libc imports.
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_net_net_ping")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_net_net_pingAddr")
    {
        runtime_symbols.push("_mfb_rt_net_net_pingAddr".to_string());
    }
    // plan-110-B: tcp's three synthesized code forms. The NIR names only the base
    // member (`tcp.connect`/`tcp.poll`/`tcp.write`); `builder_values` picks the
    // variant at emission, so each variant's body has to be forced into the symbol
    // set whenever its base is present or the call site relocates against a symbol
    // nothing defines.
    for (base, synthesized) in [
        ("_mfb_rt_tcp_tcp_connect", "_mfb_rt_tcp_tcp_connectAddr"),
        ("_mfb_rt_tcp_tcp_poll", "_mfb_rt_tcp_tcp_pollList"),
        ("_mfb_rt_tcp_tcp_write", "_mfb_rt_tcp_tcp_writeText"),
        // plan-110-C: udp's two, on the same rule.
        ("_mfb_rt_udp_udp_poll", "_mfb_rt_udp_udp_pollList"),
        ("_mfb_rt_udp_udp_send", "_mfb_rt_udp_udp_sendText"),
        // plan-110-D: tls's write split and Address connect, on the same rule.
        ("_mfb_rt_tls_tls_write", "_mfb_rt_tls_tls_writeText"),
        ("_mfb_rt_tls_tls_connect", "_mfb_rt_tls_tls_connectAddr"),
        // bug-465: `tls::localAddress` spans Socket and Listener, and the two
        // cannot share a body (macOS asks an `nw_connection` and an `nw_listener`
        // different questions). Same rule as the rest: the NIR names only the base
        // member, so the listener form is forced in beside it.
        (
            "_mfb_rt_tls_tls_localAddress",
            "_mfb_rt_tls_tls_localAddressListener",
        ),
        // bug-809: `fs::openData`'s `List OF Byte` overload, on the same rule.
        ("_mfb_rt_fs_fs_openData", "_mfb_rt_fs_fs_openDataBytes"),
    ] {
        if runtime_symbols.iter().any(|symbol| symbol == base)
            && !runtime_symbols.iter().any(|symbol| symbol == synthesized)
        {
            runtime_symbols.push(synthesized.to_string());
        }
    }
    // plan-90-A: the 4-arg `spawn(args, cwd, env, envReplace)` overload routes to
    // `process.spawnEnv`, a synthesized target the NIR never names (it carries only
    // `process.spawn`), so emit its helper body whenever `spawn` is present —
    // mirroring `connectTcpAddr`. It shares the process libc imports.
    //
    // plan-119-C: this used to be skipped on Windows, on the premise that the
    // synthesized helpers "are stubs there". That premise was wrong for four of
    // the five and is now wrong for all of them. `sendTimeout`,
    // `sendBytesTimeout`, `pollFrom`, `receiveFrom` and `receiveBytesFrom` always
    // had real Windows bodies — each `*_win` entry fn branches on the runtime-call
    // name exactly as its posix twin does — and `win_x86_64` has advertised every
    // one of them in its capability list since plan-90-D. Skipping the push meant
    // the capability was advertised, the call passed validation, and the build
    // then died at link time:
    //
    //     error: native code internal relocation target
    //            '_mfb_rt_process_process_sendTimeout' is not defined
    //
    // reproduced on `main` with a three-argument `process::send` built for
    // `windows-x86_64`. Only `spawnEnv` was genuinely a stub, and plan-119-C gave
    // it a body, so the exclusion has no remaining justification. Emitting these
    // on Windows is what makes the advertised surface true.
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_process_process_spawn")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_process_process_spawnEnv")
    {
        runtime_symbols.push("_mfb_rt_process_process_spawnEnv".to_string());
    }
    // plan-90-B: the 3-arg `send`/`sendBytes` timeout overloads route to
    // `sendTimeout`/`sendBytesTimeout`, synthesized targets the NIR never names —
    // emit each whenever its base is present.
    for (base, timed) in [
        (
            "_mfb_rt_process_process_send",
            "_mfb_rt_process_process_sendTimeout",
        ),
        (
            "_mfb_rt_process_process_sendBytes",
            "_mfb_rt_process_process_sendBytesTimeout",
        ),
        (
            "_mfb_rt_process_process_poll",
            "_mfb_rt_process_process_pollFrom",
        ),
        (
            "_mfb_rt_process_process_receiveBytes",
            "_mfb_rt_process_process_receiveBytesFrom",
        ),
        (
            "_mfb_rt_process_process_receive",
            "_mfb_rt_process_process_receiveFrom",
        ),
    ] {
        if runtime_symbols.iter().any(|symbol| symbol == base)
            && !runtime_symbols.iter().any(|symbol| symbol == timed)
        {
            runtime_symbols.push(timed.to_string());
        }
    }
    // plan-76-A: the `poll(List OF RES Socket)` overload routes to `net.pollList`,
    // a synthesized target the NIR never names (it carries only `net.poll`), so
    // emit its helper body whenever `poll` is present — mirroring `connectTcpAddr`.
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_net_net_poll")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_net_net_pollList")
    {
        runtime_symbols.push("_mfb_rt_net_net_pollList".to_string());
    }
    // plan-76-C: `tls.pollList` is the synthesized list-multiplex target (the NIR
    // names only `tls.poll`), and its portable driver calls the scalar
    // `_mfb_rt_tls_tls_poll` — so emit both whenever `tls.poll` is present.
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_tls_tls_poll")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_tls_tls_pollList")
    {
        runtime_symbols.push("_mfb_rt_tls_tls_pollList".to_string());
    }
    // audio's overload-split runtime calls (named-device `open*`, timed `read`/`poll`,
    // per-direction `close`) are rewritten at IR level (`audio::runtime_overload_name`),
    // so the NIR already names each one and the required-helper scan collects them — no
    // code-layer synthesis push is needed (unlike net/process/tls).
    // App-mode io.input composes io.write (prompt -> transcript) + io.readLine
    // (read the window input pipe), so ensure both helpers are emitted
    // (plan-04-macos-app.md §5.4).
    if module.build_mode.is_app()
        && runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_io_io_input")
    {
        for dependency in ["_mfb_rt_io_io_write", "_mfb_rt_io_io_readLine"] {
            if !runtime_symbols.iter().any(|symbol| symbol == dependency) {
                runtime_symbols.push(dependency.to_string());
            }
        }
    }
}

/// `lower_module_for_platform`'s relocation-gated helpers: each shared helper (map hash, drops,
/// error park, concat, formatters, `construct.T`, `toString`) emitted iff a lowered function calls it.
pub(super) fn lower_relocation_gated_helpers(
    tables: &ClosureHelperTables<'_>,
    type_model: &TypeModel,
    constructor_types: &[(ParameterType, usize)],
    code_functions: &mut Vec<CodeFunction>,
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
    // Map hash index (plan-02 Phase 6): the probe lazily builds the buckets and is
    // the only caller of the build helper; the put helper backs the in-place `set`
    // insert. The probe/put are internal `bl` targets emitted during code lowering
    // (not IR-level runtime symbols), so gate on whether any lowered function
    // references them. Emit all three together (the probe calls build).
    let uses_map_hash = code_functions.iter().any(|function| {
        function.relocations.iter().any(|relocation| {
            relocation.to == MAP_PROBE_SYMBOL || relocation.to == MAP_BUCKET_PUT_SYMBOL
        })
    });
    if uses_map_hash {
        code_functions.push(lower_map_build_buckets_helper());
        code_functions.push(lower_map_bucket_put_helper());
        code_functions.push(lower_map_probe_helper());
    }
    // bug-732/733: the in-place removals' index maintenance and the map dead-byte
    // reclaim, each emitted only when a lowered function calls it.
    for (symbol, lower) in [
        (
            MAP_BUCKET_DELETE_SYMBOL,
            crate::codegen::collection::map::map_bucket_remove::lower_map_bucket_delete_helper
                as fn() -> CodeFunction,
        ),
        (
            MAP_COMPACT_FROM_SYMBOL,
            crate::codegen::collection::map::map_bucket_remove::lower_map_compact_from_helper,
        ),
        (
            MAP_RECLAIM_SYMBOL,
            crate::codegen::collection::map::map_bucket_remove::lower_map_reclaim_helper,
        ),
        // bug-760 B1: tombstone removal and the settle every other reader runs.
        (
            MAP_RETIRE_SYMBOL,
            crate::codegen::collection::map::map_bucket_remove::lower_map_retire_helper,
        ),
        (
            MAP_SETTLE_SYMBOL,
            crate::codegen::collection::map::map_bucket_remove::lower_map_settle_helper,
        ),
    ] {
        let used = code_functions.iter().any(|function| {
            function
                .relocations
                .iter()
                .any(|relocation| relocation.to == symbol)
        });
        if used {
            code_functions.push(lower());
        }
    }
    // plan-118-D: one `construct.T` per record type the census qualified, gated
    // on the relocation like every other internal `bl` target — a type can
    // qualify on site count and still have every site fold away.
    for (type_, arity) in constructor_types {
        let symbol = construct_symbol(type_);
        let used = code_functions.iter().any(|function| {
            function
                .relocations
                .iter()
                .any(|relocation| relocation.to == symbol)
        });
        if used {
            code_functions.push(lower_construct_helper(
                type_,
                *arity,
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
    // plan-118-E: the shared owned flat-collection scope drop. Same gate.
    let uses_drop_owned_collection = code_functions.iter().any(|function| {
        function
            .relocations
            .iter()
            .any(|relocation| relocation.to == DROP_OWNED_COLLECTION_SYMBOL)
    });
    if uses_drop_owned_collection {
        code_functions.push(lower_drop_owned_collection_helper(
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
    // plan-118-E: the shared owned-`String` scope drop. Same relocation gate.
    let uses_drop_owned_string = code_functions.iter().any(|function| {
        function
            .relocations
            .iter()
            .any(|relocation| relocation.to == DROP_OWNED_STRING_SYMBOL)
    });
    if uses_drop_owned_string {
        code_functions.push(lower_drop_owned_string_helper(
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
    // bug-560: the capacity-aware variant, emitted only when some function
    // actually drops a `String` binding that an in-place self-append grew. Same
    // relocation gate, so a module with no self-append is byte-identical.
    let uses_drop_owned_string_cap = code_functions.iter().any(|function| {
        function
            .relocations
            .iter()
            .any(|relocation| relocation.to == DROP_OWNED_STRING_CAP_SYMBOL)
    });
    if uses_drop_owned_string_cap {
        code_functions.push(lower_drop_owned_string_cap_helper(
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
    // plan-158-A: the status-message raise. Same relocation gate, and it must run
    // BEFORE the park's gate below: `_mfb_rt_fail_status` calls the park, and that
    // call is what makes a module whose only parker is this helper emit the park.
    let fail_status_callers: Vec<&str> = code_functions
        .iter()
        .filter(|function| {
            function
                .relocations
                .iter()
                .any(|relocation| relocation.to == FAIL_STATUS_SYMBOL)
        })
        .map(|function| function.symbol.as_str())
        .collect();
    if !fail_status_callers.is_empty() {
        // A helper outside `PARKING_HELPER_PACKAGES` would return a parked error to
        // call sites that carry no re-stamp branch: the error would lose its
        // location. Refuse the build instead.
        if let Some(caller) = fail_status_callers
            .iter()
            .find(|symbol| !runtime_helper_symbol_may_park_error(symbol))
        {
            return Err(format!(
                "`{caller}` raises through `{FAIL_STATUS_SYMBOL}`, but its package is \
                 not in PARKING_HELPER_PACKAGES, so its call sites do not re-stamp a \
                 parked error"
            ));
        }
        code_functions
            .push(crate::codegen::error::emission::fail_status::lower_fail_status_helper());
    }
    // plan-158-A: the call-site re-stamp of a parked helper error. Same gate, and
    // also ahead of the park's: it calls the park.
    let uses_restamp_parked_error = code_functions.iter().any(|function| {
        function
            .relocations
            .iter()
            .any(|relocation| relocation.to == RESTAMP_PARKED_ERROR_SYMBOL)
    });
    if uses_restamp_parked_error {
        code_functions.push(
            crate::codegen::error::emission::fail_status::lower_restamp_parked_error_helper(
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
    // plan-118-E: the shared error-block park, the single largest shape left in
    // the module after B/C/D. Same relocation gate.
    let uses_park_error = code_functions.iter().any(|function| {
        function
            .relocations
            .iter()
            .any(|relocation| relocation.to == PARK_ERROR_SYMBOL)
    });
    if uses_park_error {
        code_functions.push(lower_park_error_helper(
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
    // plan-118-C: the shared `String` concat. Same demand gate as the map
    // helpers — an internal `bl` target, emitted iff a lowered function
    // relocates against it.
    let uses_string_concat = code_functions.iter().any(|function| {
        function
            .relocations
            .iter()
            .any(|relocation| relocation.to == STRING_CONCAT_SYMBOL)
    });
    if uses_string_concat {
        code_functions.push(lower_string_concat_helper());
    }
    // The in-tree Float decimal formatter (float_format.rs): an internal `bl`
    // target emitted by `emit_float_to_string_value` call sites, so gate on the
    // relocations like the map-hash helpers.
    let uses_float_to_string = code_functions.iter().any(|function| {
        function
            .relocations
            .iter()
            .any(|relocation| relocation.to == FLOAT_TO_STRING_SYMBOL)
    });
    if uses_float_to_string {
        code_functions.push(lower_float_to_string_helper());
    }
    // plan-120-G: the significant-digit stream behind json's number rendering,
    // and the two limb helpers it calls. Same relocation gate.
    let uses_sci = code_functions.iter().any(|function| {
        function.relocations.iter().any(|relocation| {
            relocation.to
                == crate::codegen::string::format::float_format_sci::FLOAT_TO_STRING_SCI_SYMBOL
        })
    });
    if uses_sci {
        code_functions.extend(
            crate::codegen::string::format::float_format_sci::lower_float_to_string_sci_helpers(),
        );
    }
    // plan-120-F: the correctly-rounded String -> Float parser and the four
    // helpers it calls. Same relocation gate; the family is all-or-nothing
    // because the entry point is the only caller of the rest.
    let uses_string_to_float = code_functions.iter().any(|function| {
        function.relocations.iter().any(|relocation| {
            relocation.to == crate::codegen::string::format::float_parse::STRING_TO_FLOAT_SYMBOL
        })
    });
    if uses_string_to_float {
        code_functions
            .extend(crate::codegen::string::format::float_parse::lower_string_to_float_helpers());
    }
    // plan-118-C: the integer twin of the float formatter, same gate.
    let uses_int_to_string = code_functions.iter().any(|function| {
        function
            .relocations
            .iter()
            .any(|relocation| relocation.to == INT_TO_STRING_SYMBOL)
    });
    if uses_int_to_string {
        code_functions.push(lower_int_to_string_helper());
    }
    // plan-118-C: the synthesized `toString` renderers (Fixed/Money/byte-list/
    // Scalar). Same relocation gate; each is built by replaying the very emitter
    // the call site used to inline, so nothing can drift between them.
    for kind in ToStringHelper::every() {
        let used = code_functions.iter().any(|function| {
            function
                .relocations
                .iter()
                .any(|relocation| relocation.to == kind.symbol())
        });
        if used {
            code_functions.push(lower_to_string_helper(
                kind,
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

    Ok(())
}

/// `lower_module_for_platform`'s canvas and `LINK` support: `fs.pathJoin`, the graphics and Vulkan
/// builder trampolines, and the `LINK` thunks, initializer and data.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_canvas_and_link_support(
    module: &NirModule,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    layout: &ModuleLayout,
    type_model: &TypeModel,
    link_libraries: &link_locator::LinkLibraries,
    close_op_symbols: &HashSet<String>,
    code_functions: &mut Vec<CodeFunction>,
    data_objects: &mut Vec<CodeDataObject>,
) -> Result<(), String> {
    let ModuleLayout {
        module_teardown_symbol,
        ref global_initializer_symbol,
        globals_base,
        link_count,
        link_init_symbol,
        ..
    } = *layout;
    if module_uses_call(module, "fs.pathJoin") {
        code_functions
            .push(crate::codegen::builtins::fs::gen_path_builder::lower_fs_path_join_helper());
    }
    // plan-98-D Phase 2: the graphics thread's trampoline. Emitted for a program that
    // draws, alongside the shared-state block `canvas::startGraphics` spawns it from.
    if module_uses_call(module, "canvas.startGraphics") {
        code_functions.push(crate::codegen::runtime::canvas::emit_graphics_trampoline(
            platform_imports,
            platform,
            ArenaInitSymbols {
                link_init: link_init_symbol,
                global_init: global_initializer_symbol.as_deref(),
                teardown: module_teardown_symbol,
            },
        )?);
    }

    // bug-996: the Vulkan builder thread's entry, which `canvas::vulkanBuild` starts
    // and which calls it back. A target with no Vulkan backend starts no such thread.
    if module_uses_call(module, "canvas.vulkanBuild")
        && crate::codegen::runtime::canvas::vulkan::has_vulkan_backend(platform)
    {
        code_functions.push(
            crate::codegen::runtime::canvas::vulkan_handoff::emit_vulkan_builder_entry(platform),
        );
    }

    // Native `LINK` marshaling thunks + load-time initializer (plan-linker.md §12).
    if link_count > 0 {
        let support = link_thunk::emit_link_support(
            &module.link_functions,
            &module.link_cstructs,
            &type_model.record_fields,
            link_thunk::LinkCodegenOptions {
                globals_base,
                max_buffer_bytes: module.max_buffer_bytes,
            },
            platform_imports,
            platform,
            link_libraries,
            close_op_symbols,
        )?;
        code_functions.extend(support.functions);
        // The support set dedupes against itself only; a string it shares with
        // the module set (the resource guard's `ErrResourceClosed` /
        // `ErrResourceMoved`, which an `fs`/`thread` program already carries)
        // must be defined once.
        for object in support.data_objects {
            match data_objects
                .iter()
                .find(|existing| existing.symbol == object.symbol)
            {
                Some(existing) if existing.value == object.value => {}
                Some(_) => {
                    return Err(format!(
                        "LINK support data object '{}' conflicts with a module data object",
                        object.symbol
                    ))
                }
                None => data_objects.push(object),
            }
        }
        for (_, message, symbol) in native_link_error_messages() {
            if !data_objects.iter().any(|object| object.symbol == *symbol) {
                data_objects.push(string_data_object(symbol, message.to_string()));
            }
        }
    }

    Ok(())
}

/// `lower_module_for_platform`'s relocation-gated data objects: the math constant pool, the empty
/// string, the Unicode tables, error strings, status prefixes and the powers of ten — each emitted iff
/// a lowered function relocates against it.
pub(super) fn add_relocation_gated_data_objects(
    module: &NirModule,
    code_functions: &[CodeFunction],
    data_objects: &mut Vec<CodeDataObject>,
) -> Result<(), String> {
    // Math-kernel constant pool: emitted iff some kernel references it (plan-03
    // Phase 2). A read-only blob of the broadcast f64/i64 constants, each value
    // duplicated across both `.2d` lanes of a 16-byte slot.
    if code_functions.iter().any(|function| {
        function
            .relocations
            .iter()
            .any(|relocation| relocation.to == builder_simd_float_math::MATH_CONST_POOL_SYMBOL)
    }) {
        let words = builder_simd_float_math::math_const_pool_words();
        data_objects.push(CodeDataObject {
            symbol: builder_simd_float_math::MATH_CONST_POOL_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.math.const_pool.v1 { u128 lanes[] }".to_string(),
            align: 16,
            size: words.len() * 16,
            value: builder_simd_float_math::math_const_pool_data_value(),
        });
    }

    // A synthesized function (plan-118-C's `toString` renderers, plan-118-D's
    // `construct.T`) has no source file, so its allocation-failure path builds
    // an `ErrorLoc` with an empty filename and relocates against the
    // empty-string constant — which the module's own code may never have
    // required. Same force-emit the recursive-copy functions above need, and for
    // the same reason; without it the link dies on an undefined
    // `_mfb_str_empty`. Scanned over EVERY function rather than a suffix: the
    // synthesis sites are interleaved with the demand gates, and an earlier
    // version that only looked at the tail missed `construct.T` entirely.
    if code_functions.iter().any(|function| {
        function
            .relocations
            .iter()
            .any(|relocation| relocation.to == EMPTY_STRING_SYMBOL)
    }) && !data_objects
        .iter()
        .any(|object| object.symbol == EMPTY_STRING_SYMBOL)
    {
        data_objects.push(string_data_object(EMPTY_STRING_SYMBOL, String::new()));
    }

    // Unicode property/mapping tables (`strings::upper/lower/caseFold/
    // normalizeNfc/graphemes*` at runtime). Emit iff some generated function
    // actually relocates against a `_mfb_unicode_*` data symbol, unioned with the
    // legacy NIR heuristic so the emitted set can only grow relative to prior
    // builds (no golden churn). The relocation scan is the ground truth: the NIR
    // heuristic could deem a `caseFold(localVar)` static (folded, table skipped)
    // while codegen lowered it at runtime, leaving the `_mfb_unicode_casefold_*`
    // relocation undefined — a deterministic build failure this closes.
    let referenced_unicode_tables: std::collections::HashSet<&str> = code_functions
        .iter()
        .flat_map(|function| function.relocations.iter())
        .filter(|relocation| {
            relocation.binding == "data" && relocation.to.starts_with("_mfb_unicode_")
        })
        .map(|relocation| relocation.to.as_str())
        .collect();
    if !referenced_unicode_tables.is_empty() {
        // Per-symbol emission (plan-77 U5): emit exactly the tables some function
        // relocates against, so a program that only reaches part of the unicode
        // surface (e.g. `strings::graphemes` but no case mapping) stops carrying
        // the tables it never touches.
        data_objects.extend(unicode_runtime_data_objects(Some(
            &referenced_unicode_tables,
        )));
    } else if module_uses_unicode_runtime_tables(module) {
        // The NIR heuristic sees unicode use but no relocation names a specific
        // table (practically unreachable — the builder emits a relocation for
        // every table read). Fall back to the full set rather than risk an
        // undefined symbol.
        data_objects.extend(unicode_runtime_data_objects(None));
    }

    // bug-545: the fixed `_mfb_str_error_*` messages, emitted iff some generated
    // function actually relocates against one — the same ground-truth scan the
    // unicode tables above use, and for the same reason.
    //
    // The gates that register these run BEFORE codegen and predict the emission
    // instead of observing it: one fires on a planned `_mfb_rt_fs_*`/
    // `_mfb_rt_thread_*` runtime symbol, the other on a list of call names
    // (`thread.cancel`, `thread.send`, …). Neither describes "this module emits a
    // resource closed guard", so an aliasing `RES` rebind of a `tcp`/`udp`
    // socket — §15.6's documented shape, with no other call into the package —
    // emitted the guard, referenced `_mfb_str_error_resource_closed`, and failed
    // the build with a message about a compiler-internal symbol. `fs::File` was
    // fine only because its package drags in the whole standard set, and adding
    // any `tcp::` call "fixed" it for the same accidental reason. That is the
    // bug-256 class, patched twice before by adding one more name to the list;
    // this closes it by construction, because the emitter and the registrar stop
    // being two lists that have to be kept in step by hand.
    //
    // Additive only: a symbol is emitted here just when a function relocates
    // against it AND it is not already present. Every program that links today
    // already carries every string it references, so nothing is added to it and
    // its data section stays byte-identical — the delta is confined to the
    // programs that fail today.
    let referenced_error_strings: std::collections::HashSet<&str> = code_functions
        .iter()
        .flat_map(|function| function.relocations.iter())
        .filter(|relocation| {
            relocation.binding == "data" && relocation.to.starts_with("_mfb_str_error_")
        })
        .map(|relocation| relocation.to.as_str())
        .collect();
    if !referenced_error_strings.is_empty() {
        for (_, message, symbol) in standard_error_messages() {
            if referenced_error_strings.contains(symbol)
                && !data_objects.iter().any(|object| object.symbol == *symbol)
            {
                data_objects.push(string_data_object(symbol, message.to_string()));
            }
        }
    }

    // plan-158-A: the `"<message> (<call>: "` prefixes `_mfb_rt_fail_status`
    // formats from. Each symbol encodes its own text
    // (`error_status_prefix_symbol`), so the scan decodes it rather than looking it
    // up. A `BTreeSet` fixes the data-section order.
    let referenced_status_prefixes: std::collections::BTreeSet<&str> = code_functions
        .iter()
        .flat_map(|function| function.relocations.iter())
        .filter(|relocation| {
            relocation.binding == "data"
                && relocation.to.starts_with(ERROR_STATUS_PREFIX_SYMBOL_PREFIX)
        })
        .map(|relocation| relocation.to.as_str())
        .collect();
    for symbol in referenced_status_prefixes {
        let text = decode_error_status_prefix_symbol(symbol)
            .ok_or_else(|| format!("malformed error-status prefix symbol `{symbol}`"))?;
        data_objects.push(string_data_object(symbol, text));
    }

    // plan-120-F: the powers-of-ten table `_mfb_rt_string_to_float` indexes by a
    // runtime decimal exponent. Same relocation gate as the unicode tables, and
    // for the same reason — it is 10 KiB, and a program that never converts text
    // to a Float should not carry it.
    let uses_powers_of_ten = code_functions.iter().any(|function| {
        function.relocations.iter().any(|relocation| {
            relocation.to == crate::codegen::string::format::float_parse_table::POWERS_OF_TEN_SYMBOL
        })
    });
    if uses_powers_of_ten {
        data_objects.push(crate::codegen::memory::data::powers_of_ten_data_object());
    }

    Ok(())
}
