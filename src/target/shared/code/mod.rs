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
    /// Next virtual-register index `allocate_register` will hand out. Virtual
    /// registers are colored to physical registers after the whole function is
    /// lowered (`regalloc::allocate`); the bump-and-reset replay uses
    /// `vreg_eager` (plan-03 Stage A).
    next_vreg: u32,
    /// Per-virtual-register physical register the bump allocator computed
    /// eagerly at allocation time (index == virtual register number). Consumed
    /// by `regalloc::allocate` to reproduce the legacy assignment byte-for-byte.
    vreg_eager: Vec<String>,
    /// Next FP-class temporary register the bump strategy would hand out
    /// (`d0`–`d7`, reset per statement), and the FP virtual-register counter and
    /// per-vreg eager FP physical (plan-03 Stage C).
    next_fp_register: usize,
    next_fp_vreg: u32,
    fp_vreg_eager: Vec<String>,
    /// Float values whose bits in a GPR `ValueResult.location` are *also* resident
    /// in an FP virtual register (the `d`-register a float op left its result in).
    /// Lets a chained float operand be read straight from the `d`-register instead
    /// of round-tripping back through a GPR (plan-03 Stage C). Sound because both
    /// names hold the same value and neither vreg is ever redefined; the allocator
    /// keeps the FP vreg live (or spills/reloads it) wherever it is used.
    float_residents: HashMap<String, String>,
    /// Float locals currently promoted to an FP virtual register for the duration
    /// of an enclosing loop (plan-03 Stage D part 2): name -> the `%fN` holding
    /// the live value. While promoted, reads of the local come from this register
    /// and writes update it, instead of round-tripping its stack slot every
    /// iteration; it is loaded once before the loop and stored back once after.
    promoted_float_locals: HashMap<String, String>,
    /// Locals whose address is taken anywhere in the function (a `LocalRef`).
    /// Such a local may be observed or mutated through the borrow, so it is never
    /// loop-promoted (its slot must stay authoritative).
    address_taken_locals: HashSet<String>,
    /// The register-allocation strategy selected for this build (`-regalloc`).
    regalloc_kind: regalloc::RegallocKind,
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
    code_functions.push(lower_build_error_loc());
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

    builder.run_register_allocation();
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

mod builder_arena_transfer;
mod builder_bits;
mod builder_codegen_primitives;
mod error_constants;
pub(crate) use error_constants::*;
mod types;
pub(crate) use types::*;
mod validation;
mod entry_and_arena;
use entry_and_arena::*;
mod codegen_utils;
use codegen_utils::*;
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
mod builder_pow;
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
mod type_utils;
use type_utils::*;
mod serialization_utils;
use serialization_utils::*;
mod function_lowering;
use function_lowering::*;
mod peephole;
pub(crate) mod regalloc;

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

