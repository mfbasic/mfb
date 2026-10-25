//! plan-176-U: `lower_module_for_platform`'s data objects, arena layout and program entry.
//! Called in order by `lower_module_for_platform` (`engine/builder/lower_module.rs`).

use super::*;

/// `lower_module_for_platform`'s arena layout, built once by [`plan_arena_layout`] and read by
/// the regions after it.
pub(super) struct ModuleLayout {
    pub(super) module_teardown_symbol: Option<&'static str>,
    pub(super) skip_entry_arena_destroy: bool,
    pub(super) global_initializer_symbol: Option<String>,
    pub(super) globals_base: usize,
    pub(super) link_count: usize,
    pub(super) uses_term: bool,
    pub(super) uses_net: bool,
    pub(super) uses_rng: bool,
    pub(super) uses_mouse: bool,
    pub(super) term_state_offset: Option<usize>,
    pub(super) presentation_mode_offset: Option<usize>,
    pub(super) mouse_state_offset: Option<usize>,
    pub(super) input_state_offset: Option<usize>,
    pub(super) initial_mode: PresentationMode,
    pub(super) seed_presentation_mode_offset: Option<usize>,
    pub(super) arena_global_slots: usize,
    pub(super) link_init_symbol: Option<&'static str>,
    pub(super) closure_init_symbol: Option<&'static str>,
    pub(super) register_signal_handlers: bool,
}

/// `lower_module_for_platform`'s data objects: the string table and every fixed data object the
/// plan's runtime symbols ask for.
pub(super) fn module_data_objects(
    module: &NirModule,
    native_plan: &NativePlan,
    platform: &dyn CodegenPlatform,
    string_symbols: &HashMap<String, String>,
) -> Vec<CodeDataObject> {
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
    // plan-183-A: the AArch64 `dc zva` block size the entry probes and every
    // `arena_fill_zero` reads. Every AArch64 module links the fill, so every one
    // carries the word; a module without an entry leaves it 0 (the generic loop).
    if platform.arch() == "aarch64" {
        data_objects.push(CodeDataObject {
            symbol: DCZVA_BYTES_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.dczva_bytes.v1 { u64 blockBytes }".to_string(),
            align: 8,
            size: 8,
            value: "0000000000000000".to_string(),
        });
    }
    // plan-183-B: the arena reservation's words — every module's `arena_alloc` reads
    // the slice size.
    data_objects.extend(crate::codegen::runtime::arena_cap::data_objects_reservation(module));
    // plan-130-A: the `--debug` report's data (once-guard, key and line strings)
    // and each registered section's. Empty unless the report is on, so every
    // normal build keeps its exact data-object set.
    data_objects.extend(crate::codegen::debug::data_objects(module));
    // plan-179-B: the arena memory cap's limit word and startup-check strings.
    // Empty with `maxArenaMemoryDisable: true`.
    data_objects.extend(crate::codegen::runtime::arena_cap::data_objects(
        module, platform,
    ));
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
    data_objects
}

/// `lower_module_for_platform`'s arena layout: the per-arena slot regions (globals, `LINK`, `term`,
/// presentation mode, mouse, input buffer), the closure descriptors and the signal-handler decision.
#[allow(clippy::too_many_arguments)]
pub(super) fn plan_arena_layout(
    module: &NirModule,
    platform: &dyn CodegenPlatform,
    function_symbols: &HashMap<String, String>,
    package_global_count: usize,
    module_resource_slots: usize,
    module_teardown_symbol: Option<&'static str>,
    runtime_symbols: &mut Vec<String>,
    data_objects: &mut Vec<CodeDataObject>,
    code_functions: &mut Vec<CodeFunction>,
) -> ModuleLayout {
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
    // The auto-restore on exit (§6.5) calls the `term::off` helper, which presents
    // the final frame through `term::sync` (plan-35-C): both are in
    // `runtime_symbols` whenever `term::` is used, because the native plan adds
    // them for any `_mfb_rt_term_` symbol (`plan::symbols::runtime_symbols`).
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
    let uses_input_buffer = crate::codegen::io::input::program_uses_input_buffer(&*runtime_symbols);
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
    let register_signal_handlers = module.entry.is_some()
        && !module.build_mode.is_app()
        && platform.family() != PlatformFamily::Windows;
    ModuleLayout {
        module_teardown_symbol,
        skip_entry_arena_destroy,
        global_initializer_symbol,
        globals_base,
        link_count,
        uses_term,
        uses_net,
        uses_rng,
        uses_mouse,
        term_state_offset,
        presentation_mode_offset,
        mouse_state_offset,
        input_state_offset,
        initial_mode,
        seed_presentation_mode_offset,
        arena_global_slots,
        link_init_symbol,
        closure_init_symbol,
        register_signal_handlers,
    }
}

/// `lower_module_for_platform`'s program entry: the console `_main`, or the app worker entry and
/// its toolkit bootstrap.
pub(super) fn emit_program_entries(
    module: &NirModule,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    layout: &ModuleLayout,
    code_functions: &mut Vec<CodeFunction>,
    data_objects: &mut Vec<CodeDataObject>,
) -> Result<(), String> {
    let ModuleLayout {
        module_teardown_symbol,
        ref global_initializer_symbol,
        uses_term,
        uses_net,
        uses_rng,
        uses_mouse,
        initial_mode,
        seed_presentation_mode_offset,
        arena_global_slots,
        link_init_symbol,
        closure_init_symbol,
        register_signal_handlers,
        ..
    } = *layout;
    // plan-130: the active `--debug` features; the program entry runs each one's
    // `emit_entry_start`. Empty for a normal build.
    let debug_features: Vec<&'static dyn crate::codegen::debug::DebugFeature> =
        crate::codegen::debug::active_features(module).collect();
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
                .emit_app_program_entry(&app_spec, platform_imports)
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
                    arena_cap_init: crate::codegen::runtime::arena_cap::arena_cap_enabled(module),
                },
                platform_imports,
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
                    arena_cap_init: crate::codegen::runtime::arena_cap::arena_cap_enabled(module),
                },
                platform_imports,
            )?);
        }
    }
    Ok(())
}
