//! plan-176-U: the module lowering — `lower_module_for_platform`, its MIR variant and
//! `variant_demand`. The builder types and shared imports live in `engine/builder/mod.rs`.

use super::*;

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
    let mut string_objects = string_symbols.iter().collect::<Vec<_>>();
    string_objects.sort_by_key(|(_, left_symbol)| *left_symbol);
    let mut data_objects = string_objects
        .into_iter()
        .map(|(value, symbol)| string_data_object(symbol, value.clone()))
        .collect::<Vec<_>>();
    if module_requires_empty_string_constant(module) {
        data_objects.push(string_data_object(EMPTY_STRING_SYMBOL, String::new()));
    }
    // Writable global pointing at the main thread's arena state (set at startup,
    // read by `_mfb_shutdown` / the signal handler). Zero-initialized; it must
    // land in a writable section, which it does on every target that has an
    // entry: Linux entry programs are always dynamically linked (RW data
    // segment) and the macOS linker emits a writable `__DATA` segment.
    if module.entry.is_some() {
        data_objects.push(CodeDataObject {
            symbol: MAIN_ARENA_GLOBAL_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.main_arena.v1 { u64 arenaState }".to_string(),
            align: 8,
            size: 8,
            value: "0000000000000000".to_string(),
        });
        // bug-902: the path the POSIX entry (or app bootstrap) opens onto each
        // standard descriptor the process was started without.
        if platform.family() != crate::codegen::engine::types::PlatformFamily::Windows {
            data_objects.push(crate::codegen::engine::function::entry::dev_null_path_data_object());
        }
    }
    // plan-130-A: the `--debug` report's data (once-guard, key and line strings)
    // and each registered section's. Empty unless the report is on, so every
    // normal build keeps its exact data-object set.
    data_objects.extend(crate::codegen::debug::data_objects(module));
    // plan-32-A: the RVV runtime-detection flag byte (padded to 8), a writable
    // zeroed global the startup auxv scan sets to 1 on a V-capable chip and the
    // dual-path v128 lowering (plan-32-C) branches on. Emitted only for a
    // `linux-riscv64` entry module — the exact gate under which the auxv scan is
    // injected into the program entry — so every other target stays byte-identical.
    // plan-98-D Phase 2: the canvas scene region. A writable process-global block
    // rather than part of the arena state, because arena state is per-thread and the
    // graphics thread must see what the worker published — see the constant's own
    // documentation and `.ai/canvas-threading.md` §2. Emitted only for a module that
    // uses `canvas::`, so every other program keeps its exact data-object set.
    if module_uses_canvas(module) {
        data_objects.push(crate::codegen::runtime::canvas::graphics_state_data_object(
            module.canvas_auto_paint_ms,
        ));
        // A `--debug` build's GPU frame profile, which only its renderer writes and
        // only its stats line reads; a normal build has neither.
        if module.debug.enabled {
            data_objects.push(crate::codegen::runtime::canvas::profile_data_object());
        }
        data_objects.push(CodeDataObject {
            symbol: CANVAS_SCENE_SYMBOL.to_string(),
            kind: "raw".to_string(),
            // The annotation names every slot, `retiredHead` included — it was six
            // names against ten words, which read as if the retirement side were not
            // part of this object at all. bug-683 turned those four words into one.
            layout: "mfb.runtime.canvas_scene.v1 { u64 revision, count, items, hashes, \
                     layers, layerCount, retiredHead }"
                .to_string(),
            align: 8,
            size: CANVAS_SCENE_SLOTS * 8,
            value: "00".repeat(CANVAS_SCENE_SLOTS * 8),
        });
        // plan-98-G: the loaded-font table, process-global for the same reason — the
        // worker loads a font and the graphics thread rasterises from it.
        data_objects.push(CodeDataObject {
            symbol: CANVAS_FONTS_SYMBOL.to_string(),
            kind: "raw".to_string(),
            // The first chunk of the chain (bug-756); later chunks are arena blocks.
            layout: "mfb.runtime.canvas_fonts.v2 { { u64 handle, block }[16], u64 next }"
                .to_string(),
            align: 8,
            size: CANVAS_FONT_CHUNK_BYTES,
            value: "00".repeat(CANVAS_FONT_CHUNK_BYTES),
        });
        // plan-116-G: the named-group table, process-global for the same reason again —
        // `canvas::setGroup` runs on the worker and the renderer that draws a group
        // runs on the graphics thread. Fixed size because the graphics thread reads it
        // without a lock and a reallocating table would move under a reader.
        data_objects.push(CodeDataObject {
            symbol: CANVAS_GROUPS_SYMBOL.to_string(),
            kind: "raw".to_string(),
            // The slot count is interpolated rather than spelled: this string is the
            // only description of the table a reader of the `.ncode` gets, and a
            // hand-written `[256]` is one edit away from disagreeing with the array it
            // describes.
            layout: format!(
                "mfb.runtime.canvas_groups.v1 {{ u64 name, items, count, revision, \
                 refs, retiredFrame }}[{CANVAS_MAX_GROUPS}]; u64 ownedBytes"
            ),
            align: 8,
            size: CANVAS_GROUP_TABLE_BYTES,
            value: "00".repeat(CANVAS_GROUP_TABLE_BYTES),
        });
    }
    if module.entry.is_some() && module.target == "linux-riscv64" {
        data_objects.push(CodeDataObject {
            symbol: HAS_RVV_GLOBAL_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.has_rvv.v1 { u8 hasRvv }".to_string(),
            align: 8,
            size: 8,
            value: "0000000000000000".to_string(),
        });
    }
    // Writable `argc`/`argv` globals for `os::args()` and `os::prog()` (plan-128-A): filled by the
    // program entry from the values the OS passes in, read back when a later
    // `os::args()` builds its `List OF String`. Emitted only when the module uses
    // `os.args`, so existing programs' data layout is unchanged.
    if module_uses_any_call(module, &["os.args", "os.prog"]) {
        for symbol in [
            crate::codegen::builtins::os::OS_ARGC_GLOBAL_SYMBOL,
            crate::codegen::builtins::os::OS_ARGV_GLOBAL_SYMBOL,
        ] {
            data_objects.push(CodeDataObject {
                symbol: symbol.to_string(),
                kind: "raw".to_string(),
                layout: "mfb.runtime.os_args.v1 { u64 word }".to_string(),
                align: 8,
                size: 8,
                value: "0000000000000000".to_string(),
            });
        }
    }
    // Process-global mutex serializing `os::` env/pwd access against a concurrent
    // `os::setEnv`/`os::unsetEnv` from another MFBASIC thread (bug-64). Statically
    // initialized so no runtime initializer runs: Linux `PTHREAD_MUTEX_INITIALIZER`
    // is an all-zero `pthread_mutex_t`; macOS carries the `_PTHREAD_MUTEX_SIG_init`
    // signature in the first word so libc lazily first-use-initializes it. Writable
    // (the same section guarantee as the arena/argv globals above), and emitted only
    // when the module uses an env/pwd helper so existing programs' layout is
    // unchanged.
    if crate::codegen::builtins::os::module_uses_env_lock(module) {
        data_objects.push(CodeDataObject {
            symbol: crate::codegen::builtins::os::OS_ENV_LOCK_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.os_env_lock.v1 { u8 pthread_mutex[64] }".to_string(),
            align: 8,
            size: crate::codegen::builtins::os::OS_ENV_LOCK_SIZE,
            value: crate::codegen::builtins::os::os_env_lock_init_hex(platform.family()),
        });
    }
    // Process-global stdin broadcast log (plan-15): one zero-initialized writable
    // structure. Emitted whenever the module uses a stdin read builtin or
    // `thread::openStdIn`/`closeStdIn`, or polls the mouse (bug-1099); app mode
    // reads a window pipe, not fd 0.
    if !module.build_mode.is_app()
        && module_uses_any_call(module, crate::codegen::io::input::STDIN_READER_CALLS)
    {
        data_objects.push(stdin_log_data_object());
    }
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_fs_") || symbol.starts_with("_mfb_rt_thread_"))
    {
        for (_, message, symbol) in standard_error_messages() {
            if !data_objects.iter().any(|object| object.symbol == *symbol) {
                data_objects.push(string_data_object(symbol, (*message).to_string()));
            }
        }
    }
    // Unicode property/mapping tables are emitted below, after code generation,
    // driven by the relocations the generated functions actually reference (a
    // pre-codegen NIR heuristic and codegen disagreed on whether a
    // `caseFold(localVar)` folds, leaving an undefined `_mfb_unicode_*` reloc).
    // `term::` console helpers reference fixed ANSI escape-sequence byte strings
    // (plan-01-term.md §6.1).
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_term_"))
    {
        data_objects.extend(term::console_data_objects());
        // bug-699 Phase 5: `term::on`/`term::off` (and `io::readLine`'s line-mode
        // window) switch the kitty keyboard protocol for a console program whose
        // `io` readers decode keys — the same key that reserves the input region.
        if !module.build_mode.is_app()
            && crate::codegen::io::input::program_uses_input_buffer(&native_plan.runtime_symbols)
        {
            data_objects.extend(term::kitty_data_objects());
        }
    }
    // plan-94-B: the mouse-tracking escapes and the injection variable's name.
    // Gated on the mouse members rather than on `term::` at all, so a program that
    // only draws a box carries neither — the same reason the escape table above was
    // trimmed to two entries (bug-326-A21), and the same key that reserves the
    // mouse arena region.
    if native_plan.runtime_symbols.iter().any(|symbol| {
        symbol.ends_with("_enableMouse")
            || symbol.ends_with("_pollMouse")
            || symbol.ends_with("_pollMouseRaw")
    }) {
        data_objects.extend(term::mouse_data_objects());
    }
    // TLS helpers reference read-only C strings (library names + symbol names)
    // for their load-time dlopen/dlsym.
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_tls_"))
    {
        // Server-only strings (listener symbols, identity plumbing) are gated
        // so client-only programs keep their exact pre-server data section.
        let tls_server = native_plan
            .runtime_symbols
            .iter()
            .any(|symbol| is_tls_server_symbol(symbol));
        match platform.family() {
            PlatformFamily::MacOS => {
                data_objects.extend(
                    crate::codegen::builtins::tls::gen_shared::macos_tls_data_objects(tls_server),
                );
            }
            PlatformFamily::Linux => {
                data_objects.extend(
                    crate::codegen::builtins::tls::gen_shared::tls_cstring_data_objects(tls_server),
                );
            }
            // The Windows TLS data objects are the Schannel package-name wide
            // string (plan-47-J).
            PlatformFamily::Windows => {
                let _ = tls_server;
                data_objects.extend(crate::codegen::builtins::tls::gen_schannel::data_objects());
            }
        }
    }
    // The audio backend's read-only data objects (the Linux libasound soname +
    // ALSA symbol names; none on macOS). The backend owns the platform decision
    // and the symbol gate (bug-330).
    data_objects.extend(AudioBackend::select(platform).data_objects(&native_plan.runtime_symbols));
    // plan-98-F: the Vulkan loader's `dlopen`/`dlsym` C strings. Gated on the member
    // being in the plan rather than on the platform, so a canvas program that never
    // probes Vulkan carries none of them — the same shape the audio backend uses.
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| symbol.contains("canvas_vulkan"))
    {
        data_objects.extend(crate::codegen::runtime::canvas::vulkan::data_objects(
            platform,
        ));
    }
    // The clean-room `Certificate`-typed AbiFunction members (`crypto::generate`,
    // `crypto::sign`, `crypto::verify`) share one unified `_mfb_crypto_cert_*` read-only
    // data-object set (framework paths + dlsym names + PKCS#8/SPKI templates + CNG wide
    // ids), distinct from the EC helpers' `_mfb_crypto_ec_*`. Emit it once if any of
    // those members is in the plan.
    if native_plan.runtime_symbols.iter().any(|symbol| {
        symbol == "_mfb_rt_abi_crypto_generate"
            || symbol == "_mfb_rt_abi_crypto_sign"
            || symbol == "_mfb_rt_abi_crypto_verify"
    }) {
        data_objects.extend(crate::codegen::builtins::crypto::gen_cert::data_objects(
            platform.family(),
        ));
    }
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
    // bug-547: the main arena is NOT destroyed at exit when the program can still
    // have a live worker — on EVERY platform family. This is not a platform
    // preference, it is a use-after-free.
    //
    // `thread.start` used to `arena_alloc` BOTH the worker's thread control block and
    // the worker's entire arena-state block (arena state + the per-arena globals
    // region) out of the SPAWNING thread's arena, so a running worker's two pinned
    // registers — its arena register and `abi::CURRENT_THREAD` — both pointed INTO the
    // main arena's blocks. plan-156-A moved both to the process heap; whether any
    // other worker pointer into the main arena remains is plan-156-B's audit, and
    // until it proves none does, the skip stays. Dropping a
    // `Thread` handle only cancels and broadcasts (detached semantics, bug-205), so
    // a program that returns from `main` without `thread::waitFor` reaches
    // `_mfb_shutdown` with those blocks live; `_mfb_arena_destroy` then
    // `munmap`/`VirtualFree`s them and the still-running worker faults on its next
    // touch of its own arena state.
    //
    // Only Linux deferred. Windows was left `false` by 47-H pending "when Windows
    // threads (CreateThread) land" — they landed, and the deferral was never
    // revisited: box 2230 faulted `0xC0000005` in ~8% of runs of a program whose
    // worker merely spins (bug-547). macOS carried the identical latent UAF and was
    // only ever incidentally clean (its `exit` follows the `munmap` closely enough
    // that the worker rarely gets scheduled in between).
    //
    // Skipping the free costs nothing: the process is exiting, so the OS reclaims
    // every block either way — a bulk `arena_destroy` at exit is ceremony. And it
    // leaves NO window rather than narrowing one: nothing at all is released before
    // the platform process-exit, so a worker terminated at an arbitrary instruction
    // boundary (which is exactly what Windows `ExitProcess` does) cannot fault.
    // Observable semantics are unchanged — a detached worker still does not block
    // exit, which joining every worker here would have broken.
    //
    // The gate keeps it surgical: a program with no `thread.` runtime call can have
    // no worker outliving it, and still destroys the arena byte-identically.
    let skip_entry_arena_destroy = runtime_symbols.iter().any(|symbol| {
        runtime::spec_for_symbol(symbol)
            .map(|spec| spec.call.starts_with("thread."))
            .unwrap_or(false)
    });

    let global_initializer_symbol = if module.globals.is_empty() {
        None
    } else {
        Some(nir::function_symbol(&nir::global_initializer_name(
            &module.project,
        )))
    };

    // Native `LINK` bindings reserve one writable global slot per function (just
    // past the program's own globals) for their dlopen/dlsym-resolved pointers
    // (plan-linker.md §12.1).
    let globals_base = module.globals.len() + package_global_count + module_resource_slots;
    let link_count = module.link_functions.len();
    // Each `FREE` block resolves its deallocator into an additional writable slot,
    // reserved just past the per-function slots (17_native-libraries.md).
    let free_count = module
        .link_functions
        .iter()
        .filter(|function| function.free.is_some())
        .count();
    let link_slot_count = link_count + free_count;
    // `term::` keeps its TUI-mode state (the §4.2.1 `active` gate plus current
    // attributes) in writable slots reserved just past the program globals and
    // `LINK` slots in the program-entry frame (plan-01-term.md §6.2). The slots
    // are addressed off the pinned arena-state register `x19`, so every console
    // helper reaches the state at a fixed offset without per-module threading
    // beyond this byte offset. When the program never uses `term::`, no slots are
    // reserved.
    let uses_term = runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_term_"));
    // Whether the program reaches any socket helper. Windows needs
    // WSAStartup/WSACleanup in the entry only then (plan-47-I §3.2); every other
    // platform ignores the flag, so a socket-free program stays byte-identical.
    //
    // bug-460: this listed only `net` and `tls`, from when those were the only two
    // families that touched Winsock. plan-110-B/C moved the transports into `tcp`
    // and `udp`, which carry their OWN runtime families, so a tcp- or udp-only
    // program initialized nothing and every socket call failed with
    // WSANOTINITIALISED. It hid behind `net::lookup`: resolving a host first pulled
    // in a `_mfb_rt_net_` symbol and the gate fired for the rest of the program.
    // `tls` stays listed because its Schannel client opens its own raw Winsock
    // socket (getaddrinfo/socket/connect/send/recv).
    const WINSOCK_FAMILIES: [&str; 4] = [
        "_mfb_rt_net_",
        "_mfb_rt_tcp_",
        "_mfb_rt_udp_",
        "_mfb_rt_tls_",
    ];
    let uses_net = runtime_symbols
        .iter()
        .any(|symbol| WINSOCK_FAMILIES.iter().any(|f| symbol.starts_with(f)));
    // Whether the program uses the `math::` random generator. When it does we
    // emit the PCG64 helpers, seed each thread's arena, and draw a fresh
    // per-thread stream on spawn.
    let uses_rng = module_uses_call(module, "math.rand") || module_uses_call(module, "math.seed");
    // The auto-restore on exit (§6.5) and a program that only calls `term::on()`
    // both need the `off` helper emitted; ensure it is present whenever `term::`
    // is used.
    if uses_term && !runtime_symbols.iter().any(|s| s == "_mfb_rt_term_term_off") {
        runtime_symbols.push("_mfb_rt_term_term_off".to_string());
    }
    // plan-35-C: `term::off` presents the final frame by calling the `term::sync`
    // helper, and the shutdown teardown frees the grid, so the present helper must
    // exist whenever `term::` is used even if the program never calls `sync`.
    if uses_term
        && !runtime_symbols
            .iter()
            .any(|s| s == "_mfb_rt_term_term_sync")
    {
        runtime_symbols.push("_mfb_rt_term_term_sync".to_string());
    }
    let term_state_offset = if uses_term {
        Some(ENTRY_GLOBALS_OFFSET + (globals_base + link_slot_count) * 8)
    } else {
        None
    };
    let term_state_slots = if uses_term { TERM_STATE_SLOTS } else { 0 };
    // plan-62-B: the `app::` presentation-mode word sits one slot past the
    // `term::` state region, addressed off the same pinned arena-state register.
    // Reserved only when the program actually uses `app::` (the `uses_term` model,
    // NOT every app build) so an app binary that never touches `app::` keeps its
    // exact entry frame and goldens. `app::` is `--app`-gated, so this is
    // implicitly false in console builds.
    let uses_app = runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_app_"));
    let presentation_mode_offset = if uses_app {
        Some(ENTRY_GLOBALS_OFFSET + (globals_base + link_slot_count + term_state_slots) * 8)
    } else {
        None
    };
    let presentation_mode_slots = if uses_app { PRESENTATION_MODE_SLOTS } else { 0 };
    // plan-94-A §4.4a: the mouse-state region — the decoded-event ring, its cursors
    // and the stdin decoder's partial-sequence buffer. A THIRD conditionally-reserved
    // region, appended past the presentation-mode word.
    //
    // Appending is load-bearing, not tidiness: every program that does not use mouse
    // keeps `term_state_offset` and `presentation_mode_offset` byte-identical, so no
    // existing entry frame or golden moves. The rejected alternative — growing
    // `TERM_STATE_SLOTS` — would have shifted the presentation-mode word for every
    // program using both `term::` and `app::`, and would have given a canvas-only
    // program (which reserves no `term::` state at all) nowhere to put its ring.
    //
    // Keyed on the member symbols rather than on the package prefix, because
    // `_mfb_rt_term_` is already true for any `term::` program: a program that draws
    // a box and never touches the mouse must not pay for a ring. Matching the
    // suffix covers both packages' spellings (`_mfb_rt_term_term_pollMouse`,
    // `_mfb_rt_canvas_canvas_pollMouseRaw` -- plan-172-I: the canvas native is the raw
    // half of an MFBASIC `canvas::pollMouse`) without enumerating four symbols that
    // would then have to be kept in step with the registry.
    let uses_mouse = runtime_symbols.iter().any(|symbol| {
        symbol.ends_with("_enableMouse")
            || symbol.ends_with("_pollMouse")
            || symbol.ends_with("_pollMouseRaw")
    });
    let mouse_state_offset = if uses_mouse {
        Some(
            ENTRY_GLOBALS_OFFSET
                + (globals_base + link_slot_count + term_state_slots + presentation_mode_slots) * 8,
        )
    } else {
        None
    };
    let mouse_state_slots = if uses_mouse { MOUSE_STATE_SLOTS } else { 0 };
    // bug-699: the input-buffer region — the bytes a stdin subscriber has pulled
    // but no reader has consumed. A FOURTH conditionally-reserved region, appended
    // past the mouse-state region for the same reason that one was appended: every
    // program that does not read through the buffer keeps every existing offset,
    // entry frame and golden byte-identical. Keyed on the readers that decode
    // through it (`io::input::INPUT_BUFFER_READERS`), because only a reader that
    // looks ahead can leave bytes in it; the plain readers then drain it first.
    let uses_input_buffer = crate::codegen::io::input::program_uses_input_buffer(&runtime_symbols);
    let input_state_offset = if uses_input_buffer {
        Some(
            ENTRY_GLOBALS_OFFSET
                + (globals_base
                    + link_slot_count
                    + term_state_slots
                    + presentation_mode_slots
                    + mouse_state_slots)
                    * 8,
        )
    } else {
        None
    };
    #[cfg(test)]
    crate::codegen::io::input::RESERVED_INPUT_STATE_OFFSET
        .with(|slot| slot.set(input_state_offset));
    let input_state_slots = if uses_input_buffer {
        crate::codegen::io::input::INPUT_STATE_SLOTS
    } else {
        0
    };
    // plan-94-A §4.4b: the process-global mouse-mode word. Not part of any arena,
    // because an app backend's mouse handler runs on the UI thread and has no arena
    // state to read — see `MOUSE_MODE_SYMBOL`. Emitted only when the program uses
    // mouse, so every other program keeps its exact data-object set.
    if uses_mouse {
        data_objects.push(CodeDataObject {
            symbol: MOUSE_MODE_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.mouse_mode.v1 { u64 mode }".to_string(),
            align: 8,
            size: 8,
            value: "00".repeat(8),
        });
    }
    // bug-699: the process-global "the terminal reports key events" word the
    // console readers consult (`term::on` sets it while the kitty keyboard protocol
    // is on). Process-global because the terminal is; emitted with the input
    // buffer, the only thing that reads it.
    if uses_input_buffer && !module.build_mode.is_app() {
        data_objects.push(CodeDataObject {
            symbol: crate::codegen::builtins::io::gen_read_key_family::INPUT_EVENTS_SYMBOL
                .to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.input_events.v1 { u64 reporting }".to_string(),
            align: 8,
            size: 8,
            value: "00".repeat(8),
        });
    }
    // plan-98-D Phase 2: the `canvas::` retained scene is NOT in arena state. It was
    // (plan-98-B put it one region past the presentation-mode word), but arena state is
    // per-thread and the graphics thread has to see what the worker published, so it
    // moved to the process-global `CANVAS_SCENE_SYMBOL` block — see that constant and
    // `.ai/canvas-threading.md` §2. Nothing is reserved here any more, so a canvas
    // program's entry frame is the same size as any other app program's.
    // plan-62-B §3.3: the static initial presentation mode. A program that
    // references `app::setMode` anywhere starts windowless (`None`); one that
    // never does keeps the default terminal-in-a-window surface (`Console`). Keyed
    // on `setMode` specifically — a read-only `getMode` must not force `None`
    // startup. Scanned off the NIR module (like `uses_rng`), robust to the helper
    // symbol's `_mfb_rt_app_app_setMode` spelling.
    let initial_mode = if module_uses_call(module, "app.setMode") {
        PresentationMode::None
    } else {
        PresentationMode::Console
    };
    // Seed the slot to `None` at worker entry when that is the static default;
    // `Console` needs no store (the arena-state region zero-inits to `0` =
    // `Console`, verified by the default-mode runtime test). `None` implies
    // `setMode` is used, which implies `uses_app`, so the offset is always present
    // here. This is the single reader of `initial_mode` in plan-62-B.
    let seed_presentation_mode_offset = match initial_mode {
        PresentationMode::None => presentation_mode_offset,
        PresentationMode::Console => None,
    };
    // Every writable slot addressed off the pinned arena-state register: the
    // program's own globals, the `LINK`/`FREE` pointer slots, and the `term::`
    // state. The region is PER-ARENA, so a worker thread needs exactly as many
    // slots as the entry frame reserves — the worker's arena block is sized from
    // this same number in `lower_thread_start_helper` (bug-369). Before that, a
    // worker's arena block was only `ARENA_STATE_SIZE` bytes, so every global read
    // in a worker ran off the end of the block into neighbouring arena memory.
    let arena_global_slots = globals_base
        + link_slot_count
        + term_state_slots
        + presentation_mode_slots
        + mouse_state_slots
        + input_state_slots;
    let link_init_symbol = if link_count > 0 {
        Some(nir::LINK_INIT_SYMBOL)
    } else {
        None
    };
    // bug-78: one STATIC closure descriptor per function referenced as a no-capture
    // function value, so a `FunctionRef` loads its address instead of arena-
    // allocating a fresh `{code, env=0}` descriptor on every evaluation. Emit the
    // descriptors as zeroed data objects and a startup initializer that writes each
    // `code` word with `&func`; the entry runs it once before `main`.
    let closure_descriptor_func_symbols: Vec<String> = {
        let mut symbols: Vec<String> = module_analysis::collect_function_value_refs(module)
            .into_iter()
            .map(|(name, type_)| {
                data_objects::builtin_function_symbol_for_type(&name, &type_)
                    .or_else(|| function_symbols.get(&name).cloned())
                    .unwrap_or(name)
            })
            .collect();
        symbols.sort();
        symbols.dedup();
        symbols
    };
    let closure_init_symbol = if closure_descriptor_func_symbols.is_empty() {
        None
    } else {
        for func_symbol in &closure_descriptor_func_symbols {
            data_objects.push(CodeDataObject {
                symbol: closure_descriptor_symbol(func_symbol),
                kind: "raw".to_string(),
                layout: "mfb.runtime.closure_descriptor.v1 { u64 code; u64 env }".to_string(),
                align: 8,
                size: CLOSURE_OBJECT_SIZE,
                value: "0".repeat(2 * CLOSURE_OBJECT_SIZE),
            });
        }
        code_functions.push(lower_closure_descriptor_initializer(
            &closure_descriptor_func_symbols,
        ));
        Some(CLOSURE_DESC_INIT_SYMBOL)
    };
    // Install SIGINT/SIGTERM handlers for console programs only. App-mode builds
    // keep their window-driven finish path (the worker has no Ctrl-C semantics),
    // but still share `_mfb_shutdown` for their normal-exit cleanup. Windows has
    // no POSIX `signal()` (and this build links only kernel32, no CRT); Ctrl-C
    // handling there is `SetConsoleCtrlHandler`, deferred past the machine floor.
    // plan-130: the active `--debug` features; the program entry runs each one's
    // `emit_entry_start`. Empty for a normal build.
    let debug_features: Vec<&'static dyn crate::codegen::debug::DebugFeature> =
        crate::codegen::debug::active_features(module).collect();
    let register_signal_handlers = module.entry.is_some()
        && !module.build_mode.is_app()
        && platform.family() != PlatformFamily::Windows;
    if let Some(entry) = &module.entry {
        let language_entry_symbol = nir::function_symbol(&entry.name);
        // An arg-accepting entry appends the args region (argc/argv/list/data
        // length/saved count slots) ABOVE the globals; without the extra room
        // those slots overlapped the first global slots — or, with no globals,
        // spilled past the frame onto the OS argc/argv words at a raw Linux
        // ELF entry (the frame is carved from the initial stack).
        let entry_args_region = if entry.accepts_args {
            ENTRY_ARGS_REGION_SIZE
        } else {
            0
        };
        let entry_stack_size =
            align(ENTRY_STACK_SIZE + arena_global_slots * 8, 16) + entry_args_region;
        let entry_global_slots = arena_global_slots;
        let uses_window = crate::codegen::builtins::app::module_uses_app_window(module);
        if module.build_mode.is_app() {
            // App mode (plan-04-macos-app.md §6.6, plan-05-linux-app.md §6.1): the
            // standard program entry runs on a worker thread under the app program
            // symbol, while `_main` becomes the toolkit bootstrap (AppKit / GTK4)
            // that creates the window and spawns the worker.
            let app_spec = AppEntrySpec {
                // bug-743: the worker hands the entry argc/argv whenever the entry
                // reads them — to materialize `main`'s parameter OR to capture them
                // for `os::args`/`os::prog`. Keyed on the parameter alone, a
                // `FUNC main AS Integer` that calls `os::args` captured the argument
                // block's pointer as argc and a garbage argv (macOS: SIGSEGV).
                worker_passes_args: entry.accepts_args
                    || module_uses_any_call(module, &["os.args", "os.prog"]),
                uses_term,
                initial_mode,
                uses_canvas: module_uses_canvas(module)
                    || module_uses_call(module, "canvas.blitSurface"),
                uses_mouse,
                uses_window,
                debug_hooks: module.debug.enabled,
            };
            let app_entry = platform
                .emit_app_program_entry(&app_spec, &platform_imports)
                .ok_or_else(|| {
                    format!(
                        "native target '{}' does not support app mode codegen",
                        platform.target()
                    )
                })??;
            // The worker runs the standard program-entry logic under its own symbol.
            code_functions.push(platform.emit_program_entry(
                &ProgramEntrySpec {
                    entry_symbol: MACAPP_PROGRAM_SYMBOL,
                    language_entry_symbol: &language_entry_symbol,
                    language_entry_returns: &entry.returns,
                    language_entry_accepts_args: entry.accepts_args,
                    global_initializer_symbol: global_initializer_symbol.as_deref(),
                    link_init_symbol,
                    closure_init_symbol,
                    module_teardown_symbol,
                    entry_stack_size,
                    global_slot_count: entry_global_slots,
                    emit_cleanup_failure_audit: module_may_record_cleanup_failure(module),
                    seed_rng: uses_rng,
                    register_signal_handlers,
                    capture_args: module_uses_any_call(module, &["os.args", "os.prog"]),
                    // App mode reads the window input pipe, not fd 0 — no broadcast log.
                    subscribe_stdin: false,
                    // The toolkit bootstrap owns `_main`; this body is CALLED by
                    // the worker thread, whose stack has no kernel argv layout
                    // (bug-240).
                    entry_called_as_function: true,
                    needs_winsock: uses_net,
                    // plan-62-B: the worker owns the per-arena presentation-mode
                    // slot the program's `app::` calls read/write, so it seeds the
                    // `None` static default here.
                    seed_presentation_mode_offset,
                    debug_features: &debug_features,
                },
                &platform_imports,
            )?);
            code_functions.extend(app_entry);
            // plan-172-J: a canvas program's window has pointer handlers whether or
            // not it uses the mouse members -- they publish the pointer to the graphics
            // state -- so a platform is told both facts, and emits the strings its
            // handlers name for whichever handlers it installs.
            data_objects.extend(platform.app_mode_data_objects(
                &module.project,
                module.debug.enabled,
                uses_mouse,
                app_spec.uses_canvas,
            ));
            // plan-62-C Phase 2: the reconcile's selector/key data objects, for a
            // program that can change mode (static default `None`) — matching where
            // its reconcile helpers are emitted, so a Console-default program that
            // never draws is unchanged.
            //
            // plan-98-C Phase 3 added the second half of the condition. The frame-blit
            // helpers reference this same selector set and are emitted for any program
            // that *draws*, which is not the same population: `IMPORT canvas` injects
            // the package's always-helpers — `__canvas_presentSurface` among them —
            // so even a program that only calls `canvas::rgb` emits a call to
            // `canvas::blitSurface`. Without this the data objects the emitted helpers
            // name would be missing ("data relocation target
            // '_mfb_macapp_sel_delegate' is not a data object").
            if initial_mode == PresentationMode::None || app_spec.uses_canvas {
                data_objects.extend(platform.app_mode_reconcile_data_objects());
            }
            // The `app` window members' process-global state (title, its lock, the
            // default title, the fullscreen word) plus the backend's sync data —
            // only for a program that uses one, so no other app program moves.
            if uses_window {
                data_objects.extend(crate::codegen::builtins::app::app_window_data_objects(
                    platform.family(),
                    &platform.app_default_window_title(&module.project),
                ));
                data_objects.extend(platform.app_window_data_objects());
            }
        } else {
            code_functions.push(platform.emit_program_entry(
                &ProgramEntrySpec {
                    entry_symbol: "_main",
                    language_entry_symbol: &language_entry_symbol,
                    language_entry_returns: &entry.returns,
                    language_entry_accepts_args: entry.accepts_args,
                    global_initializer_symbol: global_initializer_symbol.as_deref(),
                    link_init_symbol,
                    closure_init_symbol,
                    module_teardown_symbol,
                    entry_stack_size,
                    global_slot_count: entry_global_slots,
                    emit_cleanup_failure_audit: module_may_record_cleanup_failure(module),
                    seed_rng: uses_rng,
                    register_signal_handlers,
                    capture_args: module_uses_any_call(module, &["os.args", "os.prog"]),
                    subscribe_stdin: module_uses_any_call(
                        module,
                        crate::codegen::io::input::STDIN_READER_CALLS,
                    ),
                    // `_main` IS the process entry here, so args arrive however
                    // the platform's raw entry delivers them.
                    entry_called_as_function: false,
                    needs_winsock: uses_net,
                    // Console builds have no `app::` (the package is `--app`-gated),
                    // so this is always `None` here and the entry is unchanged.
                    seed_presentation_mode_offset,
                    debug_features: &debug_features,
                },
                &platform_imports,
            )?);
        }
    }
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
        synthesized_constructor_types(module, &type_model)
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
                    &function_symbols,
                    &functions,
                    &package_return_types,
                    &platform_imports,
                    platform,
                    module.build_mode,
                    &globals,
                    &string_symbols,
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
            &functions,
            &type_model,
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
                &function_symbols,
                &functions,
                &package_return_types,
                &platform_imports,
                platform,
                module.build_mode,
                &globals,
                &string_symbols,
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
                &type_model,
                &functions,
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
                &function_symbols,
                &functions,
                &package_return_types,
                &platform_imports,
                platform,
                module.build_mode,
                &globals,
                &string_symbols,
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
            &function_symbols,
            &functions,
            &package_return_types,
            &platform_imports,
            platform,
            module.build_mode,
            &globals,
            &string_symbols,
            type_model.clone(),
        )?);
    }
    // A per-type runtime deep-copy entry point for every recursive type, so a
    // recursive value (e.g. `dom::Node`) can be transferred out of a worker arena
    // — `copy_value_to_current_arena` routes such a value's copy to a call to
    // `thread_copy_symbol(type)` rather than recursing over the type inline
    // (bug-391). plan-134-B: each is a shim into the one non-recursive walker,
    // `_mfb_rt_graph_copy`, passing its index in this set's order as the kind.
    let recursive_copy_types = recursive_transfer_types(&type_model);
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
                &function_symbols,
                &functions,
                &package_return_types,
                &platform_imports,
                platform,
                module.build_mode,
                &globals,
                &string_symbols,
                type_model.clone(),
            )?);
        }
        typed_kinds.push((name.clone(), type_));
    }
    // bug-781: each capturing `LAMBDA`'s environment routine, once. A `LAMBDA` created
    // in several functions (a cloned body) must have recorded one layout everywhere.
    let closure_tables = crate::codegen::memory::arena::closure_ops::ClosureHelperTables {
        function_symbols: &function_symbols,
        functions: &functions,
        package_return_types: &package_return_types,
        platform_imports: &platform_imports,
        platform,
        build_mode: module.build_mode,
        globals: &globals,
        string_symbols: &string_symbols,
    };
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
                    &closure_tables,
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
            &code_functions,
            &[GRAPH_DROP_SYMBOL, GRAPH_DROP_EDGES_SYMBOL],
        )
    };
    if !recursive_copy_types.is_empty() {
        code_functions.push(
            crate::codegen::memory::arena::graph_copy::lower_graph_copy_walker(
                &typed_kinds[..copy_kind_count],
                &function_symbols,
                &functions,
                &package_return_types,
                &platform_imports,
                platform,
                module.build_mode,
                &globals,
                &string_symbols,
                type_model.clone(),
            )?,
        );
        code_functions.push(
            crate::codegen::memory::arena::graph_copy::lower_graph_stack_grow(
                &function_symbols,
                &functions,
                &package_return_types,
                &platform_imports,
                platform,
                module.build_mode,
                &globals,
                &string_symbols,
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
                &function_symbols,
                &functions,
                &package_return_types,
                &platform_imports,
                platform,
                module.build_mode,
                &globals,
                &string_symbols,
                type_model.clone(),
            )?);
        }
    }
    // bug-781: a function value's copy and drop, for a module that copies or drops one.
    if references(&code_functions, &[CLOSURE_COPY_SYMBOL, CLOSURE_DROP_SYMBOL]) {
        code_functions.push(
            crate::codegen::memory::arena::closure_ops::lower_closure_copy(
                &closure_tables,
                type_model.clone(),
            )?,
        );
        code_functions.push(
            crate::codegen::memory::arena::closure_ops::lower_closure_drop(
                &closure_tables,
                type_model.clone(),
            )?,
        );
    }
    // plan-130-B: the arena hot path times itself exactly when the `--debug` perf
    // section is active for this module.
    let perf_arena =
        crate::codegen::debug::feature_active(module, crate::codegen::debug::PERF_SECTION);
    // plan-130-C: the allocator counts its events exactly when the `--debug` arena
    // section is active for this module.
    let debug_arena =
        crate::codegen::debug::feature_active(module, crate::codegen::debug::ARENA_SECTION);
    code_functions.push(lower_arena_alloc(platform, perf_arena, debug_arena)?);
    code_functions.push(lower_build_error_loc());
    code_functions.push(lower_make_error_result());
    code_functions.push(lower_simd_alloc_list());
    code_functions.push(lower_arena_insert_free(debug_arena));
    code_functions.push(lower_arena_flush_coalesce(debug_arena));
    code_functions.push(lower_arena_free(perf_arena, debug_arena));
    // The arena's zero fill is always on: it scrubs freed chunks and fills fresh blocks.
    code_functions.push(lower_arena_fill_zero());
    code_functions.push(lower_arena_destroy(platform, debug_arena)?);
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
        code_functions.push(lower_stdout_drain(&platform_imports, platform)?);
    }
    // Stdin broadcast log (plan-15): the shared reader/subscription helpers are
    // emitted whenever the module uses a stdin read builtin, polls the mouse
    // (bug-1099), or uses `thread::openStdIn`/`closeStdIn`.
    // `_mfb_rt_stdin_next_byte` replaces the per-byte `read(0,…)` in
    // every stdin read site; subscribe/unsubscribe/recompute back the broadcast
    // registry and the compat main-thread subscription. App mode reads a window
    // pipe, not fd 0, so it is excluded.
    let uses_stdin = !module.build_mode.is_app()
        && (crate::codegen::io::input::program_reads_mouse(&runtime_symbols)
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
        code_functions.push(lower_stdin_recompute_base(&platform_imports, platform)?);
        code_functions.push(lower_stdin_next_byte(
            &platform_imports,
            platform,
            module.stdin_log_cap,
        )?);
        code_functions.push(lower_stdin_subscribe(&platform_imports, platform)?);
        code_functions.push(lower_stdin_unsubscribe(&platform_imports, platform)?);
    }
    // bug-699: the input decoder, for the readers that decode through the input
    // buffer. Reserved and emitted together, so a program without such a reader
    // carries neither.
    if input_state_offset.is_some() {
        code_functions.push(crate::codegen::io::input::decode::lower_input_decode(
            crate::codegen::io::input::program_uses_grapheme_clusters(&runtime_symbols),
        )?);
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
                &platform_imports,
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
                &platform_imports,
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
                &platform_imports,
                platform,
            )?,
        );
    }
    if module.entry.is_some() {
        code_functions.push(lower_shutdown(
            uses_term,
            skip_entry_arena_destroy,
            uses_stdout_buffer,
            module_uses_call(module, "canvas.startGraphics"),
            &platform_imports,
            platform,
            crate::codegen::debug::report_enabled(module),
        )?);
        // plan-130-A: `_mfb_debug_shutdown` and every report section it calls.
        code_functions.extend(crate::codegen::debug::code_functions(
            module,
            &platform_imports,
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
        &platform_imports,
        platform,
        &runtime_symbols,
    )?);
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
    // bug-806: the module instance's teardown, lowered once every function has —
    // it closes only the resource kinds some store listed. A built-in close it
    // calls that no binding of the program named is emitted with the other
    // runtime helpers below.
    if module_teardown_symbol.is_some() {
        let drain = crate::codegen::resource::module_owner::lower_module_resource_drain(
            &function_symbols,
            &functions,
            &package_return_types,
            &platform_imports,
            platform,
            module.build_mode,
            &globals,
            &string_symbols,
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
    for symbol in &runtime_symbols {
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
                    },
                    uses_rng,
                    &type_model,
                    &platform_imports,
                    platform,
                    // plan-98-C: the real string table, not an empty one — an
                    // `abi_function` body with a string literal needs its data object.
                    &string_symbols,
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
            &platform_imports,
            platform,
        )?);
    }
    if uses_thread_start {
        code_functions.push(platform.emit_thread_trampoline(
            &platform_imports,
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
            &platform_imports,
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
        code_functions.push(lower_thread_table_acquire(&platform_imports, platform)?);
        code_functions.push(lower_thread_table_release(&platform_imports, platform)?);
        code_functions.push(lower_thread_block_arena_create(
            debug_arena,
            &platform_imports,
            platform,
        )?);
        code_functions.push(lower_thread_block_arena_destroy(
            debug_arena,
            &platform_imports,
            platform,
        )?);
        code_functions.push(lower_thread_block_release(
            debug_arena,
            &platform_imports,
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
                &function_symbols,
                &functions,
                &package_return_types,
                &platform_imports,
                platform,
                module.build_mode,
                &globals,
                &string_symbols,
                type_model.clone(),
            )?,
        );
    }
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
    for (type_, arity) in &constructor_types {
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
                &function_symbols,
                &functions,
                &package_return_types,
                &platform_imports,
                platform,
                module.build_mode,
                &globals,
                &string_symbols,
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
            &function_symbols,
            &functions,
            &package_return_types,
            &platform_imports,
            platform,
            module.build_mode,
            &globals,
            &string_symbols,
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
            &function_symbols,
            &functions,
            &package_return_types,
            &platform_imports,
            platform,
            module.build_mode,
            &globals,
            &string_symbols,
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
            &function_symbols,
            &functions,
            &package_return_types,
            &platform_imports,
            platform,
            module.build_mode,
            &globals,
            &string_symbols,
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
                &function_symbols,
                &functions,
                &package_return_types,
                &platform_imports,
                platform,
                module.build_mode,
                &globals,
                &string_symbols,
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
            &function_symbols,
            &functions,
            &package_return_types,
            &platform_imports,
            platform,
            module.build_mode,
            &globals,
            &string_symbols,
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
                &function_symbols,
                &functions,
                &package_return_types,
                &platform_imports,
                platform,
                module.build_mode,
                &globals,
                &string_symbols,
                type_model.clone(),
            )?);
        }
    }

    if module_uses_call(module, "fs.pathJoin") {
        code_functions
            .push(crate::codegen::builtins::fs::gen_path_builder::lower_fs_path_join_helper());
    }
    // plan-98-D Phase 2: the graphics thread's trampoline. Emitted for a program that
    // draws, alongside the shared-state block `canvas::startGraphics` spawns it from.
    if module_uses_call(module, "canvas.startGraphics") {
        code_functions.push(crate::codegen::runtime::canvas::emit_graphics_trampoline(
            &platform_imports,
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
            &platform_imports,
            platform,
            &link_libraries,
            &close_op_symbols,
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
