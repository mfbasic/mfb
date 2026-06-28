use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::arch::aarch64::{abi, ops::CodeOp};
use crate::binary_repr::{self};
use crate::builtins;
use crate::json_string;
use crate::numeric;

use super::nir::{
    self, NirFunction, NirMatchPattern, NirModule, NirOp, NirRecordUpdate, NirSourceLoc, NirValue,
};
use super::plan::NativePlan;
use super::runtime;


struct CodeBuilder<'a> {
    current_symbol: String,
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, String>,
    platform_imports: &'a HashMap<String, String>,
    globals: &'a HashMap<String, GlobalValue>,
    type_model: TypeModel,
    string_symbols: &'a HashMap<String, String>,
    locals: HashMap<String, LocalValue>,
    instructions: Vec<CodeInstruction>,
    relocations: Vec<CodeRelocation>,
    stack_slots: Vec<CodeStackSlot>,
    used_callee_saved: Vec<String>,
    stack_size: usize,
    next_register: usize,
    next_label: usize,
    trap: Option<TrapState>,
    loop_stack: Vec<LoopLabels>,
    active_cleanups: Vec<ActiveCleanup>,
    /// One entry per open `lower_ops` scope, holding the `active_cleanups` length
    /// at that scope's entry. Index 0 is the function body (the scope the trap
    /// handler shares). Used to compute which cleanups an error routed to a trap
    /// must run: only those belonging to inner blocks being exited, never the
    /// function-level locals that stay live (and visible) in the trap body.
    cleanup_scope_starts: Vec<usize>,
    pending_result_slots: Option<PendingResultSlots>,
    error_arena_restore_slot: Option<usize>,
    /// When set, an inline built-in error return (`emit_error_register_return`)
    /// branches to this label instead of returning, leaving the raw `Result` in
    /// the standard tag/value/message registers. Used to make inline conversions
    /// (`toInt`, …) trappable: an inline `TRAP` materializes the raw `Result`
    /// rather than auto-propagating.
    raw_result_capture: Option<String>,
    /// Project-relative source file of the function currently being lowered. Used
    /// to build `ErrorLoc.filename` for errors that originate in this function.
    current_file: String,
    /// Source location of the error-originating node currently being lowered.
    /// Updated as `lower_value` descends into call/arithmetic nodes and consulted
    /// when an error is freshly created (overflow, divide-by-zero, helper failure)
    /// so its `ErrorLoc` records the true origin.
    current_loc: NirSourceLoc,
    /// Resource ownership decisions (escape analysis, §15.6) for the function
    /// being lowered, keyed by `RES` binding name. Drives where each resource's
    /// close obligation lives (its own scope, an outer collection's owned-list,
    /// or out via a returned collection).
    resource_owners: HashMap<String, crate::escape::ResOwner>,
    /// Collection binding names that own a runtime owned-list (some resource
    /// floats up to their scope).
    owner_collections: HashSet<String>,
    /// Live owned-lists: collection binding name -> head-pointer stack slot.
    owned_list_heads: HashMap<String, usize>,
    /// Stack slots of owned freeable-flat locals, recorded as each is bound.
    /// In a function with a trap handler, an error can jump to the handler past
    /// a not-yet-run `LET`, so the handler's scope-drop would free a slot whose
    /// initializer never executed. Zeroing these slots in the prologue makes the
    /// handler's null-guarded frees skip such slots (the per-`LET` zero-init only
    /// guards a binding's *own* trapping initializer, not an earlier jump past it).
    owned_value_slots: Vec<usize>,
    /// Names of locals whose live buffer is currently being walked by an
    /// enclosing `FOR EACH` (the iterable was a plain `Local`). In-place `set`/
    /// `prepend`/map-`set` that overwrites an *existing* entry's payload would be
    /// observable to such an iterator (it reads each element from the snapshotted
    /// buffer on every step, `lower_for_each`), unlike append which only writes
    /// beyond the snapshot count. The rebuild path repoints the binding at a fresh
    /// buffer the iterator never sees, so in-place overwrite is excluded while the
    /// binding is an active `FOR EACH` iterable (plan-02 §4.1, D1). A `String`
    /// local can never be a `FOR EACH` iterable, so string self-append is exempt.
    for_each_iterable_locals: Vec<String>,
    /// `String` local name → stack slot tracking the spare payload capacity (bytes
    /// available past the current length) of the local's grown in-place self-append
    /// buffer (plan-02 §4.1 / D9). Zero means "tight" (no spare). The slot is a
    /// frame-local shadow that never escapes: any copy/return/transfer reads only
    /// `len` bytes, freezing the value to the canonical tight `[len][bytes][NUL]`
    /// form, so the spare is invisible outside the buffer. Reset to 0 on any
    /// non-self-append bind/assign of the local and zeroed at function entry.
    string_capacity_slots: HashMap<String, usize>,
}

#[derive(Clone)]
struct LocalValue {
    type_: String,
    stack_offset: usize,
    constant: Option<NirValue>,
    /// A *reference* local: the stack slot holds a pointer to another binding's
    /// slot rather than the value/block itself. Set for a non-escaping `MUT`
    /// borrow capture, where reads and writes deref through the slot pointer so
    /// the live parent binding is observed and updated. False for every ordinary
    /// binding.
    by_ref: bool,
}

#[derive(Clone)]
struct GlobalValue {
    type_: String,
    offset: usize,
}

#[derive(Clone)]
struct ValueResult {
    type_: String,
    location: String,
    text: String,
}

struct TrapState {
    name: String,
    label: String,
    in_trap_body: bool,
}

#[derive(Clone)]
struct LoopLabels {
    kind: crate::ast::LoopKind,
    continue_label: String,
    exit_label: String,
    cleanup_depth: usize,
}

#[derive(Clone)]
struct ThreadCleanup {
    name: String,
    symbol: String,
}

#[derive(Clone)]
struct ResourceCleanup {
    name: String,
    symbol: String,
}

#[derive(Clone)]
struct ResourceUnionCleanup {
    name: String,
    /// `(tag, close_symbol)` per active variant; drop reads the union tag and
    /// calls the matching close op on the variant's resource pointer.
    variants: Vec<(usize, String)>,
}

/// A per-scope runtime owned-list (§15.6): the close obligations for resources
/// whose ownership floated up to this scope from inner blocks. The list is a
/// nul-terminated singly linked list of `{record_ptr, next}` nodes, with its
/// head in `head_slot`; draining walks it head-first (most-recent first) and
/// closes each record once (the close is closed-flag idempotent).
#[derive(Clone)]
struct OwnedListCleanup {
    /// The owning collection binding's name (for transfer-on-return lookup).
    name: String,
    /// Stack offset of the list head pointer (0 when empty).
    head_slot: usize,
    /// Close op symbol for the collection's resource element type.
    close_symbol: String,
}

/// An owned, non-escaping flat value freed at scope-drop (plan-01 Phase 5 /
/// plan-02 Phase 8). Because every non-resource value is a pointer-free flat
/// block and copy-insertion (`lower_value_owned`) makes each owner's block
/// unaliased, a single `arena_free(ptr, size)` reclaims the whole value; the
/// size is recomputed from the static type at drop via
/// `emit_inlined_block_size_from_ptr_slot`.
#[derive(Clone)]
struct OwnedValueCleanup {
    /// Static type of the bound value (drives the runtime size computation).
    type_: String,
    /// Stack offset of the binding's slot (holds the block pointer).
    stack_offset: usize,
}

#[derive(Clone)]
enum ActiveCleanup {
    Thread(ThreadCleanup),
    Resource(ResourceCleanup),
    ResourceUnion(ResourceUnionCleanup),
    OwnedList(OwnedListCleanup),
    OwnedValue(OwnedValueCleanup),
}

#[derive(Clone, Copy)]
struct PendingResultSlots {
    value: usize,
    tag: usize,
    message: usize,
    // Stashed error-source pointer (`ErrorLoc`) for the pending error result.
    source: usize,
}

#[derive(Clone, Copy)]
enum ExitDestination {
    Return,
    Trap,
}

#[derive(Clone)]
struct PayloadSlot {
    slot: usize,
    type_: String,
}

#[derive(Clone)]
struct CollectionValueSlot {
    key: Option<PayloadSlot>,
    value: PayloadSlot,
}

struct CollectionTypeLayout {
    kind: usize,
    key_type_code: usize,
    value_type_code: usize,
}

#[derive(Clone)]
struct TypeModel {
    enum_members: HashMap<(String, String), usize>,
    record_fields: HashMap<String, Vec<(String, String)>>,
    union_names: HashSet<String>,
    union_variants: HashMap<String, String>,
    union_variant_unions: HashMap<String, HashSet<String>>,
    union_variant_tags: HashMap<String, usize>,
    union_variant_fields: HashMap<String, Vec<(String, String)>>,
}

pub(crate) fn lower_module_for_platform(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    platform: &dyn CodegenPlatform,
) -> Result<NativeCodePlan, String> {
    if module.target != platform.target() {
        return Err(format!(
            "native code platform '{}' cannot lower module target '{}'",
            platform.target(),
            module.target
        ));
    }
    // Imported packages are now decoded and merged into the project IR upstream
    // (see `lower::lower_project`) and lowered as ordinary functions through this
    // same codegen. The legacy flat binary_repr -> native package bridge is no
    // longer used: there are no separate package exports to lower here.
    let _ = packages;
    let mut function_symbols = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), nir::function_symbol(&function.name)))
        .collect::<HashMap<_, _>>();
    for import in &module.imports {
        function_symbols.insert(import.name.clone(), import.symbol.clone());
    }
    let globals = module
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
    let mut package_return_types: HashMap<String, String> = HashMap::new();
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
    let string_symbols = string_symbols(module);
    let mut string_objects = string_symbols.iter().collect::<Vec<_>>();
    string_objects.sort_by(|(_, left_symbol), (_, right_symbol)| left_symbol.cmp(right_symbol));
    let mut data_objects = string_objects
        .into_iter()
        .map(|(value, symbol)| CodeDataObject {
            symbol: symbol.clone(),
            kind: "constant".to_string(),
            layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }".to_string(),
            align: 8,
            size: align(8 + value.len() + 1, 8),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    if module_requires_empty_string_constant(module) {
        data_objects.push(CodeDataObject {
            symbol: EMPTY_STRING_SYMBOL.to_string(),
            kind: "constant".to_string(),
            layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }".to_string(),
            align: 8,
            size: 16,
            value: String::new(),
        });
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
    }
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_fs_") || symbol.starts_with("_mfb_rt_thread_"))
    {
        for (_, message, symbol) in standard_error_messages() {
            if !data_objects.iter().any(|object| object.symbol == *symbol) {
                data_objects.push(CodeDataObject {
                    symbol: (*symbol).to_string(),
                    kind: "constant".to_string(),
                    layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }"
                        .to_string(),
                    align: 8,
                    size: align(8 + message.len() + 1, 8),
                    value: (*message).to_string(),
                });
            }
        }
    }
    if module_uses_unicode_runtime_tables(module) {
        data_objects.extend(unicode_runtime_data_objects());
    }
    // `term::` console helpers reference fixed ANSI escape-sequence byte strings
    // (plan-01-term.md §6.1).
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_term_"))
    {
        data_objects.extend(term::console_data_objects());
    }
    // TLS helpers reference read-only C strings (library names + symbol names)
    // for their load-time dlopen/dlsym.
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_tls_"))
    {
        if platform.target().contains("macos") {
            data_objects.extend(tls::macos_tls_data_objects());
        } else {
            data_objects.extend(tls::tls_cstring_data_objects());
        }
    }
    let type_model = TypeModel::from_module_and_packages(module, packages)?;
    let mut code_functions = Vec::new();
    let mut runtime_symbols = native_plan.runtime_symbols.clone();
    let skip_entry_arena_destroy = platform.target() == "linux-aarch64"
        && runtime_symbols.iter().any(|symbol| {
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
    let globals_base = module.globals.len() + package_global_count;
    let link_count = module.link_functions.len();
    // Each `FREE` block resolves its deallocator into an additional writable slot,
    // reserved just past the per-function slots (mfbasic.md §17).
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
    let term_state_offset = if uses_term {
        Some(ENTRY_GLOBALS_OFFSET + (globals_base + link_slot_count) * 8)
    } else {
        None
    };
    let term_state_slots = if uses_term { TERM_STATE_SLOTS } else { 0 };
    let link_init_symbol = if link_count > 0 {
        Some(nir::LINK_INIT_SYMBOL)
    } else {
        None
    };
    // Install SIGINT/SIGTERM handlers for console programs only. App-mode builds
    // keep their window-driven finish path (the worker has no Ctrl-C semantics),
    // but still share `_mfb_shutdown` for their normal-exit cleanup.
    let register_signal_handlers = module.entry.is_some() && !module.build_mode.is_app();
    if let Some(entry) = &module.entry {
        let language_entry_symbol = nir::function_symbol(&entry.name);
        let entry_stack_size = align(
            ENTRY_STACK_SIZE + (globals_base + link_slot_count + term_state_slots) * 8,
            16,
        );
        let entry_global_slots = globals_base + link_slot_count + term_state_slots;
        if module.build_mode.is_app() {
            // App mode (plan-04-macos-app.md §6.6, plan-05-linux-app.md §6.1): the
            // standard program entry runs on a worker thread under the app program
            // symbol, while `_main` becomes the toolkit bootstrap (AppKit / GTK4)
            // that creates the window and spawns the worker.
            let app_spec = AppEntrySpec {
                language_entry_accepts_args: entry.accepts_args,
                uses_term,
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
            code_functions.push(lower_program_entry(
                MACAPP_PROGRAM_SYMBOL,
                &language_entry_symbol,
                &entry.returns,
                entry.accepts_args,
                global_initializer_symbol.as_deref(),
                link_init_symbol,
                entry_stack_size,
                entry_global_slots,
                &platform_imports,
                platform,
                module_may_record_cleanup_failure(module),
                uses_rng,
                register_signal_handlers,
            )?);
            code_functions.extend(app_entry);
            data_objects.extend(platform.app_mode_data_objects());
        } else {
            code_functions.push(lower_program_entry(
                "_main",
                &language_entry_symbol,
                &entry.returns,
                entry.accepts_args,
                global_initializer_symbol.as_deref(),
                link_init_symbol,
                entry_stack_size,
                entry_global_slots,
                &platform_imports,
                platform,
                module_may_record_cleanup_failure(module),
                uses_rng,
                register_signal_handlers,
            )?);
        }
    }
    for function in &module.functions {
        code_functions.push(lower_function(
            function,
            &function_symbols,
            &functions,
            &package_return_types,
            &platform_imports,
            &globals,
            &string_symbols,
            type_model.clone(),
        )?);
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
            &globals,
            &string_symbols,
            type_model.clone(),
        )?);
    }
    code_functions.push(lower_arena_alloc(platform)?);
    code_functions.push(lower_simd_alloc_list());
    code_functions.push(lower_arena_insert_free());
    code_functions.push(lower_arena_free());
    // Entropy fill is always on (plan-01 §6.5): scrub freed chunks and poison
    // fresh blocks. The fill RNG/seed helpers ship with every arena.
    code_functions.push(lower_arena_fill_random());
    code_functions.push(lower_arena_fill_seed());
    code_functions.push(lower_arena_fill_next());
    code_functions.push(lower_arena_destroy(platform)?);
    if module.entry.is_some() {
        code_functions.push(lower_shutdown(uses_term, skip_entry_arena_destroy));
    }
    if register_signal_handlers {
        code_functions.push(lower_signal_handler(platform)?);
    }
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
    for symbol in &runtime_symbols {
        code_functions.push(lower_runtime_helper(
            symbol,
            module.build_mode,
            term_state_offset,
            uses_rng,
            &platform_imports,
            platform,
        )?);
    }
    if uses_rng {
        code_functions.push(lower_rng_next());
        code_functions.push(lower_rng_seed_at());
    }
    let link_returns_cstring = module.link_functions.iter().any(|function| {
        function.abi_return_name == "return"
            && function.abi_return_ctype == "CPtr"
            && function.return_type == "String"
    });
    if link_returns_cstring
        || runtime_symbols.iter().any(|symbol| {
            matches!(
                symbol.as_str(),
                "_mfb_rt_fs_fs_readText"
                    | "_mfb_rt_fs_fs_readAll"
                    | "_mfb_rt_fs_fs_readLine"
                    | "_mfb_rt_net_net_readText"
                    | "_mfb_rt_net_net_receiveTextFrom"
                    | "_mfb_rt_tls_tls_readText"
            )
        })
    {
        code_functions.push(lower_validate_utf8_helper());
    }
    // The macOS TLS backend bridges Network.framework's async callbacks to a
    // semaphore via small emitted block-invoke functions.
    if platform.target().contains("macos")
        && runtime_symbols
            .iter()
            .any(|symbol| symbol.starts_with("_mfb_rt_tls_"))
    {
        code_functions.extend(tls::macos_tls_aux_functions());
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_fs_fs_listDirectory")
    {
        code_functions.push(lower_sort_string_list_helper());
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
    if module_uses_call(module, "fs.pathJoin") {
        code_functions.push(lower_fs_path_join_helper(platform));
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_thread_thread_start")
    {
        code_functions.push(lower_thread_trampoline(&platform_imports, platform)?);
    }

    // Native `LINK` marshaling thunks + load-time initializer (plan-linker.md §12).
    if link_count > 0 {
        let support = link_thunk::emit_link_support(
            &module.link_functions,
            globals_base,
            &platform_imports,
            platform,
        )?;
        code_functions.extend(support.functions);
        data_objects.extend(support.data_objects);
        for (_, message, symbol) in native_link_error_messages() {
            if !data_objects.iter().any(|object| object.symbol == *symbol) {
                data_objects.push(CodeDataObject {
                    symbol: symbol.to_string(),
                    kind: "constant".to_string(),
                    layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }"
                        .to_string(),
                    align: 8,
                    size: align(8 + message.len() + 1, 8),
                    value: message.to_string(),
                });
            }
        }
    }

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
    plan.validate()?;
    Ok(plan)
}

fn expanded_nir_union_variants<'a>(
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

#[allow(clippy::too_many_arguments)]
fn lower_program_entry(
    entry_symbol: &str,
    language_entry_symbol: &str,
    language_entry_returns: &str,
    language_entry_accepts_args: bool,
    global_initializer_symbol: Option<&str>,
    link_init_symbol: Option<&str>,
    entry_stack_size: usize,
    global_slot_count: usize,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    emit_cleanup_failure_audit: bool,
    seed_rng: bool,
    register_signal_handlers: bool,
) -> Result<CodeFunction, String> {
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(entry_stack_size),
        abi::add_immediate(ARENA_STATE_REGISTER, abi::stack_pointer(), 0),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 0),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 8),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 16),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 24),
        // The main arena-state lives on the entry stack (not zero-filled), so the
        // free-list head must be explicitly cleared before the first allocation.
        abi::store_u64("x31", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
    ];
    if emit_cleanup_failure_audit {
        instructions.extend([
            abi::store_u64(
                "x31",
                ARENA_STATE_REGISTER,
                ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
            ),
            abi::store_u64(
                "x31",
                ARENA_STATE_REGISTER,
                ARENA_CLEANUP_FAILURE_CODE_OFFSET,
            ),
            abi::store_u64(
                "x31",
                ARENA_STATE_REGISTER,
                ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
            ),
        ]);
    }
    for index in 0..global_slot_count {
        instructions.push(abi::store_u64(
            "x31",
            ARENA_STATE_REGISTER,
            ENTRY_GLOBALS_OFFSET + index * 8,
        ));
    }
    let mut relocations = Vec::new();
    let error_label = "entry_error";
    let exit_label = "entry_exit";
    // Publish this thread's arena-state address to the writable global so the
    // signal handler and `_mfb_shutdown` can find the arena without `x19`. `x9`
    // is a scratch temporary here; `x0`/`x1` (argc/argv) are left untouched.
    push_symbol_address(
        entry_symbol,
        MAIN_ARENA_GLOBAL_SYMBOL,
        "x9",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::store_u64(ARENA_STATE_REGISTER, "x9", 0));
    // Install SIGINT/SIGTERM handlers (console programs). `signal()` clobbers
    // `x0`/`x1`, so argc/argv are parked below the frame across the calls; `x19`
    // pins the entry frame, so temporarily lowering `sp` is safe.
    if register_signal_handlers {
        instructions.extend([
            abi::subtract_stack(16),
            abi::store_u64("x0", abi::stack_pointer(), 0),
            abi::store_u64("x1", abi::stack_pointer(), 8),
        ]);
        for signo in ["2", "15"] {
            instructions.push(abi::move_immediate("x0", "Integer", signo));
            push_symbol_address(
                entry_symbol,
                SIGNAL_HANDLER_SYMBOL,
                "x1",
                &mut instructions,
                &mut relocations,
            );
            platform.emit_libc_call(
                "signal",
                entry_symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        }
        instructions.extend([
            abi::load_u64("x0", abi::stack_pointer(), 0),
            abi::load_u64("x1", abi::stack_pointer(), 8),
            abi::add_stack(16),
        ]);
    }
    // Seed this thread's PCG64 generator from the OS entropy pool before any
    // user code (including global initializers, which may call `math::rand`).
    // The seed scratch lives in the as-yet-unused args slot; pre-fill it with
    // the arena address so a `getentropy` failure still yields a varying seed.
    if seed_rng {
        instructions.extend([
            abi::store_u64(
                ARENA_STATE_REGISTER,
                abi::stack_pointer(),
                ENTRY_ARGC_OFFSET,
            ),
            abi::add_immediate(
                abi::return_register(),
                abi::stack_pointer(),
                ENTRY_ARGC_OFFSET,
            ),
            abi::move_immediate("x1", "Integer", "8"),
        ]);
        platform.emit_random_bytes(
            entry_symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64("x1", abi::stack_pointer(), ENTRY_ARGC_OFFSET),
            abi::move_register(abi::return_register(), ARENA_STATE_REGISTER),
            abi::branch_link(RNG_SEED_SYMBOL),
        ]);
        relocations.push(internal_branch(entry_symbol, RNG_SEED_SYMBOL));
    }
    // Capture the arena start time (offset 40) and seed the dedicated memory-fill
    // RNG (offsets 16/24). Always on — entropy fill is a requirement (plan-01 §6),
    // so this runs for every program before the first allocation. The seed is OS
    // entropy XORed with the arena address and start time, so a `getentropy`
    // failure or two arenas seeding in the same instant still yield distinct
    // poison streams. This is a separate stream from `math::rand` (offsets 88/96),
    // so it never perturbs the reproducible language RNG.
    //
    // `argc`/`argv` (x0/x1) are still live here for arg-accepting entries (saved
    // to the stack further below), and this block clobbers x0–x16, so park them
    // in callee-saved x27/x28 — preserved by the libc calls and the fill helpers
    // — and restore them afterward. A local 16-byte stack buffer holds first the
    // `timespec` and then the entropy bytes, so no entry-stack slot is touched.
    instructions.extend([
        abi::move_register("x27", "x0"),
        abi::move_register("x28", "x1"),
        abi::subtract_stack(16),
        abi::move_immediate("x0", "Integer", "0"), // CLOCK_REALTIME
        abi::add_immediate("x1", abi::stack_pointer(), 0),
    ]);
    platform.emit_libc_call(
        "clock_gettime",
        entry_symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), 0),  // tv_sec
        abi::load_u64("x10", abi::stack_pointer(), 8), // tv_nsec
        abi::move_immediate("x11", "Integer", "1000000000"),
        abi::multiply_registers("x9", "x9", "x11"),
        abi::add_registers("x9", "x9", "x10"), // ns = sec*1e9 + nsec
        abi::store_u64("x9", ARENA_STATE_REGISTER, ARENA_START_TIME_OFFSET),
        // Pre-fill the seed scratch with the arena address (getentropy fallback).
        abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), 0),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), 0),
        abi::move_immediate("x1", "Integer", "8"),
    ]);
    platform.emit_random_bytes(
        entry_symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x1", abi::stack_pointer(), 0), // entropy (or arena addr)
        abi::add_stack(16),
        abi::load_u64("x9", ARENA_STATE_REGISTER, ARENA_START_TIME_OFFSET),
        abi::exclusive_or_registers("x1", "x1", "x9"), // mix start time
        abi::exclusive_or_registers("x1", "x1", ARENA_STATE_REGISTER), // mix arena address
        abi::move_register(abi::return_register(), ARENA_STATE_REGISTER),
        abi::branch_link(ARENA_FILL_SEED_SYMBOL),
        // Restore argc/argv for the arg-materialization path below.
        abi::move_register("x0", "x27"),
        abi::move_register("x1", "x28"),
    ]);
    relocations.push(internal_branch(entry_symbol, ARENA_FILL_SEED_SYMBOL));
    // Resolve native `LINK` bindings (dlopen/dlsym) before anything runs; a load
    // failure aborts before `main` through the standard error path
    // (plan-linker.md §12.1).
    if let Some(symbol) = link_init_symbol {
        instructions.push(abi::branch_link(symbol));
        relocations.push(CodeRelocation {
            from: entry_symbol.to_string(),
            to: symbol.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
            abi::branch_ne(error_label),
        ]);
    }
    if let Some(symbol) = global_initializer_symbol {
        instructions.push(abi::branch_link(symbol));
        relocations.push(CodeRelocation {
            from: entry_symbol.to_string(),
            to: symbol.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_PROGRAM_EXIT_TAG),
            abi::branch_ne("global_initializer_not_program_exit"),
            abi::move_register(abi::return_register(), RESULT_VALUE_REGISTER),
            abi::branch(exit_label),
            abi::label("global_initializer_not_program_exit"),
        ]);
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
            abi::branch_ne(error_label),
        ]);
    }
    if language_entry_accepts_args {
        instructions.extend([
            abi::store_u64("x0", abi::stack_pointer(), ENTRY_ARGC_OFFSET),
            abi::store_u64("x1", abi::stack_pointer(), ENTRY_ARGV_OFFSET),
        ]);
        emit_entry_args_list_materialization(error_label, &mut instructions, &mut relocations);
        instructions.push(abi::load_u64(
            "x0",
            abi::stack_pointer(),
            ENTRY_ARGS_LIST_OFFSET,
        ));
    }
    instructions.push(abi::branch_link(language_entry_symbol));
    relocations.push(CodeRelocation {
        from: entry_symbol.to_string(),
        to: language_entry_symbol.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_PROGRAM_EXIT_TAG),
        abi::branch_ne("entry_not_program_exit"),
        abi::move_register(abi::return_register(), RESULT_VALUE_REGISTER),
        abi::branch(exit_label),
        abi::label("entry_not_program_exit"),
    ]);
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(error_label),
    ]);
    if language_entry_returns == "Nothing" {
        instructions.push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    } else {
        instructions.push(abi::move_register(
            abi::return_register(),
            RESULT_VALUE_REGISTER,
        ));
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "255"),
            abi::branch_hi("entry_exit_range_error"),
        ]);
    }
    instructions.push(abi::branch(exit_label));
    if language_entry_returns == "Integer" {
        instructions.extend([
            abi::label("entry_exit_range_error"),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OVERFLOW_CODE),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        ]);
        push_error_message_address(
            entry_symbol,
            ERR_OVERFLOW_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::branch(error_label));
    }
    instructions.extend([
        abi::label(error_label),
        abi::store_u64(RESULT_VALUE_REGISTER, ARENA_STATE_REGISTER, 32),
        abi::move_register("x20", RESULT_ERROR_MESSAGE_REGISTER),
    ]);
    emit_write_string_object(
        ENTRY_ERROR_PREFIX_SYMBOL,
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_integer_to_stderr(
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_string_object(
        ENTRY_ERROR_SEPARATOR_SYMBOL,
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::string_length_register(), "x20", 0),
        abi::add_immediate(abi::string_data_register(), "x20", 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(
        entry_symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_string_object(
        ENTRY_ERROR_NEWLINE_SYMBOL,
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    if emit_cleanup_failure_audit {
        emit_cleanup_failure_audit_report(
            entry_symbol,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        "255",
    ));
    instructions.push(abi::label(exit_label));
    // Run the shared teardown (terminal restore + arena free), then exit. The
    // exit code is parked in the arena-state scratch slot across the call: that
    // slot lives in this stack-resident entry frame (not in the freed mmap
    // blocks), and `_mfb_shutdown` preserves `x19`, so it is valid on return.
    // `_mfb_shutdown` is internally gated and idempotent, so the SIGINT/SIGTERM
    // handler racing this path cannot double-free.
    instructions.push(abi::store_u64(
        abi::return_register(),
        ARENA_STATE_REGISTER,
        32,
    ));
    instructions.push(abi::branch_link(SHUTDOWN_SYMBOL));
    relocations.push(internal_branch(entry_symbol, SHUTDOWN_SYMBOL));
    instructions.push(abi::load_u64(
        abi::return_register(),
        ARENA_STATE_REGISTER,
        32,
    ));
    platform.emit_program_exit(entry_symbol, &mut instructions, &mut relocations)?;
    Ok(CodeFunction {
        name: if entry_symbol == "_main" {
            "program.entry".to_string()
        } else {
            "program.entry.macapp".to_string()
        },
        symbol: entry_symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

fn emit_entry_args_list_materialization(
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), ENTRY_ARGC_OFFSET),
        abi::load_u64("x21", abi::stack_pointer(), ENTRY_ARGV_OFFSET),
        abi::move_immediate("x22", "Integer", "0"),
        abi::move_immediate("x23", "Integer", "0"),
        abi::label("entry_args_count_loop"),
        abi::compare_registers("x23", "x20"),
        abi::branch_eq("entry_args_count_done"),
        abi::load_u64("x24", "x21", 0),
        abi::move_register("x25", "x24"),
        abi::move_immediate("x26", "Integer", "0"),
        abi::label("entry_args_count_len_loop"),
        abi::load_u8("x27", "x25", 0),
        abi::compare_immediate("x27", "0"),
        abi::branch_eq("entry_args_count_len_done"),
        abi::add_immediate("x26", "x26", 1),
        abi::add_immediate("x25", "x25", 1),
        abi::branch("entry_args_count_len_loop"),
        abi::label("entry_args_count_len_done"),
        abi::add_registers("x22", "x22", "x26"),
        abi::add_immediate("x21", "x21", 8),
        abi::add_immediate("x23", "x23", 1),
        abi::branch("entry_args_count_loop"),
        abi::label("entry_args_count_done"),
        abi::move_immediate("x24", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x25", "x20", "x24"),
        abi::add_registers("x25", "x25", "x22"),
        abi::store_u64("x22", abi::stack_pointer(), ENTRY_ARGS_DATA_LENGTH_OFFSET),
        abi::store_u64("x20", abi::stack_pointer(), ENTRY_ARGS_COUNT_SAVED_OFFSET),
        abi::add_immediate(abi::return_register(), "x25", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: "_main".to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("entry_args_alloc_ok"),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address("_main", ERR_ALLOCATION_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(error_label),
        abi::label("entry_args_alloc_ok"),
        abi::store_u64("x1", abi::stack_pointer(), ENTRY_ARGS_LIST_OFFSET),
        abi::load_u64("x22", abi::stack_pointer(), ENTRY_ARGS_DATA_LENGTH_OFFSET),
        abi::load_u64("x20", abi::stack_pointer(), ENTRY_ARGS_COUNT_SAVED_OFFSET),
        abi::move_immediate("x8", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_KIND),
        abi::move_immediate("x8", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("x8", "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x8", "Byte", "1"),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u64("x20", "x1", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x20", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("x22", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x22", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("x23", "x1", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x24", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x25", "x20", "x24"),
        abi::add_registers("x24", "x23", "x25"),
        abi::move_immediate("x25", "Integer", "0"),
        abi::load_u64("x21", abi::stack_pointer(), ENTRY_ARGV_OFFSET),
        abi::move_immediate("x26", "Integer", "0"),
        abi::label("entry_args_fill_loop"),
        abi::compare_registers("x26", "x20"),
        abi::branch_eq("entry_args_fill_done"),
        abi::load_u64("x27", "x21", 0),
        abi::move_register("x28", "x27"),
        abi::move_immediate("x9", "Integer", "0"),
        abi::label("entry_args_fill_len_loop"),
        abi::load_u8("x10", "x28", 0),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq("entry_args_fill_len_done"),
        abi::add_immediate("x9", "x9", 1),
        abi::add_immediate("x28", "x28", 1),
        abi::branch("entry_args_fill_len_loop"),
        abi::label("entry_args_fill_len_done"),
        abi::move_immediate("x11", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("x11", "x23", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x31", "x23", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64("x31", "x23", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64("x25", "x23", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::store_u64("x9", "x23", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::move_immediate("x11", "Integer", "0"),
        abi::label("entry_args_copy_loop"),
        abi::compare_registers("x11", "x9"),
        abi::branch_eq("entry_args_copy_done"),
        abi::load_u8("x12", "x27", 0),
        abi::store_u8("x12", "x24", 0),
        abi::add_immediate("x27", "x27", 1),
        abi::add_immediate("x24", "x24", 1),
        abi::add_immediate("x11", "x11", 1),
        abi::branch("entry_args_copy_loop"),
        abi::label("entry_args_copy_done"),
        abi::add_registers("x25", "x25", "x9"),
        abi::add_immediate("x23", "x23", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x21", "x21", 8),
        abi::add_immediate("x26", "x26", 1),
        abi::branch("entry_args_fill_loop"),
        abi::label("entry_args_fill_done"),
    ]);
}

fn emit_cleanup_failure_audit_report(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let done = "entry_cleanup_failure_audit_done";
    instructions.extend([
        abi::load_u64(
            "x9",
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
        ),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(done),
    ]);
    emit_write_string_object(
        CLEANUP_FAILURE_PREFIX_SYMBOL,
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.push(abi::load_u64(
        "x9",
        ARENA_STATE_REGISTER,
        ARENA_CLEANUP_FAILURE_CODE_OFFSET,
    ));
    instructions.push(abi::store_u64("x9", ARENA_STATE_REGISTER, 32));
    emit_write_integer_to_stderr_with_labels(
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
        "entry_cleanup_failure_code",
    )?;
    emit_write_string_object(
        CLEANUP_FAILURE_SEPARATOR_SYMBOL,
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64(
            "x20",
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
        ),
        abi::load_u64(abi::string_length_register(), "x20", 0),
        abi::add_immediate(abi::string_data_register(), "x20", 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)?;
    emit_write_string_object(
        ENTRY_ERROR_NEWLINE_SYMBOL,
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(done));
    Ok(())
}

fn lower_arena_alloc(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    // Grow-path frame: the fast (first-fit) path makes no call, but the rare
    // block-grow path calls `arena_fill_random` to poison the new block, so the
    // function carries a frame and saves the link register. The fast path never
    // touches x11–x13/x17, and the grow path saves/restores x11–x13 around the
    // fill call, so the historical clobber contract (x9, x10, x14, x15, x20–x28)
    // is preserved for callers.
    const FRAME_SIZE: usize = 64;
    const LR_SLOT: usize = 0;
    const UBASE_SLOT: usize = 8;
    const USIZE_SLOT: usize = 16;
    const X11_SLOT: usize = 24;
    const X12_SLOT: usize = 32;
    const X13_SLOT: usize = 40;
    let not_15 = (!(ARENA_MIN_CHUNK - 1)).to_string();
    let mut relocations = Vec::new();
    let mut instructions = Vec::new();
    // --- Validate alignment and normalize the request --------------------------
    // x20 = normalized size (rounded up to the 16-byte granule), x21 = effective
    // alignment (raised to ≥16 so every chunk start stays 16-aligned).
    instructions.extend([
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::compare_immediate("x1", "0"),
        abi::branch_eq("arena_alloc_invalid"),
        abi::subtract_immediate("x9", "x1", 1),
        abi::and_registers("x10", "x1", "x9"),
        abi::compare_immediate("x10", "0"),
        abi::branch_ne("arena_alloc_invalid"),
        // eff align = max(align, 16)
        abi::move_register("x21", "x1"),
        abi::compare_immediate("x21", &ARENA_MIN_CHUNK.to_string()),
        abi::branch_lo("arena_alloc_align_min"),
        abi::branch("arena_alloc_align_ready"),
        abi::label("arena_alloc_align_min"),
        abi::move_immediate("x21", "Integer", &ARENA_MIN_CHUNK.to_string()),
        abi::label("arena_alloc_align_ready"),
        // normalized size = round_up(max(size, 1), 16)
        abi::move_register("x20", "x0"),
        abi::compare_immediate("x20", "0"),
        abi::branch_ne("arena_alloc_size_nonzero"),
        abi::move_immediate("x20", "Integer", "1"),
        abi::label("arena_alloc_size_nonzero"),
        abi::add_immediate("x20", "x20", (ARENA_MIN_CHUNK - 1) as usize),
        abi::move_immediate("x9", "Integer", &not_15),
        abi::and_registers("x20", "x20", "x9"),
        // --- First-fit walk over the address-ordered free-list -----------------
        abi::label("arena_alloc_walk"),
        abi::load_u64("x22", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::move_immediate("x23", "Integer", "0"),
        abi::label("arena_alloc_walk_loop"),
        abi::compare_immediate("x22", "0"),
        abi::branch_eq("arena_alloc_grow"),
        abi::load_u64("x24", "x22", 8),          // cur_size
        abi::subtract_immediate("x9", "x21", 1), // align mask
        abi::add_registers("x25", "x22", "x9"),
        abi::compare_registers("x25", "x22"),
        abi::branch_lo("arena_alloc_walk_next"), // align overflow → skip
        abi::bitwise_not("x10", "x9"),
        abi::and_registers("x25", "x25", "x10"), // aligned
        abi::add_registers("x26", "x25", "x20"), // end_needed
        abi::compare_registers("x26", "x25"),
        abi::branch_lo("arena_alloc_walk_next"), // size overflow → skip
        abi::add_registers("x27", "x22", "x24"), // cur_end
        abi::compare_registers("x26", "x27"),
        abi::branch_hi("arena_alloc_walk_next"), // doesn't fit → next
        abi::branch("arena_alloc_found"),
        abi::label("arena_alloc_walk_next"),
        abi::move_register("x23", "x22"),
        abi::load_u64("x22", "x22", 0),
        abi::branch("arena_alloc_walk_loop"),
        // --- Found: split the chosen chunk -------------------------------------
        // cur=x22, prev=x23, cur_size=x24, aligned=x25, end_needed=x26,
        // cur_end=x27, next=x9, front_pad=x14, tail_size=x15, link target=x10.
        abi::label("arena_alloc_found"),
        abi::load_u64("x9", "x22", 0),                // next
        abi::subtract_registers("x14", "x25", "x22"), // front_pad
        abi::subtract_registers("x15", "x27", "x26"), // tail_size
        abi::compare_immediate("x14", "0"),
        abi::branch_ne("arena_alloc_have_front"),
        abi::compare_immediate("x15", "0"),
        abi::branch_ne("arena_alloc_front0_tail1"),
        // case: chunk consumed exactly → link target is `next`
        abi::move_register("x10", "x9"),
        abi::branch("arena_alloc_set_prev_link"),
        abi::label("arena_alloc_front0_tail1"),
        // case: tail remainder only → new free node at end_needed
        abi::store_u64("x9", "x26", 0),
        abi::store_u64("x15", "x26", 8),
        abi::move_register("x10", "x26"),
        abi::branch("arena_alloc_set_prev_link"),
        abi::label("arena_alloc_have_front"),
        abi::compare_immediate("x15", "0"),
        abi::branch_ne("arena_alloc_front1_tail1"),
        // case: front padding only → shrink node in place at cur
        abi::store_u64("x9", "x22", 0),
        abi::store_u64("x14", "x22", 8),
        abi::move_register("x10", "x22"),
        abi::branch("arena_alloc_set_prev_link"),
        abi::label("arena_alloc_front1_tail1"),
        // case: both front and tail remainders → two free nodes
        abi::store_u64("x26", "x22", 0), // cur.next → tail node
        abi::store_u64("x14", "x22", 8), // cur.size = front_pad
        abi::store_u64("x9", "x26", 0),  // tail.next = next
        abi::store_u64("x15", "x26", 8), // tail.size = tail_size
        abi::move_register("x10", "x22"),
        abi::label("arena_alloc_set_prev_link"),
        abi::compare_immediate("x23", "0"),
        abi::branch_eq("arena_alloc_set_head"),
        abi::store_u64("x10", "x23", 0),
        abi::branch("arena_alloc_done"),
        abi::label("arena_alloc_set_head"),
        abi::store_u64("x10", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::label("arena_alloc_done"),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register("x1", "x25"),
        abi::branch("arena_alloc_ret"),
        // --- Grow: map a new block and add its usable region as a free chunk ---
        abi::label("arena_alloc_grow"),
        abi::add_registers("x23", "x20", "x21"),
        abi::compare_registers("x23", "x20"),
        abi::branch_lo("arena_alloc_oom"),
        abi::add_immediate("x23", "x23", ARENA_BLOCK_HEADER_SIZE),
        abi::move_immediate("x14", "Integer", &ARENA_DEFAULT_BLOCK_SIZE.to_string()),
        abi::compare_registers("x23", "x14"),
        abi::branch_hi("arena_alloc_normal_block"),
        abi::move_immediate("x23", "Integer", &ARENA_DEFAULT_BLOCK_SIZE.to_string()),
        abi::branch("arena_alloc_map_size_ready"),
        abi::label("arena_alloc_normal_block"),
        abi::move_register("x15", "x23"),
        abi::add_immediate("x23", "x23", 4095),
        abi::compare_registers("x23", "x15"),
        abi::branch_lo("arena_alloc_oom"),
        abi::move_immediate("x24", "Integer", &(!4095_u64).to_string()),
        abi::and_registers("x23", "x23", "x24"),
        abi::label("arena_alloc_map_size_ready"),
    ]);
    platform.emit_arena_map(&mut instructions)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge("arena_alloc_mapped"),
        abi::branch("arena_alloc_oom"),
        abi::label("arena_alloc_mapped"),
        // Write the block header (prevBlock, blockSize, usableCapacity, bumpOffset)
        // and chain it. bumpOffset is vestigial under the free-list but kept zero
        // so the documented block layout is unchanged.
        abi::load_u64("x24", ARENA_STATE_REGISTER, 0),
        abi::store_u64("x24", abi::return_register(), 0),
        abi::store_u64("x23", abi::return_register(), 8),
        abi::subtract_immediate("x24", "x23", ARENA_BLOCK_HEADER_SIZE),
        abi::store_u64("x24", abi::return_register(), 16),
        abi::store_u64("x31", abi::return_register(), 24),
        abi::store_u64(abi::return_register(), ARENA_STATE_REGISTER, 0),
        // Poison the new block's usable region before first use (plan-01 §6.3).
        // fill_random clobbers x0/x1/x9–x16 and advances x0, so stash ubase/usize
        // and the caller-survivor registers x11–x13 across the call.
        abi::add_immediate("x9", abi::return_register(), ARENA_BLOCK_HEADER_SIZE), // ubase
        abi::store_u64("x9", abi::stack_pointer(), UBASE_SLOT),
        abi::store_u64("x24", abi::stack_pointer(), USIZE_SLOT),
        abi::store_u64("x11", abi::stack_pointer(), X11_SLOT),
        abi::store_u64("x12", abi::stack_pointer(), X12_SLOT),
        abi::store_u64("x13", abi::stack_pointer(), X13_SLOT),
        abi::move_register("x0", "x9"),
        abi::move_register("x1", "x24"),
        abi::branch_link(ARENA_FILL_RANDOM_SYMBOL),
        abi::load_u64("x9", abi::stack_pointer(), UBASE_SLOT),
        abi::load_u64("x10", abi::stack_pointer(), USIZE_SLOT),
        abi::load_u64("x11", abi::stack_pointer(), X11_SLOT),
        abi::load_u64("x12", abi::stack_pointer(), X12_SLOT),
        abi::load_u64("x13", abi::stack_pointer(), X13_SLOT),
        // Insert [base+32, base+32+usableCapacity) as one free chunk, in address
        // order. A fresh block is never adjacent to an existing chunk (the 32-byte
        // header always separates blocks), so no coalescing is required here.
        abi::load_u64("x14", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET), // cur
        abi::move_immediate("x15", "Integer", "0"),                              // prev
        abi::label("arena_alloc_ins_loop"),
        abi::compare_immediate("x14", "0"),
        abi::branch_eq("arena_alloc_ins_do"),
        abi::compare_registers("x14", "x9"),
        abi::branch_hi("arena_alloc_ins_do"),
        abi::move_register("x15", "x14"),
        abi::load_u64("x14", "x14", 0),
        abi::branch("arena_alloc_ins_loop"),
        abi::label("arena_alloc_ins_do"),
        abi::store_u64("x14", "x9", 0),
        abi::store_u64("x10", "x9", 8),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq("arena_alloc_ins_head"),
        abi::store_u64("x9", "x15", 0),
        abi::branch("arena_alloc_walk"),
        abi::label("arena_alloc_ins_head"),
        abi::store_u64("x9", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::branch("arena_alloc_walk"),
        abi::label("arena_alloc_invalid"),
        abi::move_immediate(abi::return_register(), "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate("x1", "Integer", "0"),
        abi::branch("arena_alloc_ret"),
        abi::label("arena_alloc_oom"),
        abi::move_immediate(abi::return_register(), "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate("x1", "Integer", "0"),
        abi::label("arena_alloc_ret"),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    relocations.push(internal_branch(
        ARENA_ALLOC_SYMBOL,
        ARENA_FILL_RANDOM_SYMBOL,
    ));
    Ok(CodeFunction {
        name: "runtime.arena_alloc".to_string(),
        symbol: ARENA_ALLOC_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Pointer".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

/// `_mfb_simd_alloc_list(x0 = count, x1 = valueTypeCode) -> x0 = base` —
/// allocate a tight homogeneous numeric `List` (plan-01-simd §4.3). The data
/// region is `count` contiguous 8-byte lanes at `base + 40 + count*40`. Returns
/// `0` if the arena allocation fails (the caller raises the allocation error).
///
/// Calls `_mfb_arena_alloc`, whose clobber set is wide (`x0,x1,x9,x10,x14,x15,
/// x16,x20-x28`); `count` and `valueTypeCode` are spilled across the call and
/// reloaded. After the call there are no further calls, so the header/entry
/// writes use scratch GPRs freely.
fn lower_simd_alloc_list() -> CodeFunction {
    const FRAME_SIZE: usize = 32;
    const LR_SLOT: usize = 0;
    const COUNT_SLOT: usize = 8;
    const TYPE_SLOT: usize = 16;
    let mut relocations = Vec::new();
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::store_u64("x0", abi::stack_pointer(), COUNT_SLOT),
        abi::store_u64("x1", abi::stack_pointer(), TYPE_SLOT),
        // alloc size = 40 (header) + count*40 (lookup table) + count*8 (data)
        //            = 40 + count*48.
        abi::move_immediate(
            "x9",
            "Integer",
            &(COLLECTION_ENTRY_SIZE + 8).to_string(),
        ),
        abi::multiply_registers("x0", "x0", "x9"),
        abi::add_immediate("x0", "x0", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        // x0 = result tag, x1 = pointer. Return x0 = base, x1 = status (0 = ok,
        // else the arena error tag) so the caller can raise the allocation error.
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("simd_alloc_ok"),
        abi::move_register("x1", abi::return_register()),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::branch("simd_alloc_ret"),
        abi::label("simd_alloc_ok"),
        // x11 = base, x12 = count, x13 = typeCode.
        abi::move_register("x11", "x1"),
        abi::load_u64("x12", abi::stack_pointer(), COUNT_SLOT),
        abi::load_u64("x13", abi::stack_pointer(), TYPE_SLOT),
        // Header: kind=0 (list), keyType=0, valueType=typeCode, flagsVersion=1.
        abi::move_immediate("x8", "Integer", "0"),
        abi::store_u8("x8", "x11", COLLECTION_OFFSET_KIND),
        abi::store_u8("x8", "x11", COLLECTION_OFFSET_KEY_TYPE),
        abi::store_u8("x13", "x11", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x8", "Integer", "1"),
        abi::store_u8("x8", "x11", COLLECTION_OFFSET_FLAGS_VERSION),
        // count, capacity = count; dataLength, dataCapacity = count*8.
        abi::store_u64("x12", "x11", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x12", "x11", COLLECTION_OFFSET_CAPACITY),
        abi::shift_left_immediate("x9", "x12", 3),
        abi::store_u64("x9", "x11", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x9", "x11", COLLECTION_OFFSET_DATA_CAPACITY),
        // Fill the lookup entries: flags=USED, valueOffset=i*8, valueLength=8.
        // x10 = entry ptr, x9 = index, x14 = running value offset.
        abi::add_immediate("x10", "x11", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x9", "Integer", "0"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label("simd_alloc_entry_loop"),
        abi::compare_registers("x9", "x12"),
        abi::branch_ge("simd_alloc_entry_done"),
        abi::move_immediate("x8", "Integer", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("x8", "x10", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x14", "x10", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("x8", "Integer", "8"),
        abi::store_u64("x8", "x10", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_immediate("x14", "x14", 8),
        abi::add_immediate("x10", "x10", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x9", "x9", 1),
        abi::branch("simd_alloc_entry_loop"),
        abi::label("simd_alloc_entry_done"),
        abi::move_register(abi::return_register(), "x11"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::label("simd_alloc_ret"),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ];
    relocations.push(internal_branch(SIMD_ALLOC_LIST_SYMBOL, ARENA_ALLOC_SYMBOL));
    CodeFunction {
        name: "runtime.simd_alloc_list".to_string(),
        symbol: SIMD_ALLOC_LIST_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Pointer".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}

/// `arena_insert_free(x0 = ptr, x1 = size)` — insert a chunk into the
/// address-ordered free-list and coalesce with the address-adjacent neighbor on
/// either side. `size` must already be normalized (≥16, multiple of 16) and
/// `ptr` 16-aligned; both hold for every chunk the allocator hands out and for a
/// fresh block's usable region. Leaf function; clobbers x9–x13.
fn lower_arena_insert_free() -> CodeFunction {
    let instructions = vec![
        abi::label("entry"),
        // Walk to the insertion slot: prev (x10) = largest node < ptr (or 0),
        // cur (x9) = smallest node > ptr (or 0).
        abi::load_u64("x9", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::move_immediate("x10", "Integer", "0"),
        abi::label("insert_find"),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq("insert_slot"),
        abi::compare_registers("x9", "x0"),
        abi::branch_hi("insert_slot"), // cur > ptr
        abi::move_register("x10", "x9"),
        abi::load_u64("x9", "x9", 0),
        abi::branch("insert_find"),
        abi::label("insert_slot"),
        // x13 = merged-into-prev flag.
        abi::move_immediate("x13", "Integer", "0"),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq("insert_check_next"),
        abi::load_u64("x11", "x10", 8),          // prev.size
        abi::add_registers("x12", "x10", "x11"), // prev_end
        abi::compare_registers("x12", "x0"),
        abi::branch_ne("insert_check_next"),
        // prev is address-adjacent: absorb the chunk into prev.
        abi::add_registers("x11", "x11", "x1"),
        abi::store_u64("x11", "x10", 8),
        abi::move_immediate("x13", "Integer", "1"),
        abi::label("insert_check_next"),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq("insert_finish_no_next"),
        abi::compare_immediate("x13", "0"),
        abi::branch_eq("insert_next_unmerged"),
        // Merged into prev already: does the (now larger) prev meet cur?
        abi::load_u64("x11", "x10", 8),
        abi::add_registers("x12", "x10", "x11"),
        abi::compare_registers("x12", "x9"),
        abi::branch_ne("insert_done"),
        // Absorb cur into prev too (three-way merge).
        abi::load_u64("x11", "x9", 8),  // cur.size
        abi::load_u64("x12", "x10", 8), // prev.size
        abi::add_registers("x12", "x12", "x11"),
        abi::store_u64("x12", "x10", 8),
        abi::load_u64("x11", "x9", 0), // cur.next
        abi::store_u64("x11", "x10", 0),
        abi::branch("insert_done"),
        abi::label("insert_next_unmerged"),
        abi::add_registers("x12", "x0", "x1"), // chunk_end
        abi::compare_registers("x12", "x9"),
        abi::branch_ne("insert_standalone"),
        // chunk is address-adjacent to cur: new node at ptr absorbs cur.
        abi::load_u64("x11", "x9", 8), // cur.size
        abi::add_registers("x11", "x11", "x1"),
        abi::store_u64("x11", "x0", 8),
        abi::load_u64("x11", "x9", 0), // cur.next
        abi::store_u64("x11", "x0", 0),
        abi::branch("insert_link_prev"),
        abi::label("insert_standalone"),
        abi::store_u64("x9", "x0", 0), // ptr.next = cur
        abi::store_u64("x1", "x0", 8), // ptr.size = size
        abi::branch("insert_link_prev"),
        abi::label("insert_finish_no_next"),
        abi::compare_immediate("x13", "0"),
        abi::branch_ne("insert_done"), // merged into prev, nothing to link
        abi::store_u64("x31", "x0", 0), // ptr.next = 0
        abi::store_u64("x1", "x0", 8), // ptr.size = size
        abi::branch("insert_link_prev"),
        abi::label("insert_link_prev"),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq("insert_set_head"),
        abi::store_u64("x0", "x10", 0), // prev.next = ptr
        abi::branch("insert_done"),
        abi::label("insert_set_head"),
        abi::store_u64("x0", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::label("insert_done"),
        abi::return_(),
    ];
    CodeFunction {
        name: "runtime.arena_insert_free".to_string(),
        symbol: ARENA_INSERT_FREE_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// `arena_free(x0 = ptr, x1 = size)` — return a single compiler-sized allocation
/// to the per-arena free-list. Normalizes `size` exactly as `arena_alloc` did
/// (so the freed extent matches the live chunk), entropy-scrubs the chunk
/// (plan-01 §6.2), then coalesces it in via `arena_insert_free`. Never unmaps.
/// Clobbers x9–x16.
fn lower_arena_free() -> CodeFunction {
    const FRAME_SIZE: usize = 32;
    const LR_SLOT: usize = 0;
    const PTR_SLOT: usize = 8;
    const SIZE_SLOT: usize = 16;
    let not_15 = (!(ARENA_MIN_CHUNK - 1)).to_string();
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        // normalize size = round_up(max(size, 1), 16)
        abi::compare_immediate("x1", "0"),
        abi::branch_ne("arena_free_size_nonzero"),
        abi::move_immediate("x1", "Integer", "1"),
        abi::label("arena_free_size_nonzero"),
        abi::add_immediate("x1", "x1", (ARENA_MIN_CHUNK - 1) as usize),
        abi::move_immediate("x9", "Integer", &not_15),
        abi::and_registers("x1", "x1", "x9"),
        // Scrub the chunk: fill_random clobbers x0/x1/x9–x16 and advances x0, so
        // stash ptr/size and reload them for the coalescing insert.
        abi::store_u64("x0", abi::stack_pointer(), PTR_SLOT),
        abi::store_u64("x1", abi::stack_pointer(), SIZE_SLOT),
        abi::branch_link(ARENA_FILL_RANDOM_SYMBOL),
        abi::load_u64("x0", abi::stack_pointer(), PTR_SLOT),
        abi::load_u64("x1", abi::stack_pointer(), SIZE_SLOT),
        abi::branch_link(ARENA_INSERT_FREE_SYMBOL),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ];
    CodeFunction {
        name: "runtime.arena_free".to_string(),
        symbol: ARENA_FREE_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: vec![
            internal_branch(ARENA_FREE_SYMBOL, ARENA_FILL_RANDOM_SYMBOL),
            internal_branch(ARENA_FREE_SYMBOL, ARENA_INSERT_FREE_SYMBOL),
        ],
    }
}

fn lower_arena_destroy(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    let mut instructions = Vec::new();
    instructions.extend([
        abi::label("entry"),
        abi::load_u64("x20", ARENA_STATE_REGISTER, 0),
        abi::label("arena_destroy_loop"),
        abi::compare_immediate("x20", "0"),
        abi::branch_eq("arena_destroy_done"),
        abi::load_u64("x21", "x20", 0),
        abi::load_u64("x1", "x20", 8),
        abi::move_register(abi::return_register(), "x20"),
    ]);
    platform.emit_arena_unmap(&mut instructions)?;
    instructions.extend([
        abi::move_register("x20", "x21"),
        abi::branch("arena_destroy_loop"),
        abi::label("arena_destroy_done"),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 0),
        abi::return_(),
    ]);
    Ok(CodeFunction {
        name: "runtime.arena_destroy".to_string(),
        symbol: ARENA_DESTROY_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    })
}

/// Shared process teardown. Reads the main arena-state address from the writable
/// global, clears the global (so a second entry — e.g. a signal arriving during
/// normal cleanup — becomes a no-op), pins it in `x19`, then conditionally
/// restores the terminal and frees the arena. Both underlying helpers are
/// idempotent (`term::off` gates on its `active` flag; `arena_destroy` clears the
/// block-list head), so the guard is belt-and-suspenders. Preserves `x19`/`x30`
/// for its callers (the entry exit path relies on `x19` afterwards).
fn lower_shutdown(auto_term_off: bool, skip_arena_destroy: bool) -> CodeFunction {
    let done = "shutdown_done";
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(16),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), 8),
    ];
    let mut relocations = Vec::new();
    push_symbol_address(
        SHUTDOWN_SYMBOL,
        MAIN_ARENA_GLOBAL_SYMBOL,
        "x9",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64("x10", "x9", 0),
        abi::store_u64("x31", "x9", 0),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(done),
        abi::move_register(ARENA_STATE_REGISTER, "x10"),
    ]);
    if auto_term_off {
        instructions.push(abi::branch_link("_mfb_rt_term_term_off"));
        relocations.push(internal_branch(SHUTDOWN_SYMBOL, "_mfb_rt_term_term_off"));
    }
    if !skip_arena_destroy {
        instructions.push(abi::branch_link(ARENA_DESTROY_SYMBOL));
        relocations.push(internal_branch(SHUTDOWN_SYMBOL, ARENA_DESTROY_SYMBOL));
    }
    instructions.extend([
        abi::label(done),
        abi::load_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), 8),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(16),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.shutdown".to_string(),
        symbol: SHUTDOWN_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}

/// `void handler(int signo)` for SIGINT/SIGTERM: run the shared teardown, then
/// `_exit(128 + signo)`. It never returns, so it need not preserve the
/// interrupted context; it locates the arena through `_mfb_shutdown`'s global
/// read rather than the interrupted `x19`. The 16-byte frame keeps `sp` aligned
/// across the `bl`s (Darwin requires this) and parks `signo` across the call.
fn lower_signal_handler(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(16),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), 0),
        abi::branch_link(SHUTDOWN_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(SIGNAL_HANDLER_SYMBOL, SHUTDOWN_SYMBOL)];
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), 0),
        abi::add_immediate(abi::return_register(), abi::return_register(), 128),
        abi::add_stack(16),
    ]);
    platform.emit_program_exit(SIGNAL_HANDLER_SYMBOL, &mut instructions, &mut relocations)?;
    Ok(CodeFunction {
        name: "runtime.signal_handler".to_string(),
        symbol: SIGNAL_HANDLER_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

/// Append the PCG64 LCG step `state = state * MULT + INC` operating on the
/// 128-bit state held in (`lo`, `hi`). The limbs are read at the start and
/// rewritten in place; `x11`-`x16` are used as scratch (caller-saved, so these
/// leaf helpers need not preserve them).
fn emit_pcg_step(instructions: &mut Vec<CodeInstruction>, lo: &str, hi: &str) {
    instructions.extend([
        // 128-bit (truncated) product of state by the 128-bit multiplier.
        abi::move_immediate("x11", "Integer", &PCG_MULT_LO.to_string()),
        abi::multiply_registers("x13", "x11", lo), // result low limb
        abi::unsigned_multiply_high_registers("x14", "x11", lo), // carry into high
        abi::multiply_registers("x15", "x11", hi), // MULT_LO * state_hi
        abi::move_immediate("x12", "Integer", &PCG_MULT_HI.to_string()),
        abi::multiply_registers("x16", "x12", lo), // MULT_HI * state_lo
        abi::add_registers("x14", "x14", "x15"),
        abi::add_registers("x14", "x14", "x16"), // result high limb
        // Add the 128-bit increment with carry between limbs.
        abi::move_immediate("x11", "Integer", &PCG_INC_LO.to_string()),
        abi::move_immediate("x12", "Integer", &PCG_INC_HI.to_string()),
        abi::add_registers_set_flags(lo, "x13", "x11"),
        abi::add_with_carry_registers(hi, "x14", "x12"),
    ]);
}

/// `_mfb_rng_next` — advance the calling thread's PCG64 generator one step and
/// return the next 64-bit value in `x0`. State lives in the arena (`x19`).
fn lower_rng_next() -> CodeFunction {
    let mut instructions = vec![abi::label("entry")];
    instructions.extend([
        abi::load_u64("x9", ARENA_STATE_REGISTER, ARENA_RNG_STATE_LO_OFFSET),
        abi::load_u64("x10", ARENA_STATE_REGISTER, ARENA_RNG_STATE_HI_OFFSET),
    ]);
    emit_pcg_step(&mut instructions, "x9", "x10");
    instructions.extend([
        abi::store_u64("x9", ARENA_STATE_REGISTER, ARENA_RNG_STATE_LO_OFFSET),
        abi::store_u64("x10", ARENA_STATE_REGISTER, ARENA_RNG_STATE_HI_OFFSET),
        // XSL-RR output: rotate (hi ^ lo) right by the top 6 bits of hi.
        abi::shift_right_immediate("x11", "x10", 58),
        abi::exclusive_or_registers("x12", "x10", "x9"),
        abi::rotate_right_registers(abi::return_register(), "x12", "x11"),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.rng_next".to_string(),
        symbol: RNG_NEXT_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// `_mfb_rng_seed_at(x0 = arena ptr, x1 = seed)` — initialize the PCG64 state at
/// the given arena from a 64-bit seed, following the canonical seeding dance
/// (`state = 0; step; state += seed; step`).
fn lower_rng_seed_at() -> CodeFunction {
    let mut instructions = vec![abi::label("entry")];
    instructions.extend([
        abi::move_immediate("x9", "Integer", "0"),
        abi::move_immediate("x10", "Integer", "0"),
    ]);
    emit_pcg_step(&mut instructions, "x9", "x10");
    instructions.extend([
        abi::add_registers_set_flags("x9", "x9", "x1"),
        abi::add_with_carry_registers("x10", "x10", "xzr"),
    ]);
    emit_pcg_step(&mut instructions, "x9", "x10");
    instructions.extend([
        abi::store_u64("x9", "x0", ARENA_RNG_STATE_LO_OFFSET),
        abi::store_u64("x10", "x0", ARENA_RNG_STATE_HI_OFFSET),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.rng_seed_at".to_string(),
        symbol: RNG_SEED_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// `arena_fill_seed(x0 = arena ptr, x1 = seed)` — seed the dedicated fill RNG at
/// offsets 16/24 from a 64-bit seed (same PCG64 dance as `rng_seed_at`, different
/// state words). Leaf; clobbers x9–x16.
fn lower_arena_fill_seed() -> CodeFunction {
    let mut instructions = vec![abi::label("entry")];
    instructions.extend([
        abi::move_immediate("x9", "Integer", "0"),
        abi::move_immediate("x10", "Integer", "0"),
    ]);
    emit_pcg_step(&mut instructions, "x9", "x10");
    instructions.extend([
        abi::add_registers_set_flags("x9", "x9", "x1"),
        abi::add_with_carry_registers("x10", "x10", "xzr"),
    ]);
    emit_pcg_step(&mut instructions, "x9", "x10");
    instructions.extend([
        abi::store_u64("x9", "x0", ARENA_FILL_RNG_LO_OFFSET),
        abi::store_u64("x10", "x0", ARENA_FILL_RNG_HI_OFFSET),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.arena_fill_seed".to_string(),
        symbol: ARENA_FILL_SEED_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// `arena_fill_next()` — advance the calling thread's fill RNG (`x19`, offsets
/// 16/24) and return the next 64-bit XSL-RR output in `x0`. Leaf; clobbers
/// x9–x16. Used only to draw a child fill seed from the parent at spawn.
fn lower_arena_fill_next() -> CodeFunction {
    let mut instructions = vec![abi::label("entry")];
    instructions.extend([
        abi::load_u64("x9", ARENA_STATE_REGISTER, ARENA_FILL_RNG_LO_OFFSET),
        abi::load_u64("x10", ARENA_STATE_REGISTER, ARENA_FILL_RNG_HI_OFFSET),
    ]);
    emit_pcg_step(&mut instructions, "x9", "x10");
    instructions.extend([
        abi::store_u64("x9", ARENA_STATE_REGISTER, ARENA_FILL_RNG_LO_OFFSET),
        abi::store_u64("x10", ARENA_STATE_REGISTER, ARENA_FILL_RNG_HI_OFFSET),
        abi::shift_right_immediate("x11", "x10", 58),
        abi::exclusive_or_registers("x12", "x10", "x9"),
        abi::rotate_right_registers(abi::return_register(), "x12", "x11"),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.arena_fill_next".to_string(),
        symbol: ARENA_FILL_NEXT_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// `arena_fill_random(x0 = ptr, x1 = len)` — overwrite `len` bytes at `ptr` with
/// output from the calling thread's fill RNG. `len` is rounded up to an 8-byte
/// word; every chunk handed to this helper is a multiple of 16 bytes, so the
/// rounding is exact and never writes past the chunk. Streams PRNG words without
/// a syscall (§6.1). Leaf; clobbers x0, x1, x9–x16.
fn lower_arena_fill_random() -> CodeFunction {
    let mut instructions = vec![
        abi::label("entry"),
        // word count = (len + 7) >> 3
        abi::add_immediate("x1", "x1", 7),
        abi::shift_right_immediate("x1", "x1", 3),
        abi::compare_immediate("x1", "0"),
        abi::branch_eq("arena_fill_done"),
        abi::load_u64("x9", ARENA_STATE_REGISTER, ARENA_FILL_RNG_LO_OFFSET),
        abi::load_u64("x10", ARENA_STATE_REGISTER, ARENA_FILL_RNG_HI_OFFSET),
        abi::label("arena_fill_loop"),
    ];
    emit_pcg_step(&mut instructions, "x9", "x10");
    instructions.extend([
        abi::shift_right_immediate("x11", "x10", 58),
        abi::exclusive_or_registers("x12", "x10", "x9"),
        abi::rotate_right_registers("x13", "x12", "x11"),
        abi::store_u64("x13", "x0", 0),
        abi::add_immediate("x0", "x0", 8),
        abi::subtract_immediate("x1", "x1", 1),
        abi::compare_immediate("x1", "0"),
        abi::branch_ne("arena_fill_loop"),
        abi::store_u64("x9", ARENA_STATE_REGISTER, ARENA_FILL_RNG_LO_OFFSET),
        abi::store_u64("x10", ARENA_STATE_REGISTER, ARENA_FILL_RNG_HI_OFFSET),
        abi::label("arena_fill_done"),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.arena_fill_random".to_string(),
        symbol: ARENA_FILL_RANDOM_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

fn emit_write_string_object(
    symbol: &str,
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.extend([
        abi::load_page_address("x21", symbol),
        abi::add_page_offset("x21", "x21", symbol),
        abi::load_u64(abi::string_length_register(), "x21", 0),
        abi::add_immediate(abi::string_data_register(), "x21", 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)
}

fn emit_write_integer_to_stderr(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_write_integer_to_stderr_with_labels(
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
        "entry_error_code",
    )
}

fn emit_write_integer_to_stderr_with_labels(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    label_prefix: &str,
) -> Result<(), String> {
    let absolute_ready_label = format!("{label_prefix}_absolute_ready");
    let digit_loop_label = format!("{label_prefix}_digit_loop");
    let digits_done_label = format!("{label_prefix}_digits_done");
    let write_label = format!("{label_prefix}_write");
    instructions.extend([
        abi::subtract_stack(64),
        abi::load_u64("x21", ARENA_STATE_REGISTER, 32),
        abi::compare_immediate("x21", "0"),
        abi::branch_ge(&absolute_ready_label),
        abi::move_immediate("x22", "Integer", "0"),
        abi::subtract_registers("x21", "x22", "x21"),
        abi::label(&absolute_ready_label),
        abi::add_immediate("x23", abi::stack_pointer(), 64),
        abi::move_immediate("x24", "Integer", "10"),
        abi::compare_immediate("x21", "0"),
        abi::branch_ne(&digit_loop_label),
        abi::subtract_immediate("x23", "x23", 1),
        abi::move_immediate("x22", "Integer", "48"),
        abi::store_u8("x22", "x23", 0),
        abi::branch(&digits_done_label),
        abi::label(&digit_loop_label),
        abi::unsigned_divide_registers("x25", "x21", "x24"),
        abi::multiply_subtract_registers("x26", "x25", "x24", "x21"),
        abi::add_immediate("x26", "x26", 48),
        abi::subtract_immediate("x23", "x23", 1),
        abi::store_u8("x26", "x23", 0),
        abi::move_register("x21", "x25"),
        abi::compare_immediate("x21", "0"),
        abi::branch_ne(&digit_loop_label),
        abi::label(&digits_done_label),
        abi::compare_immediate("x19", "0"),
        abi::branch_ge(&write_label),
        abi::subtract_immediate("x23", "x23", 1),
        abi::move_immediate("x22", "Integer", "45"),
        abi::store_u8("x22", "x23", 0),
        abi::label(&write_label),
        abi::add_immediate("x27", abi::stack_pointer(), 64),
        abi::subtract_registers(abi::string_length_register(), "x27", "x23"),
        abi::move_register(abi::string_data_register(), "x23"),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)?;
    instructions.push(abi::add_stack(64));
    Ok(())
}

fn lower_function(
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

fn lower_builtin_function_wrapper(
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

    let mut instructions = builder.instructions;
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

fn lower_runtime_helper(
    symbol: &str,
    build_mode: crate::target::NativeBuildMode,
    term_state_offset: Option<usize>,
    uses_rng: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let Some(spec) = runtime::spec_for_symbol(symbol) else {
        return Err(format!(
            "native code plan does not emit runtime helper '{symbol}'"
        ));
    };
    let app_mode = build_mode.is_app();
    if builtins::term::is_term_call(spec.call) {
        let term_state_offset = term_state_offset.ok_or_else(|| {
            format!("native code plan emits '{symbol}' without reserving term state")
        })?;
        // App mode drives the synthesized TermView surface for the mode toggle
        // (plan-01-term.md §6.3); the remaining term:: helpers keep the shared
        // console backend until Phase 5 wires their app bodies.
        let app_term_helper = if app_mode {
            platform.emit_app_term_helper(spec.call, symbol, term_state_offset)
        } else {
            None
        };
        let (frame, instructions, relocations) = match app_term_helper {
            Some(result) => result?,
            None => term::lower_term_helper(
                spec.call,
                symbol,
                term_state_offset,
                platform_imports,
                platform,
            )?,
        };
        return Ok(CodeFunction {
            name: format!("runtime.{}", spec.call),
            symbol: symbol.to_string(),
            params: spec
                .abi
                .params
                .iter()
                .map(|param| CodeParam {
                    name: param.name.to_string(),
                    type_: param.type_.to_string(),
                    location: param.location.to_string(),
                })
                .collect(),
            returns: spec.abi.returns.to_string(),
            frame,
            stack_slots: Vec::new(),
            instructions,
            relocations,
        });
    }
    match spec.call {
        "datetime.nowNanos" | "datetime.monotonicNanos" | "datetime.localOffset" => {
            let (frame, instructions, relocations) =
                datetime::lower_datetime_helper(spec.call, symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.print" | "io.write" | "io.printError" | "io.writeError" => {
            let stderr = matches!(spec.call, "io.printError" | "io.writeError");
            let newline = matches!(spec.call, "io.print" | "io.printError");
            // App mode routes io output to the AppKit transcript window
            // (plan-04-macos-app.md §5.4) instead of a file descriptor.
            let (frame, instructions, relocations) = if app_mode {
                platform
                    .emit_app_io_write_helper(
                        symbol,
                        stderr,
                        newline,
                        term_state_offset,
                        platform_imports,
                    )
                    .ok_or_else(|| {
                        format!(
                            "native target '{}' does not support app-mode io helpers",
                            platform.target()
                        )
                    })??
            } else {
                lower_io_write_helper(symbol, platform_imports, platform, stderr, newline)?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.flush" | "io.flushError" => {
            // App-mode transcript writes are synchronous (each io write blocks on
            // the main thread via performSelectorOnMainThread), so output is
            // already visible; flush succeeds immediately (plan §5.4).
            let (frame, instructions, relocations) = if app_mode {
                platform.emit_app_io_flush_helper(symbol).ok_or_else(|| {
                    format!(
                        "native target '{}' does not support app-mode io helpers",
                        platform.target()
                    )
                })??
            } else {
                lower_io_flush_helper(
                    symbol,
                    platform_imports,
                    platform,
                    matches!(spec.call, "io.flushError"),
                )?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.pollInput" => {
            let (frame, instructions, relocations) =
                lower_io_poll_input_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.input" | "io.readLine" => {
            // App-mode io.input writes its prompt to the transcript (via io.write)
            // then reads a line (via io.readLine); io.readLine itself is the
            // unchanged console helper, which reads fd 0 — the window input pipe
            // in app mode (plan §5.4). All other read helpers are likewise
            // unchanged and read the pipe.
            let (frame, instructions, relocations) = if app_mode && spec.call == "io.input" {
                platform.emit_app_io_input_helper(symbol).ok_or_else(|| {
                    format!(
                        "native target '{}' does not support app-mode io helpers",
                        platform.target()
                    )
                })??
            } else {
                lower_io_read_line_helper(
                    symbol,
                    platform_imports,
                    platform,
                    spec.call == "io.input",
                )?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.readChar" => {
            let (frame, instructions, relocations) =
                lower_io_read_char_helper(symbol, platform_imports, platform, app_mode)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.readByte" => {
            let (frame, instructions, relocations) =
                lower_io_read_byte_helper(symbol, platform_imports, platform, app_mode)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => {
            let fd = match spec.call {
                "io.isInputTerminal" => 0,
                "io.isOutputTerminal" => 1,
                "io.isErrorTerminal" => 2,
                _ => unreachable!(),
            };
            // App mode: the window is the interactive console, so these return
            // TRUE rather than probing a file descriptor (plan §5.4).
            let (frame, instructions, relocations) = if app_mode {
                platform
                    .emit_app_io_is_terminal_helper(symbol)
                    .ok_or_else(|| {
                        format!(
                            "native target '{}' does not support app-mode io helpers",
                            platform.target()
                        )
                    })??
            } else {
                lower_io_is_terminal_helper(symbol, platform_imports, platform, fd)?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.exists" => {
            let (frame, instructions, relocations) =
                lower_fs_exists_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.fileExists" | "fs.directoryExists" => {
            let kind = if spec.call == "fs.fileExists" {
                FS_MODE_REGULAR
            } else {
                FS_MODE_DIRECTORY
            };
            let (frame, instructions, relocations) =
                lower_fs_kind_exists_helper(symbol, platform_imports, platform, kind)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.currentDirectory" | "fs.tempDirectory" => {
            let (frame, instructions, relocations) = if spec.call == "fs.currentDirectory" {
                lower_fs_current_directory_helper(symbol, platform_imports, platform)?
            } else {
                lower_fs_temp_directory_helper(symbol, platform_imports, platform)?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.setCurrentDirectory"
        | "fs.deleteFile"
        | "fs.createDirectory"
        | "fs.deleteDirectory" => {
            let operation = match spec.call {
                "fs.setCurrentDirectory" => FsPathOperation::Chdir,
                "fs.deleteFile" => FsPathOperation::Unlink,
                "fs.createDirectory" => FsPathOperation::Mkdir,
                "fs.deleteDirectory" => FsPathOperation::Rmdir,
                _ => unreachable!(),
            };
            let (frame, instructions, relocations) =
                lower_fs_path_operation_helper(symbol, platform_imports, platform, operation)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.createDirectories" => {
            let (frame, instructions, relocations) =
                lower_fs_create_directories_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.listDirectory" => {
            let (frame, instructions, relocations) =
                lower_fs_list_directory_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.open" | "fs.openFile" | "fs.openFileNoFollow" => {
            let no_follow = spec.call == "fs.openFileNoFollow";
            let (frame, instructions, relocations) =
                lower_fs_open_helper(symbol, platform_imports, platform, no_follow)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.createTempFile" => {
            let (frame, instructions, relocations) =
                lower_fs_create_temp_file_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.close" => {
            let (frame, instructions, relocations) =
                lower_fs_close_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.writeAll" => {
            let (frame, instructions, relocations) =
                lower_fs_write_all_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.writeAllBytes" => {
            let (frame, instructions, relocations) =
                lower_fs_write_all_bytes_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.readText" => {
            let (frame, instructions, relocations) =
                lower_fs_read_text_path_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.readBytes" => {
            let (frame, instructions, relocations) =
                lower_fs_read_bytes_path_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.writeText" | "fs.appendText" => {
            let append = spec.call == "fs.appendText";
            let (frame, instructions, relocations) =
                lower_fs_write_text_path_helper(symbol, platform_imports, platform, append)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.writeBytes" | "fs.appendBytes" => {
            let append = spec.call == "fs.appendBytes";
            let (frame, instructions, relocations) =
                lower_fs_write_bytes_path_helper(symbol, platform_imports, platform, append)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.writeTextAtomic" | "fs.writeBytesAtomic" => {
            let value_kind = if spec.call == "fs.writeTextAtomic" {
                AtomicWriteValueKind::String
            } else {
                AtomicWriteValueKind::Bytes
            };
            let (frame, instructions, relocations) =
                lower_fs_atomic_write_helper(symbol, platform_imports, platform, value_kind)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.readAll" => {
            let (frame, instructions, relocations) =
                lower_fs_read_all_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.readAllBytes" => {
            let (frame, instructions, relocations) =
                lower_fs_read_all_bytes_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.readLine" => {
            let (frame, instructions, relocations) =
                lower_fs_read_line_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.eof" => {
            let (frame, instructions, relocations) =
                lower_fs_eof_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.canonicalPath" => {
            let (frame, instructions, relocations) =
                lower_fs_canonical_path_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.isWithin" => {
            let (frame, instructions, relocations) =
                lower_fs_is_within_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "strings.trim"
        | "strings.trimStart"
        | "strings.trimEnd"
        | "strings.upper"
        | "strings.lower"
        | "strings.caseFold"
        | "strings.normalizeNfc"
        | "strings.graphemes"
        | "strings.startsWith"
        | "strings.endsWith"
        | "strings.contains"
        | "strings.split"
        | "strings.join"
        | "strings.byteLen" => lower_direct_builtin_runtime_helper(symbol, spec, platform_imports),
        "thread.start"
        | "thread.isRunning"
        | "thread.waitFor"
        | "thread.cancel"
        | "thread.drop"
        | "thread.send"
        | "thread.poll"
        | "thread.read"
        | "thread.receive"
        | "thread.emit"
        | "thread.transferResource"
        | "thread.acceptResource"
        | "thread.emitResource"
        | "thread.readResource"
        | "thread.isCancelled" => {
            let (frame, instructions, relocations) =
                lower_thread_helper(symbol, spec.call, uses_rng, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        call if call.starts_with("net.") => {
            let (frame, instructions, relocations) = match call {
                "net.lookup" => net::lower_net_lookup_helper(symbol, platform_imports, platform)?,
                "net.connectTcp" => {
                    net::lower_net_connect_tcp_helper(symbol, platform_imports, platform)?
                }
                "net.connectTcpAddr" => {
                    net::lower_net_connect_tcp_addr_helper(symbol, platform_imports, platform)?
                }
                "net.listenTcp" => {
                    net::lower_net_listen_tcp_helper(symbol, platform_imports, platform)?
                }
                "net.accept" => net::lower_net_accept_helper(symbol, platform_imports, platform)?,
                "net.poll" => net::lower_net_poll_helper(symbol, platform_imports, platform)?,
                "net.read" => {
                    net::lower_net_read_helper(symbol, platform_imports, platform, false)?
                }
                "net.readText" => {
                    net::lower_net_read_helper(symbol, platform_imports, platform, true)?
                }
                "net.write" => {
                    net::lower_net_write_helper(symbol, platform_imports, platform, false)?
                }
                "net.writeText" => {
                    net::lower_net_write_helper(symbol, platform_imports, platform, true)?
                }
                // A socket/listener handle shares the `File` record layout, so the
                // standard file close helper closes net handles too.
                "net.close" => lower_fs_close_helper(symbol, platform_imports, platform)?,
                "net.localAddress" => {
                    net::lower_net_address_helper(symbol, platform_imports, platform, false)?
                }
                "net.remoteAddress" => {
                    net::lower_net_address_helper(symbol, platform_imports, platform, true)?
                }
                "net.setReadTimeout" => {
                    net::lower_net_set_timeout_helper(symbol, platform_imports, platform, false)?
                }
                "net.setWriteTimeout" => {
                    net::lower_net_set_timeout_helper(symbol, platform_imports, platform, true)?
                }
                "net.bindUdp" => {
                    net::lower_net_bind_udp_helper(symbol, platform_imports, platform)?
                }
                "net.receiveFrom" => {
                    net::lower_net_receive_from_helper(symbol, platform_imports, platform, false)?
                }
                "net.receiveTextFrom" => {
                    net::lower_net_receive_from_helper(symbol, platform_imports, platform, true)?
                }
                "net.sendTo" => {
                    net::lower_net_send_to_helper(symbol, platform_imports, platform, false)?
                }
                "net.sendTextTo" => {
                    net::lower_net_send_to_helper(symbol, platform_imports, platform, true)?
                }
                other => {
                    return Err(format!(
                        "native code plan does not emit runtime call '{other}'"
                    ));
                }
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        call if call.starts_with("tls.") => {
            let (frame, instructions, relocations) = match call {
                "tls.connect" => tls::lower_tls_connect_helper(symbol, platform_imports, platform)?,
                "tls.read" => {
                    tls::lower_tls_read_helper(symbol, platform_imports, platform, false)?
                }
                "tls.readText" => {
                    tls::lower_tls_read_helper(symbol, platform_imports, platform, true)?
                }
                "tls.write" => {
                    tls::lower_tls_write_helper(symbol, platform_imports, platform, false)?
                }
                "tls.writeText" => {
                    tls::lower_tls_write_helper(symbol, platform_imports, platform, true)?
                }
                "tls.close" => tls::lower_tls_close_helper(symbol, platform_imports, platform)?,
                other => {
                    return Err(format!(
                        "native code plan does not emit runtime call '{other}'"
                    ));
                }
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        other => Err(format!(
            "native code plan does not emit runtime call '{other}'"
        )),
    }
}

fn lower_direct_builtin_runtime_helper(
    symbol: &str,
    spec: &runtime::RuntimeHelperSpec,
    platform_imports: &HashMap<String, String>,
) -> Result<CodeFunction, String> {
    let function_symbols = HashMap::new();
    let functions = HashMap::new();
    let package_return_types = HashMap::new();
    let globals = HashMap::new();
    let string_symbols = standard_error_messages()
        .iter()
        .map(|(_, message, symbol)| ((*message).to_string(), (*symbol).to_string()))
        .collect::<HashMap<_, _>>();
    let mut builder = CodeBuilder {
        current_symbol: symbol.to_string(),
        function_symbols: &function_symbols,
        functions: &functions,
        package_return_types: &package_return_types,
        platform_imports,
        globals: &globals,
        type_model: TypeModel::empty(),
        string_symbols: &string_symbols,
        locals: HashMap::new(),
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
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

    let args = spec
        .abi
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let slot = builder.allocate_stack_object(param.name, 8);
            builder.locals.insert(
                param.name.to_string(),
                LocalValue {
                    type_: param.type_.to_string(),
                    stack_offset: slot,
                    constant: None,
                    by_ref: false,
                },
            );
            builder.emit(abi::store_u64(
                &abi::argument_register(index)?,
                abi::stack_pointer(),
                slot,
            ));
            Ok(NirValue::Local(param.name.to_string()))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let result = builder.lower_value(&NirValue::Call {
        target: spec.call.to_string(),
        args,
        loc: NirSourceLoc::default(),
    })?;
    if spec.abi.returns != "Nothing" {
        builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
    }
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    let mut instructions = builder.instructions;
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );

    Ok(CodeFunction {
        name: format!("runtime.{}", spec.call),
        symbol: symbol.to_string(),
        params: spec
            .abi
            .params
            .iter()
            .map(|param| CodeParam {
                name: param.name.to_string(),
                type_: param.type_.to_string(),
                location: param.location.to_string(),
            })
            .collect(),
        returns: spec.abi.returns.to_string(),
        frame,
        stack_slots,
        instructions,
        relocations: builder.relocations,
    })
}


fn internal_branch(from: &str, to: &str) -> CodeRelocation {
    CodeRelocation {
        from: from.to_string(),
        to: to.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    }
}

fn external_branch(
    from: &str,
    to: &str,
    platform_imports: &HashMap<String, String>,
) -> Result<CodeRelocation, String> {
    let library = platform_imports
        .get(to)
        .ok_or_else(|| format!("thread runtime helper requires {to} import"))?
        .clone();
    Ok(CodeRelocation {
        from: from.to_string(),
        to: to.to_string(),
        kind: "branch26".to_string(),
        binding: "external".to_string(),
        library: Some(library),
    })
}


/// Build the FNV-1a hash buckets for the `Map` whose pointer is in `x0`
/// (plan-02 Phase 6). Zeroes the `2*capacity`-entry bucket array that sits past
/// the data region, then for each lookup entry hashes its key bytes
/// (`dataBase + keyOffset`, `keyLength`) and open-addresses `entryIndex + 1` into
/// the first empty bucket, finally setting the header "ready" flag. Preserves
/// `x0`/`x1`/`x2` (so it is safe to call from the probe); uses `x3`–`x17` scratch;
/// makes no calls. A `count == 0` map fills nothing.
fn lower_map_build_buckets_helper() -> CodeFunction {
    let symbol = MAP_BUILD_BUCKETS_SYMBOL;
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let zloop = format!("{symbol}_zloop");
    let zdone = format!("{symbol}_zdone");
    let eloop = format!("{symbol}_eloop");
    let edone = format!("{symbol}_edone");
    let hloop = format!("{symbol}_hloop");
    let hdone = format!("{symbol}_hdone");
    let ploop = format!("{symbol}_ploop");
    let place = format!("{symbol}_place");
    let nowrap = format!("{symbol}_nowrap");
    let instructions = vec![
        abi::label("entry"),
        // dataBase (x11) = x0 + HEADER + capacity*ENTRY ; bucketBase (x12) += dataCap.
        abi::load_u64("x9", "x0", COLLECTION_OFFSET_COUNT),
        abi::load_u64("x14", "x0", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("x16", "Integer", &entry_size),
        abi::multiply_registers("x11", "x14", "x16"),
        abi::add_registers("x11", "x11", "x0"),
        abi::add_immediate("x11", "x11", COLLECTION_HEADER_SIZE),
        abi::load_u64("x15", "x0", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers("x12", "x11", "x15"),
        abi::shift_left_immediate("x10", "x14", 1), // bucketCount = 2*capacity
        abi::move_immediate("x8", "Integer", FNV1A_PRIME),
        // Zero the bucket array.
        abi::move_immediate("x13", "Integer", "0"),
        abi::label(&zloop),
        abi::compare_registers("x13", "x10"),
        abi::branch_ge(&zdone),
        abi::shift_left_immediate("x7", "x13", 3),
        abi::add_registers("x7", "x12", "x7"),
        abi::move_immediate("x6", "Integer", "0"),
        abi::store_u64("x6", "x7", 0),
        abi::add_immediate("x13", "x13", 1),
        abi::branch(&zloop),
        abi::label(&zdone),
        // For each entry: hash its key, open-address its index+1.
        abi::move_immediate("x13", "Integer", "0"),
        abi::label(&eloop),
        abi::compare_registers("x13", "x9"),
        abi::branch_ge(&edone),
        abi::move_immediate("x16", "Integer", &entry_size),
        abi::multiply_registers("x14", "x13", "x16"),
        abi::add_registers("x14", "x14", "x0"),
        abi::add_immediate("x14", "x14", COLLECTION_HEADER_SIZE), // entry addr
        abi::load_u64("x15", "x14", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers("x15", "x11", "x15"), // keyPtr
        abi::load_u64("x17", "x14", COLLECTION_ENTRY_OFFSET_KEY_LENGTH), // keyLen
        abi::move_immediate("x16", "Integer", FNV1A_BASIS), // h
        abi::move_register("x5", "x15"),
        abi::move_register("x6", "x17"),
        abi::label(&hloop),
        abi::compare_immediate("x6", "0"),
        abi::branch_eq(&hdone),
        abi::load_u8("x3", "x5", 0),
        abi::exclusive_or_registers("x16", "x16", "x3"),
        abi::multiply_registers("x16", "x16", "x8"),
        abi::add_immediate("x5", "x5", 1),
        abi::subtract_immediate("x6", "x6", 1),
        abi::branch(&hloop),
        abi::label(&hdone),
        // slot = h mod bucketCount.
        abi::unsigned_divide_registers("x4", "x16", "x10"),
        abi::multiply_subtract_registers("x4", "x4", "x10", "x16"),
        abi::label(&ploop),
        abi::shift_left_immediate("x7", "x4", 3),
        abi::add_registers("x7", "x12", "x7"),
        abi::load_u64("x6", "x7", 0),
        abi::compare_immediate("x6", "0"),
        abi::branch_eq(&place),
        abi::add_immediate("x4", "x4", 1),
        abi::compare_registers("x4", "x10"),
        abi::branch_lo(&nowrap),
        abi::move_immediate("x4", "Integer", "0"),
        abi::label(&nowrap),
        abi::branch(&ploop),
        abi::label(&place),
        abi::add_immediate("x6", "x13", 1),
        abi::store_u64("x6", "x7", 0),
        abi::add_immediate("x13", "x13", 1),
        abi::branch(&eloop),
        abi::label(&edone),
        abi::move_immediate("x6", "Integer", "1"),
        abi::store_u8("x6", "x0", COLLECTION_OFFSET_BUCKETS_READY),
        abi::return_(),
    ];
    CodeFunction {
        name: "runtime.mapBuildBuckets".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// Incrementally insert lookup entry `x1` of the `Map` in `x0` into its (already
/// built) bucket array (plan-02 Phase 6): hashes that entry's key and stores
/// `x1 + 1` into the first empty bucket. The `2*capacity` load factor guarantees a
/// free slot until the next capacity grow (which marks the index not-ready). Used
/// by the in-place `set` insert so building a map via repeated `set` stays O(n).
/// Makes no calls; preserves `x0`.
fn lower_map_bucket_put_helper() -> CodeFunction {
    let symbol = MAP_BUCKET_PUT_SYMBOL;
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let hloop = format!("{symbol}_hloop");
    let hdone = format!("{symbol}_hdone");
    let ploop = format!("{symbol}_ploop");
    let place = format!("{symbol}_place");
    let nowrap = format!("{symbol}_nowrap");
    let instructions = vec![
        abi::label("entry"),
        abi::load_u64("x14", "x0", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("x16", "Integer", &entry_size),
        abi::multiply_registers("x11", "x14", "x16"),
        abi::add_registers("x11", "x11", "x0"),
        abi::add_immediate("x11", "x11", COLLECTION_HEADER_SIZE), // dataBase
        abi::load_u64("x15", "x0", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers("x12", "x11", "x15"), // bucketBase
        abi::shift_left_immediate("x10", "x14", 1), // bucketCount
        abi::move_immediate("x8", "Integer", FNV1A_PRIME),
        // entry addr = x0 + HEADER + x1*ENTRY.
        abi::move_immediate("x16", "Integer", &entry_size),
        abi::multiply_registers("x14", "x1", "x16"),
        abi::add_registers("x14", "x14", "x0"),
        abi::add_immediate("x14", "x14", COLLECTION_HEADER_SIZE),
        abi::load_u64("x15", "x14", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers("x15", "x11", "x15"),
        abi::load_u64("x17", "x14", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::move_immediate("x16", "Integer", FNV1A_BASIS),
        abi::move_register("x5", "x15"),
        abi::move_register("x6", "x17"),
        abi::label(&hloop),
        abi::compare_immediate("x6", "0"),
        abi::branch_eq(&hdone),
        abi::load_u8("x3", "x5", 0),
        abi::exclusive_or_registers("x16", "x16", "x3"),
        abi::multiply_registers("x16", "x16", "x8"),
        abi::add_immediate("x5", "x5", 1),
        abi::subtract_immediate("x6", "x6", 1),
        abi::branch(&hloop),
        abi::label(&hdone),
        abi::unsigned_divide_registers("x4", "x16", "x10"),
        abi::multiply_subtract_registers("x4", "x4", "x10", "x16"),
        abi::label(&ploop),
        abi::shift_left_immediate("x7", "x4", 3),
        abi::add_registers("x7", "x12", "x7"),
        abi::load_u64("x6", "x7", 0),
        abi::compare_immediate("x6", "0"),
        abi::branch_eq(&place),
        abi::add_immediate("x4", "x4", 1),
        abi::compare_registers("x4", "x10"),
        abi::branch_lo(&nowrap),
        abi::move_immediate("x4", "Integer", "0"),
        abi::label(&nowrap),
        abi::branch(&ploop),
        abi::label(&place),
        abi::add_immediate("x6", "x1", 1),
        abi::store_u64("x6", "x7", 0),
        abi::return_(),
    ];
    CodeFunction {
        name: "runtime.mapBucketPut".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// Probe the `Map` in `x0` for the key whose bytes are `x1` (pointer) / `x2`
/// (length); returns the matching `entryIndex` in `x0`, or `-1` when absent
/// (plan-02 Phase 6). Lazily builds the buckets (calling
/// `_mfb_rt_map_build_buckets`) when the header "ready" flag is 0, so a freshly
/// allocated, copied, or grown map recomputes its index on first lookup. Key
/// equality is byte-wise over `keyLength` bytes — identical to the linear-scan
/// comparison it replaces. Has a one-slot frame to preserve the link register
/// across the build call.
fn lower_map_probe_helper() -> CodeFunction {
    let symbol = MAP_PROBE_SYMBOL;
    const FRAME: usize = 16;
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let ready = format!("{symbol}_ready");
    let notfound = format!("{symbol}_notfound");
    let hloop = format!("{symbol}_hloop");
    let hdone = format!("{symbol}_hdone");
    let ploop = format!("{symbol}_ploop");
    let pnext = format!("{symbol}_pnext");
    let nowrap = format!("{symbol}_nowrap");
    let cloop = format!("{symbol}_cloop");
    let cmatch = format!("{symbol}_cmatch");
    let done = format!("{symbol}_done");
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        // Lazy build if not ready (build preserves x0/x1/x2).
        abi::load_u8("x9", "x0", COLLECTION_OFFSET_BUCKETS_READY),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&ready),
        abi::branch_link(MAP_BUILD_BUCKETS_SYMBOL),
        abi::label(&ready),
        abi::load_u64("x9", "x0", COLLECTION_OFFSET_COUNT),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&notfound),
        abi::load_u64("x14", "x0", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("x16", "Integer", &entry_size),
        abi::multiply_registers("x11", "x14", "x16"),
        abi::add_registers("x11", "x11", "x0"),
        abi::add_immediate("x11", "x11", COLLECTION_HEADER_SIZE), // dataBase
        abi::load_u64("x15", "x0", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers("x12", "x11", "x15"), // bucketBase
        abi::shift_left_immediate("x10", "x14", 1), // bucketCount
        abi::move_immediate("x8", "Integer", FNV1A_PRIME),
        // Hash the query key (x1/x2).
        abi::move_immediate("x16", "Integer", FNV1A_BASIS),
        abi::move_register("x5", "x1"),
        abi::move_register("x6", "x2"),
        abi::label(&hloop),
        abi::compare_immediate("x6", "0"),
        abi::branch_eq(&hdone),
        abi::load_u8("x3", "x5", 0),
        abi::exclusive_or_registers("x16", "x16", "x3"),
        abi::multiply_registers("x16", "x16", "x8"),
        abi::add_immediate("x5", "x5", 1),
        abi::subtract_immediate("x6", "x6", 1),
        abi::branch(&hloop),
        abi::label(&hdone),
        abi::unsigned_divide_registers("x4", "x16", "x10"),
        abi::multiply_subtract_registers("x4", "x4", "x10", "x16"), // slot
        abi::label(&ploop),
        abi::shift_left_immediate("x7", "x4", 3),
        abi::add_registers("x7", "x12", "x7"),
        abi::load_u64("x6", "x7", 0),
        abi::compare_immediate("x6", "0"),
        abi::branch_eq(&notfound),
        abi::subtract_immediate("x13", "x6", 1), // candidate idx
        abi::move_immediate("x16", "Integer", &entry_size),
        abi::multiply_registers("x15", "x13", "x16"),
        abi::add_registers("x15", "x15", "x0"),
        abi::add_immediate("x15", "x15", COLLECTION_HEADER_SIZE), // entry addr
        abi::load_u64("x17", "x15", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::compare_registers("x17", "x2"),
        abi::branch_ne(&pnext),
        abi::load_u64("x16", "x15", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers("x16", "x11", "x16"), // storedPtr
        abi::move_register("x5", "x1"),          // queryCursor
        abi::move_register("x6", "x2"),          // remaining
        abi::label(&cloop),
        abi::compare_immediate("x6", "0"),
        abi::branch_eq(&cmatch),
        abi::load_u8("x3", "x5", 0),
        abi::load_u8("x17", "x16", 0),
        abi::compare_registers("x3", "x17"),
        abi::branch_ne(&pnext),
        abi::add_immediate("x5", "x5", 1),
        abi::add_immediate("x16", "x16", 1),
        abi::subtract_immediate("x6", "x6", 1),
        abi::branch(&cloop),
        abi::label(&cmatch),
        abi::move_register("x0", "x13"),
        abi::branch(&done),
        abi::label(&pnext),
        abi::add_immediate("x4", "x4", 1),
        abi::compare_registers("x4", "x10"),
        abi::branch_lo(&nowrap),
        abi::move_immediate("x4", "Integer", "0"),
        abi::label(&nowrap),
        abi::branch(&ploop),
        abi::label(&notfound),
        abi::move_immediate("x0", "Integer", "0"),
        abi::subtract_immediate("x0", "x0", 1), // -1
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(FRAME),
        abi::return_(),
    ];
    let relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: MAP_BUILD_BUCKETS_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    }];
    CodeFunction {
        name: "runtime.mapProbe".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame: CodeFrame {
            stack_size: FRAME,
            callee_saved: vec![abi::link_register().to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}

/// Lower the standalone string-list sort helper used to give `fs::listDirectory`
/// a deterministic, stable order. It takes a `List OF String` collection pointer
/// in `x0` and sorts its entries in place by ascending byte-wise (UTF-8
/// lexicographic) order using selection sort, swapping only the fixed-size entry
/// records and leaving the data region untouched. It makes no calls.
fn lower_sort_string_list_helper() -> CodeFunction {
    let symbol = SORT_STRING_LIST_SYMBOL;
    // x0  = collection pointer (preserved for the caller)
    // x9  = entries base (collection + header)
    // x10 = count
    // x11 = data region base (entries base + count * entry size)
    // x12 = i (outer index), x13 = min index, x14 = j (inner index)
    // x15 = entry[min] address, x16 = entry[j] address
    // x1..x7 = comparison/swap scratch
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let done = format!("{symbol}_done");
    let outer = format!("{symbol}_outer");
    let inner = format!("{symbol}_inner");
    let inner_done = format!("{symbol}_inner_done");
    let no_swap = format!("{symbol}_no_swap");
    let next_inner = format!("{symbol}_next_inner");
    let cmp_loop = format!("{symbol}_cmp_loop");
    let take_j = format!("{symbol}_take_j");
    let keep_min = format!("{symbol}_keep_min");

    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("x10", "x0", COLLECTION_OFFSET_COUNT),
        abi::compare_immediate("x10", "1"),
        abi::branch_le(&done),
        abi::add_immediate("x9", "x0", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x1", "Integer", &entry_size),
        // data region base = entries base + capacity * entry size (the data
        // region sits past the full lookup capacity for a grown list; §4.2).
        abi::load_u64("x8", "x0", COLLECTION_OFFSET_CAPACITY),
        abi::multiply_registers("x11", "x8", "x1"),
        abi::add_registers("x11", "x9", "x11"),
        abi::move_immediate("x12", "Integer", "0"),
        // outer: for i in 0..count-1
        abi::label(&outer),
        abi::add_immediate("x2", "x12", 1),
        abi::compare_registers("x2", "x10"),
        abi::branch_ge(&done),
        abi::move_register("x13", "x12"),
        abi::move_register("x14", "x2"),
        // inner: for j in i+1..count
        abi::label(&inner),
        abi::compare_registers("x14", "x10"),
        abi::branch_ge(&inner_done),
        // entry[min] -> x15, entry[j] -> x16
        abi::move_immediate("x1", "Integer", &entry_size),
        abi::multiply_registers("x15", "x13", "x1"),
        abi::add_registers("x15", "x9", "x15"),
        abi::multiply_registers("x16", "x14", "x1"),
        abi::add_registers("x16", "x9", "x16"),
        // name pointers: data_base + value_offset ; lengths: value_length
        abi::load_u64("x2", "x15", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("x2", "x11", "x2"),
        abi::load_u64("x3", "x15", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::load_u64("x4", "x16", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("x4", "x11", "x4"),
        abi::load_u64("x5", "x16", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        // compare bytes: x2/x3 = min name ptr/len, x4/x5 = j name ptr/len
        abi::move_immediate("x6", "Integer", "0"),
        abi::label(&cmp_loop),
        // if reached end of min name -> min is prefix; j<min iff j also ended? no: min shorter => min<j => keep_min
        abi::compare_registers("x6", "x3"),
        abi::branch_ge(&keep_min),
        // if reached end of j name -> j shorter, j<min => take_j
        abi::compare_registers("x6", "x5"),
        abi::branch_ge(&take_j),
        abi::load_u8("x7", "x2", 0),
        abi::load_u8("x1", "x4", 0),
        abi::compare_registers("x1", "x7"),
        abi::branch_lo(&take_j),
        abi::branch_hi(&keep_min),
        abi::add_immediate("x2", "x2", 1),
        abi::add_immediate("x4", "x4", 1),
        abi::add_immediate("x6", "x6", 1),
        abi::branch(&cmp_loop),
        abi::label(&take_j),
        abi::move_register("x13", "x14"),
        abi::label(&keep_min),
        abi::label(&next_inner),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&inner),
        abi::label(&inner_done),
        // swap entry[i] and entry[min] if different
        abi::compare_registers("x13", "x12"),
        abi::branch_eq(&no_swap),
        abi::move_immediate("x1", "Integer", &entry_size),
        abi::multiply_registers("x2", "x12", "x1"),
        abi::add_registers("x2", "x9", "x2"),
        abi::multiply_registers("x3", "x13", "x1"),
        abi::add_registers("x3", "x9", "x3"),
    ];
    // swap COLLECTION_ENTRY_SIZE bytes (8 at a time)
    let mut offset = 0;
    while offset < COLLECTION_ENTRY_SIZE {
        instructions.extend([
            abi::load_u64("x4", "x2", offset),
            abi::load_u64("x5", "x3", offset),
            abi::store_u64("x5", "x2", offset),
            abi::store_u64("x4", "x3", offset),
        ]);
        offset += 8;
    }
    instructions.extend([
        abi::label(&no_swap),
        abi::add_immediate("x12", "x12", 1),
        abi::branch(&outer),
        abi::label(&done),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.sortStringList".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// Emit a call to the shared [`VALIDATE_UTF8_SYMBOL`] helper. The byte pointer
/// must already be in `x0` and the byte length in `x1`. The helper returns `0`
/// in `x0` for valid UTF-8 and `1` for invalid; this branches to `error_label`
/// when invalid. Keeping validation in a separate `bl`-reachable function (with
/// its own frame and short-range internal branches) keeps the filesystem read
/// helpers small.
fn emit_call_validate_utf8(
    symbol: &str,
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::branch_link(VALIDATE_UTF8_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: VALIDATE_UTF8_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(error_label),
    ]);
}

/// Lower the standalone UTF-8 validation helper. It takes a byte pointer in `x0`
/// and a byte length in `x1`, and returns `0` in `x0` when the buffer is
/// well-formed UTF-8 or `1` otherwise. It makes no calls, so it needs no stack
/// frame.
fn lower_validate_utf8_helper() -> CodeFunction {
    let symbol = VALIDATE_UTF8_SYMBOL;
    let invalid = format!("{symbol}_invalid");
    let mut instructions = vec![abi::label("entry")];
    if std::env::var("MFB_ASCII").is_ok() {
        let lp = format!("{symbol}_lp");
        let ok = format!("{symbol}_ok");
        instructions.extend([
            abi::move_register("x9", "x0"),
            abi::move_register("x10", "x1"),
            abi::label(&lp),
            abi::compare_immediate("x10", "0"),
            abi::branch_eq(&ok),
            abi::load_u8("x11", "x9", 0),
            abi::compare_immediate("x11", "127"),
            abi::branch_hi(&invalid),
            abi::add_immediate("x9", "x9", 1),
            abi::subtract_immediate("x10", "x10", 1),
            abi::branch(&lp),
            abi::label(&ok),
            abi::move_immediate("x0", "Integer", "0"),
            abi::return_(),
            abi::label(&invalid),
            abi::move_immediate("x0", "Integer", "1"),
            abi::return_(),
        ]);
    } else {
        emit_validate_utf8(symbol, "x0", "x1", &invalid, &mut instructions);
        instructions.extend([
            abi::move_immediate("x0", "Integer", "0"),
            abi::return_(),
            abi::label(&invalid),
            abi::move_immediate("x0", "Integer", "1"),
            abi::return_(),
        ]);
    }
    CodeFunction {
        name: "runtime.validateUtf8".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// Validate that the `len`-byte buffer at `ptr` is well-formed UTF-8, branching
/// to `error_label` on the first invalid sequence. Used by
/// [`lower_validate_utf8_helper`]. Clobbers `x9`-`x14`. `ptr` and `len` are read
/// into scratch registers before any clobber, so they may name `x0`/`x1`.
fn emit_validate_utf8(
    symbol: &str,
    ptr: &str,
    len: &str,
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let pos = "x9";
    let rem = "x10";
    let byte = "x11";
    let cont = "x12";
    let lo = "x13";
    let hi = "x14";

    let loop_start = format!("{symbol}_utf8_loop");
    let done = format!("{symbol}_utf8_done");
    let one = format!("{symbol}_utf8_one");
    let two = format!("{symbol}_utf8_two");
    let three = format!("{symbol}_utf8_three");
    let four = format!("{symbol}_utf8_four");
    let three_ed = format!("{symbol}_utf8_three_ed");
    let three_bounds = format!("{symbol}_utf8_three_bounds");
    let four_f4 = format!("{symbol}_utf8_four_f4");
    let four_bounds = format!("{symbol}_utf8_four_bounds");

    instructions.extend([
        abi::move_register(pos, ptr),
        abi::move_register(rem, len),
        abi::label(&loop_start),
        abi::compare_immediate(rem, "0"),
        abi::branch_eq(&done),
        abi::load_u8(byte, pos, 0),
        abi::compare_immediate(byte, "128"),
        abi::branch_lo(&one),
        abi::compare_immediate(byte, "194"),
        abi::branch_lo(error_label),
        abi::compare_immediate(byte, "224"),
        abi::branch_lo(&two),
        abi::compare_immediate(byte, "240"),
        abi::branch_lo(&three),
        abi::compare_immediate(byte, "245"),
        abi::branch_lo(&four),
        abi::branch(error_label),
        // 1-byte ASCII
        abi::label(&one),
        abi::add_immediate(pos, pos, 1),
        abi::subtract_immediate(rem, rem, 1),
        abi::branch(&loop_start),
        // 2-byte sequence
        abi::label(&two),
        abi::compare_immediate(rem, "2"),
        abi::branch_lo(error_label),
        abi::load_u8(cont, pos, 1),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 2),
        abi::subtract_immediate(rem, rem, 2),
        abi::branch(&loop_start),
        // 3-byte sequence
        abi::label(&three),
        abi::compare_immediate(rem, "3"),
        abi::branch_lo(error_label),
        abi::move_immediate(lo, "Integer", "128"),
        abi::move_immediate(hi, "Integer", "191"),
        abi::compare_immediate(byte, "224"),
        abi::branch_ne(&three_ed),
        abi::move_immediate(lo, "Integer", "160"),
        abi::branch(&three_bounds),
        abi::label(&three_ed),
        abi::compare_immediate(byte, "237"),
        abi::branch_ne(&three_bounds),
        abi::move_immediate(hi, "Integer", "159"),
        abi::label(&three_bounds),
        abi::load_u8(cont, pos, 1),
        abi::compare_registers(cont, lo),
        abi::branch_lo(error_label),
        abi::compare_registers(cont, hi),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 2),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 3),
        abi::subtract_immediate(rem, rem, 3),
        abi::branch(&loop_start),
        // 4-byte sequence
        abi::label(&four),
        abi::compare_immediate(rem, "4"),
        abi::branch_lo(error_label),
        abi::move_immediate(lo, "Integer", "128"),
        abi::move_immediate(hi, "Integer", "191"),
        abi::compare_immediate(byte, "240"),
        abi::branch_ne(&four_f4),
        abi::move_immediate(lo, "Integer", "144"),
        abi::branch(&four_bounds),
        abi::label(&four_f4),
        abi::compare_immediate(byte, "244"),
        abi::branch_ne(&four_bounds),
        abi::move_immediate(hi, "Integer", "143"),
        abi::label(&four_bounds),
        abi::load_u8(cont, pos, 1),
        abi::compare_registers(cont, lo),
        abi::branch_lo(error_label),
        abi::compare_registers(cont, hi),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 2),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 3),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 4),
        abi::subtract_immediate(rem, rem, 4),
        abi::branch(&loop_start),
        abi::label(&done),
    ]);
}

fn finalize_frame(
    instructions: &mut Vec<CodeInstruction>,
    stack_slots: &mut [CodeStackSlot],
    local_stack_size: usize,
    mut callee_saved: Vec<String>,
) -> CodeFrame {
    if instructions.iter().any(|instruction| {
        instruction.op == CodeOp::BranchLink || instruction.op == CodeOp::BranchLinkRegister
    }) && !callee_saved
        .iter()
        .any(|register| register == abi::link_register())
    {
        callee_saved.push(abi::link_register().to_string());
    }
    let save_size = callee_saved.len() * 8;
    let total_stack_size = align(save_size + local_stack_size, 16);
    if total_stack_size == 0 {
        return CodeFrame {
            stack_size: 0,
            callee_saved,
        };
    }

    for slot in stack_slots {
        slot.offset += save_size as i32;
    }
    adjust_stack_instruction_offsets(instructions, save_size);

    let mut prologue = Vec::new();
    prologue.push(abi::subtract_stack(total_stack_size));
    for (index, register) in callee_saved.iter().enumerate() {
        prologue.push(abi::store_u64(register, abi::stack_pointer(), index * 8));
    }

    let insert_at = if instructions
        .first()
        .is_some_and(|instruction| instruction.op == CodeOp::Label)
    {
        1
    } else {
        0
    };
    instructions.splice(insert_at..insert_at, prologue);

    let mut rewritten = Vec::new();
    for instruction in instructions.drain(..) {
        if instruction.op == CodeOp::Ret {
            for (index, register) in callee_saved.iter().enumerate().rev() {
                rewritten.push(abi::load_u64(register, abi::stack_pointer(), index * 8));
            }
            rewritten.push(abi::add_stack(total_stack_size));
            rewritten.push(instruction);
        } else {
            rewritten.push(instruction);
        }
    }
    *instructions = rewritten;

    CodeFrame {
        stack_size: total_stack_size,
        callee_saved,
    }
}

fn adjust_stack_instruction_offsets(instructions: &mut [CodeInstruction], offset_delta: usize) {
    if offset_delta == 0 {
        return;
    }
    for instruction in instructions {
        let stack_relative = instruction
            .fields
            .iter()
            .any(|(name, value)| matches!(*name, "base" | "src") && abi::is_stack_pointer(value));
        if !stack_relative {
            continue;
        }
        for (name, value) in &mut instruction.fields {
            if matches!(*name, "offset" | "imm") {
                if let Ok(offset) = value.parse::<usize>() {
                    *value = (offset + offset_delta).to_string();
                }
            }
        }
    }
}

mod builder_arena_transfer;
mod builder_bits;
mod builder_codegen_primitives;
mod error_constants;
pub(crate) use error_constants::*;
mod types;
pub(crate) use types::*;
mod validation;
mod code_impl;
use code_impl::ToCodeJson;
mod fs_helpers;
use fs_helpers::*;
mod fs_helpers_paths;
use fs_helpers_paths::*;
mod fs_helpers_io;
use fs_helpers_io::*;
mod fs_helpers_atomic;
use fs_helpers_atomic::*;
mod io_helpers;
use io_helpers::*;
mod runtime_helpers;
use runtime_helpers::*;
mod runtime_helpers_thread;
use runtime_helpers_thread::*;
mod data_objects;
use data_objects::*;
mod module_analysis;
use module_analysis::*;
#[cfg(test)]
mod tests;
mod builder_collection_compare;
mod builder_collection_layout;
mod builder_collection_mutate;
mod builder_collection_queries;
mod builder_collection_query;
mod builder_control;
mod builder_conversions;
mod builder_emit_helpers;
mod builder_fixed_math;
mod builder_fs_paths;
mod builder_inplace_assign;
mod builder_math;
mod builder_numeric;
mod builder_search;
mod builder_simd_fixed_math;
mod builder_simd_float_math;
mod builder_simd_math;
mod builder_strings;
mod builder_strings_builtins;
mod builder_strings_package;
mod builder_value_semantics;
mod builder_values;
mod datetime;
mod link_thunk;
mod net;
mod simd_kernel_coeffs;
mod private;
mod term;
mod tls;

fn native_link_error_messages() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        (
            ERR_NATIVE_LINK_LOAD_CODE,
            ERR_NATIVE_LINK_LOAD_MESSAGE,
            ERR_NATIVE_LINK_LOAD_SYMBOL,
        ),
        (
            ERR_NATIVE_LINK_CALL_CODE,
            ERR_NATIVE_LINK_CALL_MESSAGE,
            ERR_NATIVE_LINK_CALL_SYMBOL,
        ),
        (
            ERR_OUT_OF_MEMORY_CODE,
            ERR_ALLOCATION_MESSAGE,
            ERR_ALLOCATION_SYMBOL,
        ),
        // Boundary validations (plan-linker.md §12.3/§12.4).
        (ERR_OVERFLOW_CODE, ERR_OVERFLOW_MESSAGE, ERR_OVERFLOW_SYMBOL),
        (ERR_ENCODING_CODE, ERR_ENCODING_MESSAGE, ERR_ENCODING_SYMBOL),
        (
            ERR_FLOAT_NAN_CODE,
            ERR_FLOAT_NAN_MESSAGE,
            ERR_FLOAT_NAN_SYMBOL,
        ),
        (
            ERR_FLOAT_INF_CODE,
            ERR_FLOAT_INF_MESSAGE,
            ERR_FLOAT_INF_SYMBOL,
        ),
    ]
}

fn standard_error_messages() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        (
            ERR_INVALID_ARGUMENT_CODE,
            ERR_INVALID_ARGUMENT_MESSAGE,
            ERR_INVALID_ARGUMENT_SYMBOL,
        ),
        (
            ERR_INVALID_FORMAT_CODE,
            ERR_INVALID_FORMAT_MESSAGE,
            ERR_INVALID_FORMAT_SYMBOL,
        ),
        (ERR_OVERFLOW_CODE, ERR_OVERFLOW_MESSAGE, ERR_OVERFLOW_SYMBOL),
        (
            ERR_UNDERFLOW_CODE,
            ERR_UNDERFLOW_MESSAGE,
            ERR_UNDERFLOW_SYMBOL,
        ),
        (
            ERR_FLOAT_DOMAIN_CODE,
            ERR_FLOAT_DOMAIN_MESSAGE,
            ERR_FLOAT_DOMAIN_SYMBOL,
        ),
        (
            ERR_FLOAT_NAN_CODE,
            ERR_FLOAT_NAN_MESSAGE,
            ERR_FLOAT_NAN_SYMBOL,
        ),
        (
            ERR_FLOAT_INF_CODE,
            ERR_FLOAT_INF_MESSAGE,
            ERR_FLOAT_INF_SYMBOL,
        ),
        (
            ERR_FLOAT_OVERFLOW_CODE,
            ERR_FLOAT_OVERFLOW_MESSAGE,
            ERR_FLOAT_OVERFLOW_SYMBOL,
        ),
        (
            ERR_OUT_OF_MEMORY_CODE,
            ERR_ALLOCATION_MESSAGE,
            ERR_ALLOCATION_SYMBOL,
        ),
        (
            ERR_INDEX_OUT_OF_RANGE_CODE,
            ERR_INDEX_OUT_OF_RANGE_MESSAGE,
            ERR_INDEX_OUT_OF_RANGE_SYMBOL,
        ),
        (
            ERR_NOT_FOUND_CODE,
            ERR_NOT_FOUND_MESSAGE,
            ERR_NOT_FOUND_SYMBOL,
        ),
        (ERR_TIMEOUT_CODE, ERR_TIMEOUT_MESSAGE, ERR_TIMEOUT_SYMBOL),
        (
            ERR_INTERRUPTED_CODE,
            ERR_INTERRUPTED_MESSAGE,
            ERR_INTERRUPTED_SYMBOL,
        ),
        (ERR_READ_CODE, ERR_READ_MESSAGE, ERR_READ_SYMBOL),
        (
            ERR_ALREADY_EXISTS_CODE,
            ERR_ALREADY_EXISTS_MESSAGE,
            ERR_ALREADY_EXISTS_SYMBOL,
        ),
        (
            ERR_ACCESS_DENIED_CODE,
            ERR_ACCESS_DENIED_MESSAGE,
            ERR_ACCESS_DENIED_SYMBOL,
        ),
        (
            ERR_DIRECTORY_NOT_EMPTY_CODE,
            ERR_DIRECTORY_NOT_EMPTY_MESSAGE,
            ERR_DIRECTORY_NOT_EMPTY_SYMBOL,
        ),
        (
            ERR_CLOSE_FAILED_CODE,
            ERR_CLOSE_FAILED_MESSAGE,
            ERR_CLOSE_FAILED_SYMBOL,
        ),
        (
            ERR_PATH_NOT_FOUND_CODE,
            ERR_PATH_NOT_FOUND_MESSAGE,
            ERR_PATH_NOT_FOUND_SYMBOL,
        ),
        (
            ERR_INVALID_PATH_CODE,
            ERR_INVALID_PATH_MESSAGE,
            ERR_INVALID_PATH_SYMBOL,
        ),
        (
            ERR_RESOURCE_CLOSED_CODE,
            ERR_RESOURCE_CLOSED_MESSAGE,
            ERR_RESOURCE_CLOSED_SYMBOL,
        ),
        (
            ERR_UNSUPPORTED_CODE,
            ERR_UNSUPPORTED_MESSAGE,
            ERR_UNSUPPORTED_SYMBOL,
        ),
        (ERR_OUTPUT_CODE, ERR_OUTPUT_MESSAGE, ERR_OUTPUT_SYMBOL),
        (ERR_EOF_CODE, ERR_EOF_MESSAGE, ERR_EOF_SYMBOL),
        (ERR_ENCODING_CODE, ERR_ENCODING_MESSAGE, ERR_ENCODING_SYMBOL),
        (ERR_INPUT_CODE, ERR_INPUT_MESSAGE, ERR_INPUT_SYMBOL),
        (
            ERR_ADDRESS_INVALID_CODE,
            ERR_ADDRESS_INVALID_MESSAGE,
            ERR_ADDRESS_INVALID_SYMBOL,
        ),
        (
            ERR_ADDRESS_NOT_FOUND_CODE,
            ERR_ADDRESS_NOT_FOUND_MESSAGE,
            ERR_ADDRESS_NOT_FOUND_SYMBOL,
        ),
        (
            ERR_NETWORK_FAILED_CODE,
            ERR_NETWORK_FAILED_MESSAGE,
            ERR_NETWORK_FAILED_SYMBOL,
        ),
        (
            ERR_CONNECTION_CLOSED_CODE,
            ERR_CONNECTION_CLOSED_MESSAGE,
            ERR_CONNECTION_CLOSED_SYMBOL,
        ),
        (
            ERR_READ_TIMEOUT_CODE,
            ERR_READ_TIMEOUT_MESSAGE,
            ERR_READ_TIMEOUT_SYMBOL,
        ),
        (
            ERR_WRITE_TIMEOUT_CODE,
            ERR_WRITE_TIMEOUT_MESSAGE,
            ERR_WRITE_TIMEOUT_SYMBOL,
        ),
        (
            ERR_MESSAGE_TOO_LARGE_CODE,
            ERR_MESSAGE_TOO_LARGE_MESSAGE,
            ERR_MESSAGE_TOO_LARGE_SYMBOL,
        ),
        (
            ERR_TLS_FAILED_CODE,
            ERR_TLS_FAILED_MESSAGE,
            ERR_TLS_FAILED_SYMBOL,
        ),
    ]
}

fn standard_error_message_symbol(message: &str) -> Option<&'static str> {
    standard_error_messages()
        .iter()
        .find_map(|(_, candidate, symbol)| (*candidate == message).then_some(*symbol))
}

fn static_nir_value_type(value: &NirValue, locals: &HashMap<String, String>) -> Option<String> {
    match value {
        NirValue::Const { type_, .. }
        | NirValue::LocalRef { type_, .. }
        | NirValue::Global { type_, .. }
        | NirValue::FunctionRef { type_, .. }
        | NirValue::Capture { type_, .. }
        | NirValue::Constructor { type_, .. }
        | NirValue::UnionExtract { type_, .. }
        | NirValue::WithUpdate { type_, .. }
        | NirValue::ListLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
        NirValue::Local(name) => locals.get(name).cloned(),
        NirValue::Binary {
            op, left, right, ..
        } => static_nir_value_type(left, locals)
            .zip(static_nir_value_type(right, locals))
            .map(|(left_type, right_type)| {
                numeric_binary_result_type(op, &left_type, &right_type).to_string()
            }),
        NirValue::Unary { operand, .. } => static_nir_value_type(operand, locals),
        NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. } => {
            let arg_types = args
                .iter()
                .map(|arg| static_nir_value_type(arg, locals))
                .collect::<Option<Vec<_>>>()?;
            builtins::general::resolve_call(target, &arg_types)
                .map(|call| call.return_type.into_owned())
                .or_else(|| {
                    builtins::collections::resolve_call(target, &arg_types)
                        .map(|call| call.return_type.into_owned())
                })
                .or_else(|| {
                    builtins::strings::resolve_call(target, &arg_types)
                        .map(|call| call.return_type.into_owned())
                })
                .or_else(|| builtins::call_return_type_name(target).map(str::to_string))
        }
        NirValue::ResultIsOk { .. } => Some("Boolean".to_string()),
        NirValue::ResultValue { value } => static_nir_value_type(value, locals)
            .and_then(|type_| type_.strip_prefix("Result OF ").map(str::to_string)),
        NirValue::ResultError { .. } => Some("Error".to_string()),
        NirValue::MemberAccess { target, member } => {
            let target_type = static_nir_value_type(target, locals)?;
            if member == "result" {
                builtins::thread::parent_thread_output(&target_type)
                    .map(|output_type| format!("Result OF {output_type}"))
            } else {
                None
            }
        }
        NirValue::RuntimeCall { .. } | NirValue::UnionWrap { .. } | NirValue::Closure { .. } => {
            None
        }
    }
}

fn collection_type_code(type_: &str) -> Option<usize> {
    match type_ {
        "Nothing" => None,
        "Boolean" => Some(COLLECTION_TYPE_BOOLEAN),
        "Byte" => Some(COLLECTION_TYPE_BYTE),
        "Integer" => Some(COLLECTION_TYPE_INTEGER),
        "Float" => Some(COLLECTION_TYPE_FLOAT),
        "Fixed" => Some(COLLECTION_TYPE_FIXED),
        "String" => Some(COLLECTION_TYPE_STRING),
        _ if type_.starts_with("List OF ") => Some(COLLECTION_TYPE_LIST),
        _ if type_.starts_with("Map OF ") => Some(COLLECTION_TYPE_MAP),
        _ => Some(COLLECTION_TYPE_OBJECT),
    }
}

/// Alignment, in bytes, of a packed collection payload identified by its compact
/// runtime type code. Mirrors `CodeBuilder::collection_payload_alignment` for
/// paths that carry the numeric type code rather than the type name: 8-byte
/// scalars, native collection/object pointers, and inline record/union slot
/// payloads require 8-byte alignment; 1-byte scalars and `String` bytes do not.
fn collection_payload_alignment_for_code(code: usize) -> usize {
    match code {
        COLLECTION_TYPE_INTEGER
        | COLLECTION_TYPE_FLOAT
        | COLLECTION_TYPE_FIXED
        | COLLECTION_TYPE_LIST
        | COLLECTION_TYPE_MAP
        | COLLECTION_TYPE_OBJECT => 8,
        _ => 1,
    }
}

fn local_constant_value_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> Option<NirValue> {
    match value {
        NirValue::Const { .. } => Some(value.clone()),
        NirValue::Local(name) => constants.get(name).cloned(),
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. }
            if target == "typeName" && args.len() == 1 =>
        {
            static_type_name_with_types(&args[0], types).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. }
            if strings_package_static_string_value(target, args, constants, types).is_some() =>
        {
            strings_package_static_string_value(target, args, constants, types).map(|value| {
                NirValue::Const {
                    type_: "String".to_string(),
                    value,
                }
            })
        }
        NirValue::Binary { op, .. } if op == "&" => {
            static_string_value_with_constants(value, constants, types).map(|value| {
                NirValue::Const {
                    type_: "String".to_string(),
                    value,
                }
            })
        }
        _ => None,
    }
}

fn strings_package_static_string_value(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> Option<String> {
    let value = args
        .first()
        .and_then(|arg| static_string_value_with_constants(arg, constants, types))?;
    match target {
        "strings.upper" if args.len() == 1 => Some(crate::unicode_backend::upper(&value)),
        "strings.lower" if args.len() == 1 => Some(crate::unicode_backend::lower(&value)),
        "strings.caseFold" if args.len() == 1 => Some(crate::unicode_backend::case_fold(&value)),
        "strings.normalizeNfc" if args.len() == 1 => {
            Some(crate::unicode_backend::normalize_nfc(&value))
        }
        _ => None,
    }
}

fn binary_may_promote_float_to_fixed(
    op: &str,
    left: &NirValue,
    right: &NirValue,
    types: &HashMap<String, String>,
) -> bool {
    if !matches!(op, "+" | "-" | "*" | "/" | "MOD" | "^") {
        return false;
    }
    let Some(left_type) = static_type_name_with_types(left, types) else {
        return false;
    };
    let Some(right_type) = static_type_name_with_types(right, types) else {
        return false;
    };
    numeric_binary_result_type(op, &left_type, &right_type) == numeric::TYPE_FIXED
        && (left_type == numeric::TYPE_FLOAT || right_type == numeric::TYPE_FLOAT)
}

fn static_primitive_text_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } => match type_.as_str() {
            "Integer" | "Byte" | "Float" | "Fixed" | "String" => Some(value.clone()),
            "Boolean" => match value.as_str() {
                "true" => Some("TRUE".to_string()),
                "false" => Some("FALSE".to_string()),
                _ => None,
            },
            _ => None,
        },
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| static_primitive_text_with_constants(constant, constants)),
        _ => None,
    }
}

fn align(value: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}

fn join_texts(values: &[ValueResult]) -> String {
    values
        .iter()
        .map(|value| value.text.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_collection_type(type_: &str) -> bool {
    type_.starts_with("List OF ") || type_.starts_with("Map OF ")
}

fn list_element_type(type_: &str) -> Option<String> {
    let element = type_.strip_prefix("List OF ")?;
    // A `List OF RES File` element stores and is read as the bare resource borrow
    // (`File`); the `RES` ownership-axis marker is not part of the value (§15.6).
    Some(strip_res_marker(element).to_string())
}

fn map_type_parts(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("Map OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), strip_res_marker(value).to_string()))
}

/// Strip a `RES ` collection-element ownership-axis marker (`RES File` -> `File`).
fn strip_res_marker(type_: &str) -> &str {
    type_.strip_prefix("RES ").unwrap_or(type_)
}

fn callable_return_type(type_: &str) -> Option<String> {
    let (_, returns) = type_.rsplit_once(") AS ")?;
    Some(returns.to_string())
}

fn function_type_parts(type_: &str) -> Option<(Vec<String>, String)> {
    let rest = type_.strip_prefix("FUNC(")?;
    let (params, returns) = rest.split_once(") AS ")?;
    let params = if params.trim().is_empty() {
        Vec::new()
    } else {
        params.split(", ").map(str::to_string).collect()
    };
    Some((params, returns.to_string()))
}

fn parse_map_entry_type(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("MapEntry OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), value.to_string()))
}

fn numeric_binary_result_type(operator: &str, left: &str, right: &str) -> &'static str {
    numeric::binary_result_type(operator, left, right).unwrap_or(numeric::TYPE_INTEGER)
}

fn native_immediate_value(type_: &str, value: &str) -> Result<String, String> {
    match type_ {
        "Nothing" => Ok("0".to_string()),
        "Float" => Ok(value
            .parse::<f64>()
            .map_err(|_| format!("invalid Float constant `{value}`"))?
            .to_bits()
            .to_string()),
        "Fixed" => Ok(fixed_raw_from_decimal(value)?.to_string()),
        _ => Ok(value.to_string()),
    }
}

fn fixed_raw_from_decimal(value: &str) -> Result<i64, String> {
    const SCALE: i128 = 1_i128 << 32;

    let (negative, digits) = value
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, value));
    let (whole, fractional) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fractional.is_empty() {
        return Err(format!("invalid Fixed constant `{value}`"));
    }
    let mut whole_value = if whole.is_empty() {
        0_i128
    } else {
        whole
            .parse::<i128>()
            .map_err(|_| format!("invalid Fixed constant `{value}`"))?
    };
    let mut fractional_value = 0_i128;
    if !fractional.is_empty() {
        let mut denominator = 1_i128;
        for digit in fractional.bytes() {
            if !digit.is_ascii_digit() {
                return Err(format!("invalid Fixed constant `{value}`"));
            }
            fractional_value = fractional_value
                .checked_mul(10)
                .and_then(|current| current.checked_add((digit - b'0') as i128))
                .ok_or_else(|| format!("Fixed constant `{value}` has too many digits"))?;
            denominator = denominator
                .checked_mul(10)
                .ok_or_else(|| format!("Fixed constant `{value}` has too many digits"))?;
        }
        let scaled = fractional_value
            .checked_mul(SCALE)
            .ok_or_else(|| format!("Fixed constant `{value}` has too many digits"))?;
        fractional_value = scaled / denominator;
        if (scaled % denominator) * 2 >= denominator {
            fractional_value += 1;
        }
        if fractional_value == SCALE {
            whole_value += 1;
            fractional_value = 0;
        }
    }
    let raw = whole_value
        .checked_mul(SCALE)
        .and_then(|current| current.checked_add(fractional_value))
        .ok_or_else(|| format!("Fixed constant `{value}` is out of range"))?;
    let raw = if negative { -raw } else { raw };
    i64::try_from(raw).map_err(|_| format!("Fixed constant `{value}` is out of range"))
}

fn join_json<T: ToCodeJson>(values: &[T], indent: usize) -> String {
    values
        .iter()
        .map(|value| value.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

fn json_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
fn checked_arena_used_after_alloc(
    block_base: u64,
    current_offset: u64,
    capacity: u64,
    size: u64,
    align: u64,
) -> Result<(u64, u64), u64> {
    let invalid_argument = ERR_INVALID_ARGUMENT_CODE
        .parse::<u64>()
        .expect("invalid argument code");
    let out_of_memory = ERR_OUT_OF_MEMORY_CODE
        .parse::<u64>()
        .expect("out of memory code");
    if align == 0 || !align.is_power_of_two() {
        return Err(invalid_argument);
    }
    let size = size.max(1);
    let payload_base = block_base
        .checked_add(ARENA_BLOCK_HEADER_SIZE as u64)
        .ok_or(out_of_memory)?;
    let raw = payload_base
        .checked_add(current_offset)
        .ok_or(out_of_memory)?;
    let mask = align - 1;
    let aligned = raw
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or(out_of_memory)?;
    let end = aligned.checked_add(size).ok_or(out_of_memory)?;
    let used = end.checked_sub(payload_base).ok_or(out_of_memory)?;
    if used > capacity {
        return Err(out_of_memory);
    }
    Ok((aligned, used))
}

/// Executable reference model of the per-arena coalescing free-list, mirroring
/// the integer arithmetic of the emitted `arena_alloc` / `arena_insert_free`
/// assembly so the algorithm can be unit-tested without running native code. The
/// list is kept sorted by `start`; `nodes` holds `(start, size)` pairs.
#[cfg(test)]
#[derive(Default, Clone)]
struct FreeListSim {
    nodes: Vec<(u64, u64)>,
}

#[cfg(test)]
impl FreeListSim {
    /// `(size, align)` normalization shared by alloc and free: size 0 → 1, then
    /// round up to the 16-byte granule; align is raised to at least 16 so every
    /// chunk stays 16-aligned.
    fn normalize(size: u64, align: u64) -> (u64, u64) {
        let size = size.max(1);
        let size = (size + (ARENA_MIN_CHUNK - 1)) & !(ARENA_MIN_CHUNK - 1);
        let align = align.max(ARENA_MIN_CHUNK);
        (size, align)
    }

    /// Insert a fresh OS block's usable region (or any chunk) and coalesce.
    fn insert_free(&mut self, ptr: u64, size: u64) {
        let (size, _) = Self::normalize(size, ARENA_MIN_CHUNK);
        // address-ordered insertion slot
        let slot = self.nodes.partition_point(|(start, _)| *start < ptr);
        self.nodes.insert(slot, (ptr, size));
        // coalesce with the node before and after, if adjacent
        if slot + 1 < self.nodes.len() {
            let (nstart, nsize) = self.nodes[slot + 1];
            if ptr + size == nstart {
                self.nodes[slot].1 += nsize;
                self.nodes.remove(slot + 1);
            }
        }
        if slot > 0 {
            let (pstart, psize) = self.nodes[slot - 1];
            if pstart + psize == self.nodes[slot].0 {
                self.nodes[slot - 1].1 += self.nodes[slot].1;
                self.nodes.remove(slot);
            }
        }
    }

    /// First-fit + split. Returns the aligned pointer, or `None` if nothing fits
    /// (the caller would map a new block and retry).
    fn alloc(&mut self, size: u64, align: u64) -> Option<u64> {
        let (size, align) = Self::normalize(size, align);
        let mask = align - 1;
        for index in 0..self.nodes.len() {
            let (start, csize) = self.nodes[index];
            let aligned = (start + mask) & !mask;
            if aligned + size <= start + csize {
                let end = start + csize;
                let front = aligned - start;
                let tail = end - (aligned + size);
                self.nodes.remove(index);
                let mut insert_at = index;
                if front > 0 {
                    self.nodes.insert(insert_at, (start, front));
                    insert_at += 1;
                }
                if tail > 0 {
                    self.nodes.insert(insert_at, (aligned + size, tail));
                }
                return Some(aligned);
            }
        }
        None
    }

    fn free(&mut self, ptr: u64, size: u64) {
        let (size, _) = Self::normalize(size, ARENA_MIN_CHUNK);
        self.insert_free(ptr, size);
    }

    /// Total free bytes and the list length — used to assert coalescing keeps the
    /// list short and never loses or duplicates bytes.
    fn free_bytes(&self) -> u64 {
        self.nodes.iter().map(|(_, size)| *size).sum()
    }

    /// Invariant: strictly ascending, non-overlapping, never two coalescable
    /// (address-adjacent) neighbors left un-merged.
    fn assert_invariants(&self) {
        for window in self.nodes.windows(2) {
            let (astart, asize) = window[0];
            let (bstart, _) = window[1];
            assert!(astart < bstart, "free list not ascending: {:?}", self.nodes);
            assert!(
                astart + asize <= bstart,
                "free list overlaps: {:?}",
                self.nodes
            );
            assert!(
                astart + asize != bstart,
                "adjacent free chunks left un-coalesced: {:?}",
                self.nodes
            );
        }
    }
}

