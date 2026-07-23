use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::arch::ops::CodeOp;
use crate::binary_repr::{self};
use crate::builtins;
use crate::json_string;
use crate::numeric;
use crate::target::shared::abi;

use super::nir::{
    self, NirFunction, NirMatchPattern, NirModule, NirOp, NirRecordUpdate, NirSourceLoc, NirValue,
};
use super::plan::NativePlan;
use super::runtime;

/// The parameters every emitter helper in `shared/code` threads through: who is
/// emitting (`symbol`), what the target can import (`platform_imports`,
/// `platform`), and the two streams being appended to.
///
/// 55 emitter helpers across 14 files spelled these five out longhand, which is
/// most of the 31 `too_many_arguments` warnings and the 54 hand-written
/// suppressions (bug-323 Phase 3). Unlike the `HelperBody` alias, this is NOT
/// neutral by construction — it bundles two `&mut Vec` references — so every
/// conversion is gated on `scripts/artifact-gate.sh` showing zero diffs.
///
/// Rust permits disjoint mutable references to `instructions` and `relocations` through a
/// single `&mut EmitCtx`, so a callee needing both still compiles.
pub(super) struct EmitCtx<'a> {
    pub(super) symbol: &'a str,
    pub(super) platform_imports: &'a HashMap<String, String>,
    pub(super) platform: &'a dyn CodegenPlatform,
    pub(super) instructions: &'a mut Vec<CodeInstruction>,
    pub(super) relocations: &'a mut Vec<CodeRelocation>,
}

/// The body of a lowered runtime helper: its frame, instruction stream,
/// relocations, and stack slots.
///
/// Every `lower_*` helper returns exactly this shape — 115 signatures across 22
/// files spelled it out longhand, which is what made `clippy::type_complexity`
/// fire 113 times (bug-323). A type alias is structurally transparent, so this
/// is a pure renaming: identical `TyKind::Tuple`, identical MIR, no construction
/// or destructuring site touched.
pub(super) type HelperBody = (
    CodeFrame,
    Vec<CodeInstruction>,
    Vec<CodeRelocation>,
    Vec<CodeStackSlot>,
);

/// The body of a platform app-mode hook: frame, instructions, relocations — the
/// same shape as `HelperBody` without stack slots.
///
/// `pub(crate)`, not `pub(super)`: 46 of its 52 sites live outside
/// `crate::target::shared` (the three Linux backends, `linux_gtk`, and
/// `macos_aarch64`), where `pub(super)` would not be nameable. Those files also
/// lack a glob `use super::*`, so they import it by name (bug-323).
///
/// Deliberately only the bare tuple: its sites wrap it in `Option<Result<_,
/// String>>`, plain `Result`, `Option`, and nothing at all, so no single
/// `AppHookResult` alias spans them.
pub(crate) type AppHookBody = (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>);

/// A fallible `HelperBody`. All 113 wrapped sites use `String` as the error
/// type, so one alias covers them; the two sites that return a bare tuple
/// (`runtime_helpers_thread::thread_is_cancelled_helper` and `pad_no_slots`)
/// take `HelperBody` directly and must not be given a `Result` they never had.
pub(super) type HelperResult = Result<HelperBody, String>;

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
    /// Such a local may be observed or mutated through the slot reference, so it is never
    /// loop-promoted (its slot must stay authoritative).
    address_taken_locals: HashSet<String>,
    /// The register-allocation strategy selected for this build (`-regalloc`).
    regalloc_kind: regalloc::RegallocKind,
    /// First scratch-register-exhaustion error recorded by an infallible vreg
    /// minter (`temporary_vreg`/`temporary_fp_vreg`), which cannot return a
    /// `Result` to their many call sites. Only the fixed-pool `-regalloc bump`
    /// oracle can exhaust; the default linear-scan spills and never sets this.
    /// `run_register_allocation` surfaces it as a clean build error instead of
    /// letting the former `.expect` panic (an ICE) escape (bug-70).
    regalloc_error: Option<String>,
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
    /// plan-59-D: the stack slot holding the value **escaping** this scope, set
    /// only while emitting a `RETURN`'s cleanups.
    ///
    /// A resource whose record pointer equals this value is escaping to the
    /// caller and must NOT be closed or reclaimed by the scope it is leaving —
    /// its obligation moves with it. `None` on every other exit path, and that is
    /// load-bearing rather than incidental: on an error exit the resource has not
    /// escaped (§15.6) and must still be closed, and `EXIT`/`CONTINUE` spill no
    /// pending result at all, so a comparison there would read a stale slot.
    escaping_value_slot: Option<usize>,
    error_arena_restore_slot: Option<usize>,
    /// When set, an inline built-in error return (`emit_error_register_return`)
    /// branches to this label instead of returning, leaving the raw `Result` in
    /// the standard tag/value/message registers. Used to make inline conversions
    /// (`toInt`, …) trappable: an inline `TRAP` materializes the raw `Result`
    /// rather than auto-propagating.
    raw_result_capture: Option<String>,
    /// Re-entrancy guard for the inline-error → function-`TRAP` route (bug-03).
    /// Set while `emit_error_register_return` is routing an inline failure to the
    /// enclosing trap; the trap route itself builds an `Error` inline, whose OOM
    /// fallback re-enters `emit_error_register_return` — which must then return to
    /// the caller (a plain `return_()`) rather than recursing into the trap route.
    emitting_error_route: bool,
    /// Re-entrancy guard for the error-block origin funnel (plan-error-block-in-slot
    /// stages 4-5). Set while `emit_park_error_block_from_registers` is building the
    /// owned Error block to park in the current-error slot. Building the block can
    /// itself fail to allocate, whose OOM fallback routes through
    /// `emit_allocation_error_return` → `emit_error_register_return`; that nested
    /// return must stay a loose `RESULT_ERR_TAG` (there is no memory to park a block)
    /// rather than recursing into another park, so the funnel suppresses parking
    /// while this flag is set.
    building_error_block: bool,
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
    /// Stack slots to zero at function entry: owned freeable-flat locals and
    /// resource (`RES`) locals, recorded as each is bound. In a function with a
    /// trap handler, an error can jump to the handler past a not-yet-run `LET`,
    /// so the handler's scope-drop would free/close a slot whose initializer never
    /// executed. Zeroing these slots in the prologue makes the handler's
    /// null-guarded frees and resource closes skip such slots (the per-`LET`
    /// zero-init only guards a binding's *own* trapping initializer, not an earlier
    /// jump past it). Consumed solely as the entry-zeroing list — the actual frees
    /// are driven by `active_cleanups`, so recording a resource slot here does not
    /// turn it into an `arena_free` (bug-246).
    owned_value_slots: Vec<usize>,
    /// Fresh, freeable-flat heap temporaries produced mid-statement that no owner
    /// claimed (plan-25 temp-lifetime fix): a call/constructor result used only as
    /// an argument, arithmetic operand, or discarded `Eval` value. Each is spilled
    /// to its own slot when produced (`lower_value`) and freed when its enclosing
    /// statement finishes (`drop_pending_temps_to`), so a hot loop of
    /// `acc = acc + len(collections::op(...))` frees the interior list every
    /// iteration instead of accumulating it until the function returns. An owning
    /// consumer (`lower_value_owned`, `RETURN`, `StateAssign`, thread-spawn move)
    /// claims its temp so the block is freed exactly once by whoever owns it.
    pending_temp_frees: Vec<PendingTemp>,
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
    /// For backends whose `RegisterModel::math_pool_base` is `None` (no free
    /// physical to pin, e.g. x86), the per-kernel virtual register holding the
    /// SIMD math constant-pool base, keyed by the function symbol it was minted
    /// in (vreg indices are per-function). Lets `emit_load_math_pool_base` and
    /// the `broadcast_*` coefficient loads share one allocator-placed base within
    /// a kernel while re-minting it for the next kernel. Unused (stays `None`) on
    /// backends that pin a physical base.
    math_pool_base_vreg: Option<(String, String)>,
    /// plan-01-vector: in-flight register-native small-vector values. A
    /// `Float2/3/4` produced by a construction or an inlined op that has not yet
    /// crossed a storage/escape boundary lives here as its N per-lane scalar
    /// `Float` `ValueResult`s (each on the scalar-Float carrier), keyed by a
    /// deliberately un-encodable marker location (`VECTOR_NATIVE_MARKER`+index)
    /// carried in the vector's `ValueResult.location`. It is materialized to the
    /// N×8-byte block by `vector_value_as_block` at every boundary; a marker that
    /// leaks to a GP/store site hard-errors at the encoder (fail-loud) rather than
    /// silently miscompiling.
    vector_natives: HashMap<String, Vec<ValueResult>>,
    next_vector_native: u32,
    /// plan-01-vector: small-vector locals promoted to their lanes for their whole
    /// lifetime (no arena block). A binding whose every use is non-materializing
    /// (a member read or an operand to an inlined vector op) never needs a block,
    /// so it lives here as `(type, lanes)` instead of a heap record — killing the
    /// per-binding `arena_alloc`. A read reconstructs a register-native view from
    /// the lanes; the escape analysis (`promotable_vector_locals`) guarantees no
    /// use materializes, and `vector_value_as_block` is the correctness fallback if
    /// one ever does. Gated to non-address-taken, single-assignment bindings.
    promoted_vector_locals: HashMap<String, (String, Vec<ValueResult>)>,
    /// Local names the escape analysis cleared for vector promotion (computed once
    /// per function, consulted at each `Bind`).
    promotable_vector_locals: HashSet<String>,
    /// plan-39 I1: proven **lower bounds** for Integer locals on the current
    /// straight-line path (`name -> C` means `name >= C` here). Established only by
    /// a guard `IF local < K THEN <terminal> END IF` with an empty else, dropped on
    /// any assignment to the local and cleared conservatively at every loop / Match
    /// / Trap and after each `IF`. Used solely to elide the overflow check on
    /// `local - const` when `C - const` provably cannot underflow — sound by
    /// construction (the default keeps the check).
    integer_lower_bounds: HashMap<String, i64>,
    /// plan-39 I1 (upper side): Integer locals known to be **strictly less than
    /// some i64** on the current path (established by a `WHILE local < S` body —
    /// then `local + 1 <= S <= i64::MAX`, so the `add` cannot overflow). Used only
    /// to elide the overflow check on `local + 1`; dropped on any assignment to the
    /// local and cleared at every loop/Match/Trap boundary.
    integer_strict_upper: std::collections::HashSet<String>,
}

#[derive(Clone)]
struct LocalValue {
    type_: String,
    stack_offset: usize,
    constant: Option<NirValue>,
    /// A *reference* local: the stack slot holds a pointer to another binding's
    /// slot rather than the value/block itself. Set for a non-escaping `MUT`
    /// by-ref capture, where reads and writes deref through the slot pointer so
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
    /// The function-level trap error local's fixed stack slot, captured when the
    /// local is allocated (`function_lowering`). Every error route stores the
    /// built `Error` here and the handler reads `e` from here — resolved from
    /// this pinned offset, NOT `self.locals[name]`, because an inline `TRAP(e)`
    /// in the body reuses the shared name `e` and rebinds `self.locals[name]` to
    /// a different slot, which would otherwise desync the route store from the
    /// handler read (bug-148).
    stack_offset: usize,
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
    /// The binding's `STATE` type, when it declared one. Carried from the bind
    /// (the only place it is known) so the drop can size the payload block it
    /// frees — a `STATE` record inlines its `String` fields, so its size is not a
    /// constant (plan-52-B Phase 2).
    state_type: Option<String>,
    /// Whether this resource kind actually uses the per-`File` I/O buffer words
    /// (`BUF_PTR`/`READ_PTR`) — i.e. whether it is a `File`.
    ///
    /// Every resource kind shares the 80-byte record, but ONLY `File`'s open
    /// helpers zero the buffer words after the (PRNG-poisoned) arena alloc.
    /// `net`'s `emit_make_handle` initializes offsets 0/8/16 and leaves 24–72 as
    /// poison; the layout comment calls those words "inert" for non-`File`
    /// resources, which was true only because nothing read them. The drop-path
    /// reclaim made them live, so freeing them unconditionally handed
    /// `arena_free` a poison value and segfaulted every `net::` program during
    /// cleanup (plan-52-B Phase 2).
    has_io_buffers: bool,
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

/// A fresh, freeable-flat heap temporary awaiting a statement-scope free
/// (plan-25 temp-lifetime fix). `location` is the register the value occupied
/// when produced — used to recognise (and exempt) the temp an owning consumer
/// claims; `slot` is the spill holding the block pointer for the eventual
/// `arena_free`.
#[derive(Clone)]
struct PendingTemp {
    type_: String,
    slot: usize,
    location: String,
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
    /// Names of the module's user-declared `RESOURCE` types. Built-in resources
    /// are recognized by `builtins::is_resource_type`; a `RESOURCE Db CLOSE BY …`
    /// is not, so without this set codegen could not tell `Db` from an unknown
    /// type — and `RES x AS Db = <fallible> TRAP` failed to build for want of a
    /// default value on the error path (bug-372).
    resource_names: HashSet<String>,
    /// User-declared resource name -> the call target of its registered
    /// `CLOSE BY` op, for scope-drop cleanup.
    ///
    /// The value is the op's *name* as the importing module routes it, not a
    /// resolved symbol: `resource_cleanup_symbol` looks it up in
    /// `function_symbols`, the same table an explicit `sql::close(db)` call goes
    /// through. That is what lets both spellings the name can take — the dotted
    /// `alias.func` and the bare re-export alias — resolve to the one thunk.
    ///
    /// bug-377: an IMPORTED package's close op takes one of two spellings,
    /// depending on how the package declared it, and only `resolve_closer_symbol`
    /// knows both. `RESOURCE T CLOSE BY link::op` serializes the internal dotted
    /// target, which `merge_packages` identity-prefixes along with the link
    /// function, so it resolves as `<id>.<package>.<alias>.<op>`. A re-exported
    /// `EXPORT FUNC close AS link::op` serializes the BARE alias, and
    /// `ir::package::merge_package` registers it as `<package>.close` with no
    /// identity prefix. Storing one spelling makes the other miss silently.
    ///
    /// bug-374: `resource_cleanup_symbol` resolved the close op only through
    /// `builtins::resource_close_function`, an 8-entry table of the language's
    /// own resources. A `RESOURCE Db CLOSE BY sql::close` missed it, so
    /// `builder_control`'s `else if let Some(symbol)` fell through, no
    /// `ActiveCleanup::Resource` was pushed, and scope exit emitted neither the
    /// close nor the record reclaim — a silent leak of the native handle on
    /// every drop, against the §15 guarantee that the spec's own worked example
    /// (a native resource) relies on.
    ///
    /// Collected here, at model construction, because this is the only layer
    /// that sees both the `RESOURCE` declarations — the module's own and every
    /// imported package's — and the code builder that needs them.
    resource_closers: HashMap<String, String>,
}

/// Adapt a not-yet-vreg shaped helper body (3-tuple, e.g. an app-mode platform
/// hook that manages its own frame) to the 4-tuple shape with an empty
/// spill-slot list, so it can share a `match`/`if` with vreg-migrated helpers.
#[allow(clippy::type_complexity)]
fn pad_no_slots(body: AppHookBody) -> HelperBody {
    (body.0, body.1, body.2, Vec::new())
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
    // Install this platform's backend so the builders, helper routing, and helper
    // finalization dispatch selection + register allocation through it instead of
    // naming AArch64 directly (plan-00-H/I additivity).
    mir::set_backend(platform.backend());
    // Imported packages are now decoded and merged into the project IR upstream
    // (see `lower::lower_project`) and lowered as ordinary functions through this
    // same codegen. The legacy flat binary_repr -> native package bridge is no
    // longer used: there are no separate package exports to lower here — the one
    // thing still read straight off the `.mfp` is each binding's native library
    // locator table (plan-46-C), which is per-target and so cannot be resolved
    // upstream of this per-flavor pass.
    let link_libraries = resolve_link_libraries(module, packages, platform)?;
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
    string_objects.sort_by_key(|(_, left_symbol)| *left_symbol);
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
    // Writable `argc`/`argv` globals for `os::args()` (plan-31-B): filled by the
    // program entry from the values the OS passes in, read back when a later
    // `os::args()` builds its `List OF String`. Emitted only when the module uses
    // `os.args`, so existing programs' data layout is unchanged.
    if module_uses_call(module, "os.args") {
        for symbol in [os::OS_ARGC_GLOBAL_SYMBOL, os::OS_ARGV_GLOBAL_SYMBOL] {
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
    if os::module_uses_env_lock(module) {
        data_objects.push(CodeDataObject {
            symbol: os::OS_ENV_LOCK_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.os_env_lock.v1 { u8 pthread_mutex[64] }".to_string(),
            align: 8,
            size: os::OS_ENV_LOCK_SIZE,
            value: os::os_env_lock_init_hex(platform.target()),
        });
    }
    // Process-global stdin broadcast log (plan-15): one zero-initialized writable
    // structure. Emitted whenever the module uses a stdin read builtin or
    // `thread::openStdIn`/`closeStdIn`; app mode reads a window pipe, not fd 0.
    if !module.build_mode.is_app()
        && module_uses_any_call(
            module,
            &[
                "io.readLine",
                "io.input",
                "io.readChar",
                "io.readByte",
                "io.pollInput",
                "thread.openStdIn",
                "thread.closeStdIn",
            ],
        )
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
        if platform.target().contains("macos") {
            data_objects.extend(tls::macos_tls_data_objects(tls_server));
        } else {
            data_objects.extend(tls::tls_cstring_data_objects(tls_server));
        }
    }
    // The audio backend's read-only data objects (the Linux libasound soname +
    // ALSA symbol names; none on macOS). The backend owns the platform decision
    // and the symbol gate (bug-330).
    data_objects
        .extend(audio::AudioBackend::select(platform).data_objects(&native_plan.runtime_symbols));
    // NIST-EC helpers reference read-only C strings (framework paths + dlsym
    // names) for their load-time dlopen/dlsym.
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| crypto_ec::is_ec_symbol(symbol))
    {
        if platform.target().contains("macos") {
            data_objects.extend(crypto_ec::macos::data_objects());
        } else {
            data_objects.extend(crypto_ec::openssl::data_objects());
        }
    }
    let type_model = TypeModel::from_module_and_packages(module, packages)?;
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
    let skip_entry_arena_destroy = platform.target().starts_with("linux")
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
    // Every writable slot addressed off the pinned arena-state register: the
    // program's own globals, the `LINK`/`FREE` pointer slots, and the `term::`
    // state. The region is PER-ARENA, so a worker thread needs exactly as many
    // slots as the entry frame reserves — the worker's arena block is sized from
    // this same number in `lower_thread_start_helper` (bug-369). Before that, a
    // worker's arena block was only `ARENA_STATE_SIZE` bytes, so every global read
    // in a worker ran off the end of the block into neighbouring arena memory.
    let arena_global_slots = globals_base + link_slot_count + term_state_slots;
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
    // but still share `_mfb_shutdown` for their normal-exit cleanup.
    let register_signal_handlers = module.entry.is_some() && !module.build_mode.is_app();
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
            code_functions.push(platform.emit_program_entry(
                &ProgramEntrySpec {
                    entry_symbol: MACAPP_PROGRAM_SYMBOL,
                    language_entry_symbol: &language_entry_symbol,
                    language_entry_returns: &entry.returns,
                    language_entry_accepts_args: entry.accepts_args,
                    global_initializer_symbol: global_initializer_symbol.as_deref(),
                    link_init_symbol,
                    closure_init_symbol,
                    entry_stack_size,
                    global_slot_count: entry_global_slots,
                    emit_cleanup_failure_audit: module_may_record_cleanup_failure(module),
                    seed_rng: uses_rng,
                    register_signal_handlers,
                    capture_args: module_uses_call(module, "os.args"),
                    // App mode reads the window input pipe, not fd 0 — no broadcast log.
                    subscribe_stdin: false,
                    // The toolkit bootstrap owns `_main`; this body is CALLED by
                    // the worker thread, whose stack has no kernel argv layout
                    // (bug-240).
                    entry_called_as_function: true,
                },
                &platform_imports,
            )?);
            code_functions.extend(app_entry);
            data_objects.extend(platform.app_mode_data_objects(&module.project));
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
                    entry_stack_size,
                    global_slot_count: entry_global_slots,
                    emit_cleanup_failure_audit: module_may_record_cleanup_failure(module),
                    seed_rng: uses_rng,
                    register_signal_handlers,
                    capture_args: module_uses_call(module, "os.args"),
                    subscribe_stdin: module_uses_any_call(
                        module,
                        &[
                            "io.readLine",
                            "io.input",
                            "io.readChar",
                            "io.readByte",
                            "io.pollInput",
                            "thread.openStdIn",
                            "thread.closeStdIn",
                        ],
                    ),
                    // `_main` IS the process entry here, so args arrive however
                    // the platform's raw entry delivers them.
                    entry_called_as_function: false,
                },
                &platform_imports,
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
    code_functions.push(lower_make_error_result());
    code_functions.push(lower_simd_alloc_list());
    code_functions.push(lower_arena_insert_free());
    code_functions.push(lower_arena_free());
    // Entropy fill is always on (plan-01 §6.5): scrub freed chunks and poison
    // fresh blocks. The fill RNG/seed helpers ship with every arena.
    code_functions.push(lower_arena_fill_random());
    code_functions.push(lower_arena_fill_seed());
    code_functions.push(lower_arena_fill_next());
    code_functions.push(lower_arena_destroy(platform)?);
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
            )
        });
    if uses_stdout_buffer {
        code_functions.push(lower_stdout_drain(&platform_imports, platform)?);
    }
    // Stdin broadcast log (plan-15): the shared reader/subscription helpers are
    // emitted whenever the module uses a stdin read builtin or `thread::openStdIn`/
    // `closeStdIn`. `_mfb_rt_stdin_next_byte` replaces the per-byte `read(0,…)` in
    // every stdin read site; subscribe/unsubscribe/recompute back the broadcast
    // registry and the compat main-thread subscription. App mode reads a window
    // pipe, not fd 0, so it is excluded.
    let uses_stdin = !module.build_mode.is_app()
        && runtime_symbols.iter().any(|symbol| {
            matches!(
                symbol.as_str(),
                "_mfb_rt_io_io_readLine"
                    | "_mfb_rt_io_io_input"
                    | "_mfb_rt_io_io_readChar"
                    | "_mfb_rt_io_io_readByte"
                    | "_mfb_rt_io_io_pollInput"
                    | "_mfb_rt_thread_thread_openStdIn"
                    | "_mfb_rt_thread_thread_closeStdIn"
            )
        });
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
        )
    });
    if uses_file_buffer {
        code_functions.push(lower_fs_file_drain(&platform_imports, platform)?);
    }
    if module.entry.is_some() {
        code_functions.push(lower_shutdown(
            uses_term,
            skip_entry_arena_destroy,
            uses_stdout_buffer,
        ));
    }
    if register_signal_handlers {
        code_functions.push(lower_signal_handler(platform)?);
    }
    // The macOS AudioQueue callbacks (plan-33-B §3.2): the output callback when an
    // output stream is built and the input callback when an input stream is,
    // since openOutput/openInput take their addresses. The backend owns the
    // platform decision and the symbol gate (bug-330).
    code_functions.extend(audio::AudioBackend::select(platform).callback_functions(
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
            &module.project,
            ArenaLayout {
                term_state_offset,
                global_slots: arena_global_slots,
            },
            uses_rng,
            &platform_imports,
            platform,
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
            && function.abi_return_ctype == "CPtr"
            && function.return_type == "String";
        let struct_has_cstring_field = function.abi_slots.iter().any(|slot| {
            module
                .link_cstructs
                .iter()
                .find(|c| c.alias == function.alias && c.name == slot.ctype)
                .is_some_and(|c| c.fields.iter().any(|f| f.ctype == "CString"))
        });
        returns_c_string || struct_has_cstring_field
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
    if module_uses_call(module, "fs.pathJoin") {
        code_functions.push(lower_fs_path_join_helper(platform));
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_thread_thread_start")
    {
        code_functions.push(platform.emit_thread_trampoline(
            &platform_imports,
            uses_stdin,
            ArenaInitSymbols {
                link_init: link_init_symbol,
                global_init: global_initializer_symbol.as_deref(),
            },
        )?);
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

    // Unicode property/mapping tables (`strings::upper/lower/caseFold/
    // normalizeNfc/graphemes*` at runtime). Emit iff some generated function
    // actually relocates against a `_mfb_unicode_*` data symbol, unioned with the
    // legacy NIR heuristic so the emitted set can only grow relative to prior
    // builds (no golden churn). The relocation scan is the ground truth: the NIR
    // heuristic could deem a `caseFold(localVar)` static (folded, table skipped)
    // while codegen lowered it at runtime, leaving the `_mfb_unicode_casefold_*`
    // relocation undefined — a deterministic build failure this closes.
    let references_unicode_table = code_functions.iter().any(|function| {
        function.relocations.iter().any(|relocation| {
            relocation.binding == "data" && relocation.to.starts_with("_mfb_unicode_")
        })
    });
    if references_unicode_table || module_uses_unicode_runtime_tables(module) {
        data_objects.extend(unicode_runtime_data_objects());
    }

    // MIR seam for the hand-written runtime helpers (plan-00-F). Builder-emitted
    // functions already round-trip through the neutral MIR at their
    // pre-allocation seam (`run_register_allocation`); the helpers do not pass
    // through the builder, so they enter the MIR pipeline here — the entry
    // sequence, the arena allocator, the error path, the PCG64 RNG, the kernels,
    // and the thread trampoline all flow through the neutral MIR (plan-00-G: the
    // sole code path).
    for function in &mut code_functions {
        mir::route_function_through_mir(function);
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
    bind_deferred_relocation_libraries(&mut code_functions, &platform_imports)?;

    // rv64 `v128` scalarization (plan-99 §6) stages SIMD lanes in a slot region.
    // That region now lives in the **per-thread** arena state (bug-122), addressed
    // off the pinned arena base — no process-global data object is emitted (a
    // global was raced between worker threads running v128 kernels concurrently).

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

fn lower_runtime_helper(
    symbol: &str,
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
    arena_layout: ArenaLayout,
    uses_rng: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let term_state_offset = arena_layout.term_state_offset;
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
        let (frame, instructions, relocations, stack_slots) = match app_term_helper {
            Some(result) => pad_no_slots(result?),
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
            params: Vec::new(),
            returns: spec.abi.returns.to_string(),
            frame,
            stack_slots,
            instructions,
            relocations,
        });
    }
    if crypto_ec::ec_call(spec.call).is_some() {
        let (frame, instructions, relocations, stack_slots) =
            crypto_ec::lower_crypto_ec_helper(spec.call, symbol, platform_imports, platform)?;
        return Ok(CodeFunction {
            name: format!("runtime.{}", spec.call),
            symbol: symbol.to_string(),
            params: Vec::new(),
            returns: spec.abi.returns.to_string(),
            frame,
            stack_slots,
            instructions,
            relocations,
        });
    }
    match spec.call {
        "crypto.randomBytes" => {
            let (frame, instructions, relocations, stack_slots) =
                crypto::lower_crypto_random_bytes_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "datetime.nowNanos" | "datetime.monotonicNanos" | "datetime.localOffset" => {
            let (frame, instructions, relocations, stack_slots) =
                datetime::lower_datetime_helper(spec.call, symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        call if builtins::os::is_os_call(call) => {
            let (frame, instructions, relocations, stack_slots) = os::lower_os_helper(
                spec.call,
                symbol,
                build_mode,
                module_name,
                platform_imports,
                platform,
            )?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "io.print" | "io.write" | "io.printError" | "io.writeError" => {
            let stderr = matches!(spec.call, "io.printError" | "io.writeError");
            let newline = matches!(spec.call, "io.print" | "io.printError");
            // App mode routes io output to the AppKit transcript window
            // (plan-04-macos-app.md §5.4) instead of a file descriptor.
            let (frame, instructions, relocations, stack_slots) = if app_mode {
                pad_no_slots(
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
                        })??,
                )
            } else {
                lower_io_write_helper(
                    symbol,
                    platform_imports,
                    platform,
                    stderr,
                    newline,
                    term_state_offset,
                )?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "io.flush" => {
            // App-mode transcript writes are synchronous (each io write blocks on
            // the main thread via performSelectorOnMainThread), so output is
            // already visible; flush succeeds immediately (plan §5.4).
            let (frame, instructions, relocations, stack_slots) = if app_mode {
                pad_no_slots(platform.emit_app_io_flush_helper(symbol).ok_or_else(|| {
                    format!(
                        "native target '{}' does not support app-mode io helpers",
                        platform.target()
                    )
                })??)
            } else {
                lower_io_flush_helper(symbol, platform_imports, platform)?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "io.isBuffered" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_io_is_buffered_helper(symbol, app_mode)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "io.setBuffered" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_io_set_buffered_helper(symbol, app_mode)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "io.pollInput" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_io_poll_input_helper(symbol, platform_imports, platform, app_mode)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
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
            let (frame, instructions, relocations, stack_slots) =
                if app_mode && spec.call == "io.input" {
                    pad_no_slots(platform.emit_app_io_input_helper(symbol).ok_or_else(|| {
                        format!(
                            "native target '{}' does not support app-mode io helpers",
                            platform.target()
                        )
                    })??)
                } else {
                    lower_io_read_line_helper(
                        symbol,
                        platform_imports,
                        platform,
                        spec.call == "io.input",
                        app_mode,
                        // bug-149: only a console build that also uses `term::`
                        // brackets the line read with a cooked-mode restore.
                        if app_mode { None } else { term_state_offset },
                    )?
                };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "io.readChar" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_io_read_char_helper(symbol, platform_imports, platform, app_mode)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "io.readByte" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_io_read_byte_helper(symbol, platform_imports, platform, app_mode)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
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
            let (frame, instructions, relocations, stack_slots) = if app_mode {
                pad_no_slots(
                    platform
                        .emit_app_io_is_terminal_helper(symbol)
                        .ok_or_else(|| {
                            format!(
                                "native target '{}' does not support app-mode io helpers",
                                platform.target()
                            )
                        })??,
                )
            } else {
                lower_io_is_terminal_helper(symbol, platform_imports, platform, fd)?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.exists" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_exists_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
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
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_kind_exists_helper(symbol, platform_imports, platform, kind)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.currentDirectory" | "fs.tempDirectory" => {
            let (frame, instructions, relocations, stack_slots) =
                if spec.call == "fs.currentDirectory" {
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
                stack_slots,
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
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_path_operation_helper(symbol, platform_imports, platform, operation)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.createDirectories" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_create_directories_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.listDirectory" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_list_directory_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.open" | "fs.openFile" | "fs.openFileNoFollow" => {
            let no_follow = spec.call == "fs.openFileNoFollow";
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_open_helper(symbol, platform_imports, platform, no_follow)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.openWithin" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_open_within_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.createTempFile" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_create_temp_file_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.close" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_close_helper(symbol, platform_imports, platform, true)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.setBuffered" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_set_buffered_helper(symbol)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.isBuffered" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_is_buffered_helper(symbol)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.flush" => {
            let (frame, instructions, relocations, stack_slots) = lower_fs_flush_helper(symbol)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.writeAll" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_write_all_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.writeAllBytes" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_write_all_bytes_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.readText" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_read_text_path_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.readBytes" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_read_bytes_path_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.writeText" | "fs.appendText" => {
            let append = spec.call == "fs.appendText";
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_write_text_path_helper(symbol, platform_imports, platform, append)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.writeBytes" | "fs.appendBytes" => {
            let append = spec.call == "fs.appendBytes";
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_write_bytes_path_helper(symbol, platform_imports, platform, append)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
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
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_atomic_write_helper(symbol, platform_imports, platform, value_kind)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.readAll" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_read_all_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.readAllBytes" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_read_all_bytes_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.readLine" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_read_line_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.eof" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_eof_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.canonicalPath" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_canonical_path_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        "fs.isWithin" => {
            let (frame, instructions, relocations, stack_slots) =
                lower_fs_is_within_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
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
        | "thread.isCancelled"
        | "thread.openStdIn"
        | "thread.closeStdIn" => {
            let (frame, instructions, relocations, stack_slots) = lower_thread_helper(
                symbol,
                spec.call,
                uses_rng,
                arena_layout.global_slots,
                platform_imports,
                platform,
            )?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        call if call.starts_with("net.") => {
            let (frame, instructions, relocations, stack_slots) = match call {
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
                // standard (vreg-allocated) file close helper closes net handles too.
                "net.close" => lower_fs_close_helper(symbol, platform_imports, platform, false)?,
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
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        call if call.starts_with("audio.") => {
            let (frame, instructions, relocations, stack_slots) =
                audio::lower_audio_helper(call, symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        call if call.starts_with("tls.") => {
            let (frame, instructions, relocations, stack_slots) = match call {
                "tls.connect" => tls::lower_tls_connect_helper(symbol, platform_imports, platform)?,
                "tls.listen" => tls::lower_tls_listen_helper(symbol, platform_imports, platform)?,
                "tls.accept" => tls::lower_tls_accept_helper(symbol, platform_imports, platform)?,
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
                "tls.closeListener" => {
                    tls::lower_tls_close_listener_helper(symbol, platform_imports, platform)?
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
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots,
                instructions,
                relocations,
            })
        }
        other => Err(format!(
            "native code plan does not emit runtime call '{other}'"
        )),
    }
}

/// Whether a runtime helper symbol belongs to the TLS **server** side
/// (`tls::listen`/`tls::accept`/the listener close). Gates the server-only
/// data objects and block trampolines so client-only programs keep their
/// exact pre-server native output (plan-06-tls-server.md §1 non-goals).
fn is_tls_server_symbol(symbol: &str) -> bool {
    matches!(
        symbol,
        "_mfb_rt_tls_tls_listen" | "_mfb_rt_tls_tls_accept" | "_mfb_rt_tls_tls_closeListener"
    )
}

/// Call `_mfb_arena_alloc` (size in `x0`, alignment in `x1`) and branch to
/// `fail` when it fails.
///
/// The free-function twin of `CodeBuilder::emit_arena_alloc_call`, for the
/// modules that emit into plain `Vec`s rather than through the builder. It lived
/// in `tls/mod.rs` as a `pub(super)` item that the whole of `code/` already
/// resolved through its glob imports, while four sibling modules each defined a
/// byte-identical private copy that shadowed it (bug-322). Those are deleted;
/// this is the one definition.
pub(super) fn emit_alloc(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    fail: &str,
) {
    instructions.push(abi::branch_link(ARENA_ALLOC_SYMBOL));
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(fail),
    ]);
}

fn internal_branch(from: &str, to: &str) -> CodeRelocation {
    CodeRelocation {
        from: from.to_string(),
        to: to.to_string(),
        kind: RelocIntent::Call,
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
        kind: RelocIntent::Call,
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
    let mut instructions = vec![
        abi::label("entry"),
        // Capture the map pointer (x0) into a vreg immediately: on x86 an
        // incoming param homes in rax/rdx by role, which the div/msub below
        // clobber — destroying it before the final store. As a vreg the
        // allocator keeps it in a safe (or spilled) location. AArch64 unaffected.
        abi::move_register("%v18", abi::ARG[0]),
        // dataBase (v11) = map + HEADER + capacity*ENTRY ; bucketBase (v12) += dataCap.
        abi::load_u64("%v9", "%v18", COLLECTION_OFFSET_COUNT),
        abi::load_u64("%v14", "%v18", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v16", "Integer", &entry_size),
        abi::multiply_registers("%v11", "%v14", "%v16"),
        abi::add_registers("%v11", "%v11", "%v18"),
        abi::add_immediate("%v11", "%v11", COLLECTION_HEADER_SIZE),
        abi::load_u64("%v15", "%v18", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers("%v12", "%v11", "%v15"),
        abi::shift_left_immediate("%v10", "%v14", 1), // bucketCount = 2*capacity
        abi::move_immediate("%v8", "Integer", FNV1A_PRIME),
        // Zero the bucket array.
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&zloop),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_ge(&zdone),
        abi::shift_left_immediate("%v7", "%v13", 3),
        abi::add_registers("%v7", "%v12", "%v7"),
        abi::move_immediate("%v6", "Integer", "0"),
        abi::store_u64("%v6", "%v7", 0),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&zloop),
        abi::label(&zdone),
        // For each entry: hash its key, open-address its index+1.
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&eloop),
        abi::compare_registers("%v13", "%v9"),
        abi::branch_ge(&edone),
        abi::move_immediate("%v16", "Integer", &entry_size),
        abi::multiply_registers("%v14", "%v13", "%v16"),
        abi::add_registers("%v14", "%v14", "%v18"),
        abi::add_immediate("%v14", "%v14", COLLECTION_HEADER_SIZE), // entry addr
        abi::load_u64("%v15", "%v14", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers("%v15", "%v11", "%v15"), // keyPtr
        abi::load_u64("%v17", "%v14", COLLECTION_ENTRY_OFFSET_KEY_LENGTH), // keyLen
        abi::move_immediate("%v16", "Integer", FNV1A_BASIS), // h
        abi::move_register("%v5", "%v15"),
        abi::move_register("%v6", "%v17"),
        abi::label(&hloop),
        abi::compare_immediate("%v6", "0"),
        abi::branch_eq(&hdone),
        abi::load_u8("%v3", "%v5", 0),
        abi::exclusive_or_registers("%v16", "%v16", "%v3"),
        abi::multiply_registers("%v16", "%v16", "%v8"),
        abi::add_immediate("%v5", "%v5", 1),
        abi::subtract_immediate("%v6", "%v6", 1),
        abi::branch(&hloop),
        abi::label(&hdone),
        // slot = h mod bucketCount.
        abi::unsigned_divide_registers("%v4", "%v16", "%v10"),
        abi::multiply_subtract_registers("%v4", "%v4", "%v10", "%v16"),
        abi::label(&ploop),
        abi::shift_left_immediate("%v7", "%v4", 3),
        abi::add_registers("%v7", "%v12", "%v7"),
        abi::load_u64("%v6", "%v7", 0),
        abi::compare_immediate("%v6", "0"),
        abi::branch_eq(&place),
        abi::add_immediate("%v4", "%v4", 1),
        abi::compare_registers("%v4", "%v10"),
        abi::branch_lo(&nowrap),
        abi::move_immediate("%v4", "Integer", "0"),
        abi::label(&nowrap),
        abi::branch(&ploop),
        abi::label(&place),
        abi::add_immediate("%v6", "%v13", 1),
        abi::store_u64("%v6", "%v7", 0),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&eloop),
        abi::label(&edone),
        abi::move_immediate("%v6", "Integer", "1"),
        abi::store_u8("%v6", "%v18", COLLECTION_OFFSET_BUCKETS_READY),
        abi::return_(),
    ];
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.mapBuildBuckets".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
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
    let mut instructions = vec![
        abi::label("entry"),
        // Capture map ptr (x0) and entry index (x1) into vregs before the div/msub
        // below clobber their x86 arg-register homes (rax/rdx). AArch64 unaffected.
        abi::move_register("%v20", abi::ARG[0]),
        abi::move_register("%v21", abi::ARG[1]),
        abi::load_u64("%v14", "%v20", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v16", "Integer", &entry_size),
        abi::multiply_registers("%v11", "%v14", "%v16"),
        abi::add_registers("%v11", "%v11", "%v20"),
        abi::add_immediate("%v11", "%v11", COLLECTION_HEADER_SIZE), // dataBase
        abi::load_u64("%v15", "%v20", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers("%v12", "%v11", "%v15"), // bucketBase
        abi::shift_left_immediate("%v10", "%v14", 1), // bucketCount
        abi::move_immediate("%v8", "Integer", FNV1A_PRIME),
        // entry addr = map + HEADER + index*ENTRY.
        abi::move_immediate("%v16", "Integer", &entry_size),
        abi::multiply_registers("%v14", "%v21", "%v16"),
        abi::add_registers("%v14", "%v14", "%v20"),
        abi::add_immediate("%v14", "%v14", COLLECTION_HEADER_SIZE),
        abi::load_u64("%v15", "%v14", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers("%v15", "%v11", "%v15"),
        abi::load_u64("%v17", "%v14", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::move_immediate("%v16", "Integer", FNV1A_BASIS),
        abi::move_register("%v5", "%v15"),
        abi::move_register("%v6", "%v17"),
        abi::label(&hloop),
        abi::compare_immediate("%v6", "0"),
        abi::branch_eq(&hdone),
        abi::load_u8("%v3", "%v5", 0),
        abi::exclusive_or_registers("%v16", "%v16", "%v3"),
        abi::multiply_registers("%v16", "%v16", "%v8"),
        abi::add_immediate("%v5", "%v5", 1),
        abi::subtract_immediate("%v6", "%v6", 1),
        abi::branch(&hloop),
        abi::label(&hdone),
        abi::unsigned_divide_registers("%v4", "%v16", "%v10"),
        abi::multiply_subtract_registers("%v4", "%v4", "%v10", "%v16"),
        abi::label(&ploop),
        abi::shift_left_immediate("%v7", "%v4", 3),
        abi::add_registers("%v7", "%v12", "%v7"),
        abi::load_u64("%v6", "%v7", 0),
        abi::compare_immediate("%v6", "0"),
        abi::branch_eq(&place),
        abi::add_immediate("%v4", "%v4", 1),
        abi::compare_registers("%v4", "%v10"),
        abi::branch_lo(&nowrap),
        abi::move_immediate("%v4", "Integer", "0"),
        abi::label(&nowrap),
        abi::branch(&ploop),
        abi::label(&place),
        abi::add_immediate("%v6", "%v21", 1),
        abi::store_u64("%v6", "%v7", 0),
        abi::return_(),
    ];
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.mapBucketPut".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
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
    let mut instructions = vec![
        abi::label("entry"),
        // Capture the params (map ptr / key ptr / key len) into vregs up front.
        // The lazy `bl build_buckets` below clobbers the SysV argument registers
        // on x86 (esp. rdx via its div), so x0/x1/x2 must not be read after it.
        // The build call still receives the map implicitly in rdi (unclobbered
        // until then). AArch64 unaffected.
        abi::move_register("%v20", abi::ARG[0]),
        abi::move_register("%v21", abi::ARG[1]),
        abi::move_register("%v22", abi::ARG[2]),
        // Lazy build if not ready.
        abi::load_u8("%v9", "%v20", COLLECTION_OFFSET_BUCKETS_READY),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&ready),
        abi::branch_link(MAP_BUILD_BUCKETS_SYMBOL),
        abi::label(&ready),
        abi::load_u64("%v9", "%v20", COLLECTION_OFFSET_COUNT),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&notfound),
        abi::load_u64("%v14", "%v20", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v16", "Integer", &entry_size),
        abi::multiply_registers("%v11", "%v14", "%v16"),
        abi::add_registers("%v11", "%v11", "%v20"),
        abi::add_immediate("%v11", "%v11", COLLECTION_HEADER_SIZE), // dataBase
        abi::load_u64("%v15", "%v20", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers("%v12", "%v11", "%v15"), // bucketBase
        abi::shift_left_immediate("%v10", "%v14", 1), // bucketCount
        abi::move_immediate("%v8", "Integer", FNV1A_PRIME),
        // Hash the query key (x1/x2).
        abi::move_immediate("%v16", "Integer", FNV1A_BASIS),
        abi::move_register("%v5", "%v21"),
        abi::move_register("%v6", "%v22"),
        abi::label(&hloop),
        abi::compare_immediate("%v6", "0"),
        abi::branch_eq(&hdone),
        abi::load_u8("%v3", "%v5", 0),
        abi::exclusive_or_registers("%v16", "%v16", "%v3"),
        abi::multiply_registers("%v16", "%v16", "%v8"),
        abi::add_immediate("%v5", "%v5", 1),
        abi::subtract_immediate("%v6", "%v6", 1),
        abi::branch(&hloop),
        abi::label(&hdone),
        abi::unsigned_divide_registers("%v4", "%v16", "%v10"),
        abi::multiply_subtract_registers("%v4", "%v4", "%v10", "%v16"), // slot
        abi::label(&ploop),
        abi::shift_left_immediate("%v7", "%v4", 3),
        abi::add_registers("%v7", "%v12", "%v7"),
        abi::load_u64("%v6", "%v7", 0),
        abi::compare_immediate("%v6", "0"),
        abi::branch_eq(&notfound),
        abi::subtract_immediate("%v13", "%v6", 1), // candidate idx
        abi::move_immediate("%v16", "Integer", &entry_size),
        abi::multiply_registers("%v15", "%v13", "%v16"),
        abi::add_registers("%v15", "%v15", "%v20"),
        abi::add_immediate("%v15", "%v15", COLLECTION_HEADER_SIZE), // entry addr
        abi::load_u64("%v17", "%v15", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::compare_registers("%v17", "%v22"),
        abi::branch_ne(&pnext),
        abi::load_u64("%v16", "%v15", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers("%v16", "%v11", "%v16"), // storedPtr
        abi::move_register("%v5", "%v21"),          // queryCursor
        abi::move_register("%v6", "%v22"),          // remaining
        abi::label(&cloop),
        abi::compare_immediate("%v6", "0"),
        abi::branch_eq(&cmatch),
        abi::load_u8("%v3", "%v5", 0),
        abi::load_u8("%v17", "%v16", 0),
        abi::compare_registers("%v3", "%v17"),
        abi::branch_ne(&pnext),
        abi::add_immediate("%v5", "%v5", 1),
        abi::add_immediate("%v16", "%v16", 1),
        abi::subtract_immediate("%v6", "%v6", 1),
        abi::branch(&cloop),
        abi::label(&cmatch),
        abi::move_register(abi::RET[0], "%v13"),
        abi::branch(&done),
        abi::label(&pnext),
        abi::add_immediate("%v4", "%v4", 1),
        abi::compare_registers("%v4", "%v10"),
        abi::branch_lo(&nowrap),
        abi::move_immediate("%v4", "Integer", "0"),
        abi::label(&nowrap),
        abi::branch(&ploop),
        abi::label(&notfound),
        abi::move_immediate(abi::RET[0], "Integer", "0"),
        abi::subtract_immediate(abi::RET[0], abi::RET[0], 1), // -1
        abi::label(&done),
        abi::return_(),
    ];
    let relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: MAP_BUILD_BUCKETS_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.mapProbe".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations,
    }
}

mod builder_arena_transfer;
mod builder_bits;
mod builder_error_emission;
mod builder_exits;
mod builder_owned_cleanup;
mod builder_registers;
mod builder_resource_cleanup;
mod builder_thread_cleanup;
mod error_constants;
pub(crate) use error_constants::*;
mod types;
pub(crate) use types::*;
mod arena;
mod entry;
mod error_result;
mod process_lifecycle;
mod rng_pcg64;
use arena::*;
use error_result::*;
use process_lifecycle::*;
use rng_pcg64::*;
#[cfg(test)]
pub(crate) mod test_support;
mod validation;
pub(crate) use entry::lower_program_entry;
pub(crate) use runtime_helpers::lower_thread_trampoline;
mod codegen_utils;
use codegen_utils::*;
mod code_impl;
use code_impl::ToCodeJson;
mod fs;
use fs::*;
mod float_format;
use float_format::*;
mod io_stdin;
use io_stdin::*;
mod io_stdout;
use io_stdout::*;
mod io_terminal;
use io_terminal::*;
mod stdin_broadcast;
use stdin_broadcast::*;
mod runtime_helpers;
use runtime_helpers::*;
mod runtime_helpers_thread;
use runtime_helpers_thread::*;
mod data_objects;
use data_objects::*;
mod module_analysis;
use module_analysis::*;
mod audio;
mod builder_collection_compare;
mod builder_collection_layout;
use builder_collection_layout::{
    byte_list_block_kind, byte_list_entry_stride, kind2_payload_size, list_block_kind,
    list_element_is_fixed_width, list_entry_stride, push_collection_data_base_from_capacity,
};
mod builder_collection_queries;
mod builder_collection_query;
mod builder_control;
mod builder_conversions;
mod builder_emit_helpers;
mod builder_fixed_math;
mod builder_fs_paths;
mod builder_inplace_assign;
mod builder_math;
mod builder_money;
mod builder_money_math;
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
mod builder_vector_inline;
mod collection_buffer;
mod collection_mutate;
mod native_helpers;

mod crypto;
mod crypto_ec;
mod datetime;
/// Consumer-side native-library locator resolution (plan-46-C). Shared with
/// plan-46-D's vendor copy via `dlopen_name`, so the emitted string and the
/// copied filename cannot diverge.
pub(crate) mod link_locator;
mod link_thunk;
mod list_mutate;
mod map_mutate;
mod net;
mod os;
mod private;
mod simd_kernel_coeffs;
mod term;
mod term_grid;
#[cfg(test)]
mod tests;
pub(crate) mod tls;
mod type_utils;
use builder_vector_inline::{vector_call_is_inlined, vector_field_count};
use type_utils::*;
mod serialization_utils;
use serialization_utils::*;
mod function_lowering;
use function_lowering::*;
mod fma_fusion;
pub(crate) mod mir;
mod peephole;
pub(crate) mod regalloc;
pub(crate) use mir::MirPlan;

/// Resolve every logical `LINK` library this module names to the concrete
/// The thunk symbol a resource's registered `CLOSE BY` op resolves to, or `None`
/// when the name routes to nothing in this module.
///
/// bug-377: two spellings reach here and both must resolve, or a resource is
/// silently never closed.
///
/// * `<id>.<package>.<alias>.<op>` — a `RESOURCE T CLOSE BY link::op` whose
///   internal dotted target `merge_packages` identity-prefixed with the link
///   function it names.
/// * `<package>.<alias-name>` — a re-exported `EXPORT FUNC close AS link::op`.
///   `ir::package::merge_package` qualifies the bare alias with the package name
///   but does NOT identity-prefix it, so the identity-prefixed spelling that
///   `code::validation` built for it misses.
///
/// Try the name as given, then again with a leading identity segment stripped.
/// The identity is a 16-hex-digit content hash, which no package or alias name
/// can be, so the retry cannot capture a legitimately-dotted first segment.
fn resolve_closer_symbol(
    close: &str,
    function_symbols: &HashMap<String, String>,
) -> Option<String> {
    if let Some(symbol) = function_symbols.get(close) {
        return Some(symbol.clone());
    }
    let (identity, rest) = close.split_once('.')?;
    if identity.len() != 16 || !identity.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    function_symbols.get(rest).cloned()
}

/// `source` the declaring binding declared for this build's `(os, arch, libc)`
/// (plan-46-C §4.2).
///
/// Locators come from two places: an imported binding's `.mfp` section 10 (read
/// here, since resolution is per-target and so cannot happen upstream of this
/// per-flavor pass), and the project's own `libraries` section for a project
/// declaring its own `LINK` block. A no-match or ambiguity is a hard build error.
fn resolve_link_libraries(
    module: &nir::NirModule,
    packages: &[PathBuf],
    platform: &dyn CodegenPlatform,
) -> Result<link_locator::LinkLibraries, String> {
    if module.link_functions.is_empty() {
        return Ok(link_locator::LinkLibraries::default());
    }

    let tables = link_locator::LibraryTables::collect(
        packages,
        &module.project,
        module.native_libraries.clone(),
    )?;

    // `platform.target()` is `<os>-<arch>`; the libc comes from the platform,
    // which on Linux is one per flavor (§4.3).
    let target = platform.target();
    let (os, arch) = target.split_once('-').ok_or_else(|| {
        format!("codegen platform target '{target}' is not in `<os>-<arch>` form")
    })?;
    let target = link_locator::LinkTarget {
        os: os.to_string(),
        arch: arch.to_string(),
        libc: platform.libc(),
    };

    let mut linked: Vec<String> = Vec::new();
    for function in &module.link_functions {
        if !linked.contains(&function.library) {
            linked.push(function.library.clone());
        }
    }
    link_locator::LinkLibraries::resolve_all(&tables, &linked, &target)
}

/// Fill in every relocation whose `library` is the deferred placeholder (an
/// empty string) from `platform_imports`, and reject a symbol the platform never
/// declared.
///
/// A backend may leave `library` empty when the correct value depends on
/// something the emitter does not know. The Linux/GTK app-mode emitters do this
/// because the C-library soname depends on the libc flavor, and threading that
/// through ~30 emitter signatures and 33 builder construction sites would
/// duplicate a mapping the native plan already owns (plan-56-A §4.2).
/// Relocations that already carry a library, or carry `None`, are left exactly
/// as they are, so no other backend is affected.
///
/// The `library` field is cosmetic — the linker binds by symbol name — but a
/// wrong or absent one makes artifact debugging lie, and on musl it is the
/// *only* place the libc flavor is observable at all: musl's loader absorbs the
/// glibc compat sonames, so the program runs either way.
fn bind_deferred_relocation_libraries(
    functions: &mut [CodeFunction],
    platform_imports: &HashMap<String, String>,
) -> Result<(), String> {
    for function in functions.iter_mut() {
        for relocation in function.relocations.iter_mut() {
            if relocation.library.as_deref() != Some("") {
                continue;
            }
            match platform_imports.get(&relocation.to) {
                Some(library) => relocation.library = Some(library.clone()),
                None => {
                    return Err(format!(
                        "codegen calls '{}' from '{}', which the platform import list \
                         does not declare",
                        relocation.to, relocation.from
                    ))
                }
            }
        }
    }
    Ok(())
}

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
        (ERR_UNKNOWN_CODE, ERR_UNKNOWN_MESSAGE, ERR_UNKNOWN_SYMBOL),
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
            ERR_RESOURCE_MOVED_CODE,
            ERR_RESOURCE_MOVED_MESSAGE,
            ERR_RESOURCE_MOVED_SYMBOL,
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
            ERR_INVALID_CONTEXT_CODE,
            ERR_INVALID_CONTEXT_MESSAGE,
            ERR_INVALID_CONTEXT_SYMBOL,
        ),
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
        (
            ERR_AUDIO_UNAVAILABLE_CODE,
            ERR_AUDIO_UNAVAILABLE_MESSAGE,
            ERR_AUDIO_UNAVAILABLE_SYMBOL,
        ),
        (
            ERR_AUDIO_DEVICE_CODE,
            ERR_AUDIO_DEVICE_MESSAGE,
            ERR_AUDIO_DEVICE_SYMBOL,
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
