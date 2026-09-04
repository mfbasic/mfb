use crate::codegen::io::stdin::lower_stdin_next_byte;
use crate::codegen::io::stdin::lower_stdin_recompute_base;
use crate::codegen::io::stdin::lower_stdin_subscribe;
use crate::codegen::io::stdin::lower_stdin_unsubscribe;
use crate::codegen::io::stdin::stdin_log_data_object;
use crate::types::{CAbiType, ParameterType};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::codegen::app::hook as app;
use crate::codegen::builtins::perf;
use crate::codegen::builtins::vector::builder_simd_float_math;
use crate::codegen::engine::mir;
use crate::codegen::link::locator as link_locator;
use crate::codegen::link::thunk as link_thunk;
use crate::codegen::memory::marshal::construct_helpers::*;
use crate::codegen::term::core as term;
use crate::target::shared::abi;
use crate::target::shared::nir::{self, NirFunction, NirModule, NirSourceLoc, NirValue};
use crate::target::shared::plan::NativePlan;
use crate::target::shared::runtime;

// Deliberate glob (bug-334 B1): `error_constants` is nothing but `pub(crate)
// const`s consumed by essentially every file in the subtree. An explicit list
// would be hundreds of lines of import noise per consumer and would obscure
// rather than reveal the dependency structure — the one legitimate `use x::*;`
// here. Keep it a glob on purpose.
use crate::codegen::builtins::math::*;
use crate::codegen::collection::sort::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::park_error_helper::{
    lower_drop_owned_collection_helper, lower_drop_owned_string_helper, lower_park_error_helper,
};
use crate::codegen::error::result::*;
use crate::codegen::memory::arena::*;
use crate::codegen::os::process::*;
use crate::codegen::string::validate::*;
// (codegen_utils items now come from the engine::util / string::validate /
// collection::sort globs above.)
// Shared syscall / EINTR-retry emission primitives (formerly in `code/fs/io.rs`;
// used by io/net/term, so they stay in the shared code layer, not the migrated
// `fs` package).
pub(crate) mod code_impl;
use crate::codegen::engine::operand::Operand;
use crate::codegen::io::stdout::*;
use crate::codegen::string::format::*;
// Re-exported flat for the relocated `fs` `native/io.rs` buffered-drain emitter
// (its `File` output-buffer drain shares the stdout buffer-sink primitives).
// Re-exported flat for the relocated `io` `native/stdin.rs` read emitters (they
// bracket a line/char/byte read with the shared terminal-mode toggle).
// Re-exported flat for the relocated `io` `native/stdin.rs` read emitters (the
// per-thread stdin broadcast-log byte reader / poll-ready check).
use crate::codegen::engine::analysis::module_uses_call;
use crate::codegen::engine::analysis::*;
use crate::codegen::memory::data::*;
// `audio`'s per-backend device emission moved to `crate::codegen::builtins::audio` (gen_* modules)
// (clean-room registry migration). The `AudioBackend` selector (data objects + macOS
// AudioQueue callbacks) is reached there; runtime-helper bodies flow through the
// `abi_function` path (`lower_abi_function_helper`), like tls/process.
use crate::codegen::builtins::audio::gen_shared::AudioBackend;
use crate::codegen::collection::layout::{recursive_transfer_types, thread_copy_symbol};
// The cross-package presentation-mode gate (app owns `Mode`) stays in the shared
// code layer; re-exported so the migrated `term` package's `abi_function` body
// (`codegen::builtins::term::gen_shared`) can fence its app-mode helpers.
pub(crate) mod builder_emit_helpers;

use crate::codegen::engine::function::*;
use crate::codegen::engine::mir::MirPlan;

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
pub(crate) struct EmitCtx<'a> {
    // `pub(crate)`: the relocated `fs` `native/io.rs` emitters construct an `EmitCtx`
    // to drive the shared `emit_transfer_loop_tail`/EINTR helpers.
    pub(crate) symbol: &'a str,
    pub(crate) platform_imports: &'a HashMap<String, String>,
    pub(crate) platform: &'a dyn CodegenPlatform,
    pub(crate) instructions: &'a mut Vec<CodeInstruction>,
    pub(crate) relocations: &'a mut Vec<CodeRelocation>,
}

/// The body of a lowered runtime helper: its frame, instruction stream,
/// relocations, and stack slots.
///
/// Every `lower_*` helper returns exactly this shape — 115 signatures across 22
/// files spelled it out longhand, which is what made `clippy::type_complexity`
/// fire 113 times (bug-323). A type alias is structurally transparent, so this
/// is a pure renaming: identical `TyKind::Tuple`, identical MIR, no construction
/// or destructuring site touched.
pub(crate) type HelperBody = (
    CodeFrame,
    Vec<CodeInstruction>,
    Vec<CodeRelocation>,
    Vec<CodeStackSlot>,
);

/// A fallible `HelperBody`. All 113 wrapped sites use `String` as the error
/// type, so one alias covers them; the site that returns a bare tuple
/// (`runtime_helpers_thread::thread_is_cancelled_helper`) takes `HelperBody`
/// directly and must not be given a `Result` it never had.
pub(crate) type HelperResult = Result<HelperBody, String>;

pub(crate) struct CodeBuilder<'a> {
    // `pub(crate)`: the relocated `fs` path-string emitters
    // (`crate::codegen::builtins::fs::gen_path_builder`) read these two fields
    // directly, as they did when they lived in this module.
    pub(crate) current_symbol: String,
    pub(crate) function_symbols: &'a HashMap<String, String>,
    pub(crate) functions: &'a HashMap<String, &'a NirFunction>,
    pub(crate) package_return_types: &'a HashMap<String, ParameterType>,
    pub(crate) platform_imports: &'a HashMap<String, String>,
    /// The target platform, threaded so builder-driven lowerings (the `AbiLower`
    /// experiment) can reach per-OS emission / DL resolution through `AbiCtx`
    /// without a separate context object.
    pub(crate) platform: &'a dyn crate::codegen::engine::types::CodegenPlatform,
    /// The native build mode (console vs app), for builder-driven lowerings that
    /// branch on it. Carried alongside `platform` for the same reason.
    pub(crate) build_mode: crate::target::NativeBuildMode,
    pub(crate) globals: &'a HashMap<String, GlobalValue>,
    pub(crate) type_model: TypeModel,
    pub(crate) string_symbols: &'a HashMap<String, String>,
    pub(crate) locals: HashMap<String, LocalValue>,
    pub(crate) instructions: Vec<CodeInstruction>,
    pub(crate) relocations: Vec<CodeRelocation>,
    pub(crate) stack_slots: Vec<CodeStackSlot>,
    pub(crate) used_callee_saved: Vec<String>,
    pub(crate) stack_size: usize,
    pub(crate) next_register: usize,
    /// Next virtual-register index `allocate_register` will hand out. Virtual
    /// registers are colored to physical registers after the whole function is
    /// lowered (`regalloc::allocate`, plan-03).
    pub(crate) next_vreg: u32,
    /// FP virtual-register counter (plan-03 Stage C).
    pub(crate) next_fp_vreg: u32,
    /// Float values whose bits in a GPR `ValueResult.location` are *also* resident
    /// in an FP virtual register (the `d`-register a float op left its result in).
    /// Lets a chained float operand be read straight from the `d`-register instead
    /// of round-tripping back through a GPR (plan-03 Stage C). Sound because both
    /// names hold the same value and neither vreg is ever redefined; the allocator
    /// keeps the FP vreg live (or spills/reloads it) wherever it is used.
    pub(crate) float_residents: HashMap<String, String>,
    /// Float locals currently promoted to an FP virtual register for the duration
    /// of an enclosing loop (plan-03 Stage D part 2): name -> the `%fN` holding
    /// the live value. While promoted, reads of the local come from this register
    /// and writes update it, instead of round-tripping its stack slot every
    /// iteration; it is loaded once before the loop and stored back once after.
    pub(crate) promoted_float_locals: HashMap<String, String>,
    /// Locals whose address is taken anywhere in the function (a `LocalRef`).
    /// Such a local may be observed or mutated through the slot reference, so it is never
    /// loop-promoted (its slot must stay authoritative).
    pub(crate) address_taken_locals: HashSet<String>,
    /// plan-77 M6: locals used as a *value* (read `Local` or address-taken
    /// `LocalRef`) anywhere in the function. A closure binding whose name is NOT
    /// here is only ever a direct invoke target and never escapes, so it is freed
    /// at scope end.
    pub(crate) value_used_locals: HashSet<String>,
    /// plan-86 E: LET bindings `e = collections::get(L, i)` whose result is consumed
    /// READ-ONLY (only a `MATCH` scrutinee) over an immutable container `L`. Such a
    /// `get` returns an aliasing borrow into `L`'s inline element instead of a fresh
    /// `copy_flat_block`, and the binding registers no scope-drop free (the container
    /// owns the element). The copy-skip AND the free-skip are BOTH gated on this set
    /// (a freed borrow is a double-free into the container).
    pub(crate) borrow_get_locals: HashSet<String>,
    /// plan-86 E: set while lowering the initializer of a `borrow_get_locals`
    /// binding, so `materialize_owned_element` returns the aliasing borrow instead of
    /// copying it. Scoped to the one initializer (reset immediately after).
    pub(crate) borrow_get_result: bool,
    /// plan-86 K1: true while lowering a function whose every value-return is a bare
    /// parameter (`function_returns_param_borrow`). Its `RETURN <param>` returns the
    /// argument pointer uncopied (a borrow); callers deep-copy the result only at an
    /// owning store, so the copy moves to the caller's ownership boundary with
    /// identical value semantics. Consistent with the caller side because both key
    /// off the same predicate.
    pub(crate) current_returns_param_borrow: bool,
    /// plan-86 K1: names of every function used as a `FunctionRef` (callback) in the
    /// module. Such a function is invoked through an owning ABI, so it is excluded
    /// from the parameter-passthrough borrow elision (`function_returns_param_borrow`)
    /// — both here (callee) and at every call site (caller) — keeping the two sides
    /// consistent. Shared verbatim across all functions in the build.
    pub(crate) callback_referenced_functions: HashSet<String>,
    /// plan-118-D: record types this module constructs often enough to get their
    /// own `construct.T` function. A construction of a type in here marshals and
    /// calls instead of inlining the allocation, the failure block, the field
    /// stores and a byte-copy loop per `String` field. Computed once per module
    /// from the NIR, so caller and callee cannot disagree about which types are
    /// synthesized. Shared verbatim across all functions in the build.
    pub(crate) synthesized_constructors: HashSet<ParameterType>,
    pub(crate) next_label: usize,
    pub(crate) trap: Option<TrapState>,
    pub(crate) loop_stack: Vec<LoopLabels>,
    pub(crate) active_cleanups: Vec<ActiveCleanup>,
    /// One entry per open `lower_ops` scope, holding the `active_cleanups` length
    /// at that scope's entry. Index 0 is the function body (the scope the trap
    /// handler shares). Used to compute which cleanups an error routed to a trap
    /// must run: only those belonging to inner blocks being exited, never the
    /// function-level locals that stay live (and visible) in the trap body.
    pub(crate) cleanup_scope_starts: Vec<usize>,
    pub(crate) pending_result_slots: Option<PendingResultSlots>,
    /// plan-59-D: the stack slot holding the value **escaping** this scope, set
    /// only while emitting a `RETURN`'s cleanups.
    ///
    /// A resource whose record pointer equals this value is escaping to the
    /// caller and must NOT be closed or reclaimed by the scope it is leaving —
    /// its obligation moves with it. `None` on every other exit path, and that is
    /// load-bearing rather than incidental: on an error exit the resource has not
    /// escaped (§15.6) and must still be closed, and `EXIT`/`CONTINUE` spill no
    /// pending result at all, so a comparison there would read a stale slot.
    pub(crate) escaping_value_slot: Option<usize>,
    /// When set, an inline built-in error return (`emit_error_register_return`)
    /// branches to this label instead of returning, leaving the raw `Result` in
    /// the standard tag/value/message registers. Used to make inline conversions
    /// (`toInt`, …) trappable: an inline `TRAP` materializes the raw `Result`
    /// rather than auto-propagating.
    pub(crate) raw_result_capture: Option<String>,
    /// plan-64-I: names of inline-conversion `CallResult` Result-locals whose
    /// trapped error is provably unused — the paired `Bind err = ResultError`
    /// binds an `err` that the `RECOVER` handler never reads (or there is no
    /// `ResultError` at all). `CallResult` is produced *only* by the inline-
    /// `TRAP` desugar, so such a local's error object is never observed and the
    /// error path can skip building the ErrorLoc + flat `Error` block, keeping
    /// only the tag. Populated once per function before lowering the body.
    pub(crate) trap_discard_error_results: HashSet<String>,
    /// Set transiently (compile-time) while lowering a `Bind` whose Result-local
    /// is in [`trap_discard_error_results`]: it makes `emit_error_register_return`
    /// and `materialize_current_result` emit only a bare tag on the error path.
    pub(crate) raw_result_discard_error: bool,
    /// bug-425: set transiently by the thread-send lowering while it copies a
    /// thread-sendable resource for hand-over. `copy_resource_to_current_arena`
    /// otherwise flags the *source* record `moved|closed` at copy time — before the
    /// enqueue outcome is known — which tombstones the sender's handle even when the
    /// transfer then fails with `ErrTimeout`/`ErrInterrupted`/`ErrResourceClosed`, so
    /// the sender's `TRAP`/scope cleanup can neither use nor close it (a leak the man
    /// page forbids: a failed transfer keeps the resource with the sender). When set,
    /// the copy skips the source flagging; the send helper re-emits it on the enqueue
    /// success branch instead (mirroring `deactivate_moved_resource_arguments`). The
    /// accept side and the nested union/collection copies leave this false and keep
    /// flagging inline (their source is a transient queue record, already garbage).
    pub(crate) suppress_resource_source_flag: bool,
    /// Re-entrancy guard for the inline-error → function-`TRAP` route (bug-03).
    /// Set while `emit_error_register_return` is routing an inline failure to the
    /// enclosing trap; the trap route itself builds an `Error` inline, whose OOM
    /// fallback re-enters `emit_error_register_return` — which must then return to
    /// the caller (a plain `return_()`) rather than recursing into the trap route.
    pub(crate) emitting_error_route: bool,
    /// Re-entrancy guard for the error-block origin funnel (plan-error-block-in-slot
    /// stages 4-5). Set while `emit_park_error_block_from_registers` is building the
    /// owned Error block to park in the current-error slot. Building the block can
    /// itself fail to allocate, whose OOM fallback routes through
    /// `emit_allocation_error_return` → `emit_error_register_return`; that nested
    /// return must stay a loose `RESULT_ERR_TAG` (there is no memory to park a block)
    /// rather than recursing into another park, so the funnel suppresses parking
    /// while this flag is set.
    pub(crate) building_error_block: bool,
    /// Project-relative source file of the function currently being lowered. Used
    /// to build `ErrorLoc.filename` for errors that originate in this function.
    pub(crate) current_file: String,
    /// Source location of the error-originating node currently being lowered.
    /// Updated as `lower_value` descends into call/arithmetic nodes and consulted
    /// when an error is freshly created (overflow, divide-by-zero, helper failure)
    /// so its `ErrorLoc` records the true origin.
    pub(crate) current_loc: NirSourceLoc,
    /// Resource ownership decisions (escape analysis, §15.6) for the function
    /// being lowered, keyed by `RES` binding name. Drives where each resource's
    /// close obligation lives (its own scope, an outer collection's owned-list,
    /// or out via a returned collection).
    pub(crate) resource_owners: HashMap<String, crate::ir::resource_escape::ResOwner>,
    /// Container binding names that own a runtime owned-list (some resource
    /// floats up to their scope). A container is a `List`/`Map` or, since
    /// plan-114-C, a record with a `RES` field — the set is built generically
    /// from the `ResOwner::Float(name)` decisions and reads nothing about the
    /// container's representation, which is why a record binding needs no new
    /// runtime structure.
    pub(crate) owner_containers: HashSet<String>,
    /// Live owned-lists: collection binding name -> head-pointer stack slot.
    pub(crate) owned_list_heads: HashMap<String, usize>,
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
    pub(crate) owned_value_slots: Vec<usize>,
    /// Fresh, freeable-flat heap temporaries produced mid-statement that no owner
    /// claimed (plan-25 temp-lifetime fix): a call/constructor result used only as
    /// an argument, arithmetic operand, or discarded `Eval` value. Each is spilled
    /// to its own slot when produced (`lower_value`) and freed when its enclosing
    /// statement finishes (`drop_pending_temps_to`), so a hot loop of
    /// `acc = acc + len(collections::op(...))` frees the interior list every
    /// iteration instead of accumulating it until the function returns. An owning
    /// consumer (`lower_value_owned`, `RETURN`, `StateAssign`, thread-spawn move)
    /// claims its temp so the block is freed exactly once by whoever owns it.
    pub(crate) pending_temp_frees: Vec<PendingTemp>,
    /// Names of locals whose live buffer is currently being walked by an
    /// enclosing `FOR EACH` (the iterable was a plain `Local`). In-place `set`/
    /// `prepend`/map-`set` that overwrites an *existing* entry's payload would be
    /// observable to such an iterator (it reads each element from the snapshotted
    /// buffer on every step, `lower_for_each`), unlike append which only writes
    /// beyond the snapshot count. The rebuild path repoints the binding at a fresh
    /// buffer the iterator never sees, so in-place overwrite is excluded while the
    /// binding is an active `FOR EACH` iterable (plan-02 §4.1, D1). A `String`
    /// local can never be a `FOR EACH` iterable, so string self-append is exempt.
    pub(crate) for_each_iterable_locals: Vec<String>,
    /// Active `FOR EACH` iterables of the shape `resource.state.field` (bug-430):
    /// the loop snapshots an ALIAS into the STATE collection's inlined buffer, so
    /// an in-place `s.state.field = append(...)` that reallocates+frees that
    /// buffer mid-iteration is a use-after-free. While `(resource, field)` is on
    /// this stack, the in-place STATE grow is excluded and the append falls back
    /// to the (non-freeing) whole-record rebuild, matching the guard
    /// `for_each_iterable_locals` provides for a plain `MUT` list local.
    pub(crate) for_each_iterable_state_fields: Vec<(String, String)>,
    /// Active `FOR EACH` iterables of the shape `local.field` where `local` is a
    /// `MUT` record (bug-430): the loop snapshots an ALIAS into the record's
    /// inlined collection buffer, so an in-place
    /// `local = WITH local { field := append(local.field, …) }` in the body must
    /// NOT reallocate+free that buffer mid-iteration. While `(local, field)` is on
    /// this stack the in-place record grow is excluded and the update falls back
    /// to the whole-record rebuild. The MUT-record analogue of
    /// `for_each_iterable_state_fields`.
    pub(crate) for_each_iterable_record_fields: Vec<(String, String)>,
    /// `String` local name → stack slot tracking the spare payload capacity (bytes
    /// available past the current length) of the local's grown in-place self-append
    /// buffer (plan-02 §4.1 / D9). Zero means "tight" (no spare). The slot is a
    /// frame-local shadow that never escapes: any copy/return/transfer reads only
    /// `len` bytes, freezing the value to the canonical tight `[len][bytes][NUL]`
    /// form, so the spare is invisible outside the buffer. Reset to 0 on any
    /// non-self-append bind/assign of the local and zeroed at function entry.
    pub(crate) string_capacity_slots: HashMap<String, usize>,
    /// For backends whose `RegisterModel::math_pool_base` is `None` (no free
    /// physical to pin, e.g. x86), the per-kernel virtual register holding the
    /// SIMD math constant-pool base, keyed by the function symbol it was minted
    /// in (vreg indices are per-function). Lets `emit_load_math_pool_base` and
    /// the `broadcast_*` coefficient loads share one allocator-placed base within
    /// a kernel while re-minting it for the next kernel. Unused (stays `None`) on
    /// backends that pin a physical base.
    pub(crate) math_pool_base_vreg: Option<(String, String)>,
    /// plan-01-vector: in-flight register-native small-vector values. A
    /// `Float2/3/4` produced by a construction or an inlined op that has not yet
    /// crossed a storage/escape boundary lives here as its N per-lane scalar
    /// `Float` `ValueResult`s (each on the scalar-Float carrier), keyed by a
    /// deliberately un-encodable marker location (`VECTOR_NATIVE_MARKER`+index)
    /// carried in the vector's `ValueResult.location`. It is materialized to the
    /// N×8-byte block by `vector_value_as_block` at every boundary; a marker that
    /// leaks to a GP/store site hard-errors at the encoder (fail-loud) rather than
    /// silently miscompiling.
    pub(crate) vector_natives: HashMap<String, Vec<ValueResult>>,
    pub(crate) next_vector_native: u32,
    /// plan-01-vector: small-vector locals promoted to their lanes for their whole
    /// lifetime (no arena block). A binding whose every use is non-materializing
    /// (a member read or an operand to an inlined vector op) never needs a block,
    /// so it lives here as `(type, lanes)` instead of a heap record — killing the
    /// per-binding `arena_alloc`. A read reconstructs a register-native view from
    /// the lanes; the escape analysis (`promotable_vector_locals`) guarantees no
    /// use materializes, and `vector_value_as_block` is the correctness fallback if
    /// one ever does. Gated to non-address-taken, single-assignment bindings.
    pub(crate) promoted_vector_locals: HashMap<String, (ParameterType, Vec<ValueResult>)>,
    /// Local names the escape analysis cleared for vector promotion (computed once
    /// per function, consulted at each `Bind`).
    pub(crate) promotable_vector_locals: HashSet<String>,
    /// plan-39 I1: proven **lower bounds** for Integer locals on the current
    /// straight-line path (`name -> C` means `name >= C` here). Established only by
    /// a guard `IF local < K THEN <terminal> END IF` with an empty else, dropped on
    /// any assignment to the local and cleared conservatively at every loop / Match
    /// / Trap and after each `IF`. Used solely to elide the overflow check on
    /// `local - const` when `C - const` provably cannot underflow — sound by
    /// construction (the default keeps the check).
    pub(crate) integer_lower_bounds: HashMap<String, i64>,
    /// plan-39 I1 (upper side): Integer locals known to be **strictly less than
    /// some i64** on the current path (established by a `WHILE local < S` body —
    /// then `local + 1 <= S <= i64::MAX`, so the `add` cannot overflow). Used only
    /// to elide the overflow check on `local + 1`; dropped on any assignment to the
    /// local and cleared at every loop/Match/Trap boundary.
    pub(crate) integer_strict_upper: std::collections::HashSet<String>,
    /// plan-86 G1: value expr of each synthetic `$for_end*` / `$for_step*` binding
    /// the IR emits for a `FOR` bound/step, so the loop's `end`/`step` (which reach
    /// `lower_numeric_for` as `Local($for_endN)`) can be resolved back to their
    /// `n - k` / `1` exprs. These synthetics are write-once and unique per loop.
    pub(crate) for_bound_expr: HashMap<String, NirValue>,
    /// plan-86 G1: `n -> (L, depth)` for a binding `LET n = len(L)` (both locals).
    /// Lets a loop bound written as `n - k` resolve to `len(L) - k`. Dropped when
    /// `n` or `L` is reassigned. `depth` is `enclosing_loop_reassigned.len()` when
    /// the fact was recorded (bug-495): a loop entered AFTER that point may run a
    /// reassignment of `L`/`n` on its back edge and re-enter an inner `FOR` with the
    /// fact stale, so `recognize_provable_index` declines when any enclosing set at
    /// index `>= depth` names `L` or `n`. A loop that CONTAINS the `LET` re-runs it
    /// every iteration and cannot stale the fact.
    pub(crate) len_of_local: HashMap<String, (String, usize)>,
    /// bug-495: the reassigned-locals set (`collect_reassigned_locals`) of every
    /// loop body currently being lowered, outermost first — pushed/popped by
    /// `lower_loop_body`. Lowering is one linear pass, so a reassignment placed
    /// AFTER an inner loop in program order is invalidated only after that loop is
    /// already emitted; this stack is how the inner loop's proof sees it.
    pub(crate) enclosing_loop_reassigned: Vec<HashSet<String>>,
    /// plan-86 G1: induction var `i -> (L, headroom k)` for a `FOR i = 0 TO
    /// len(L) - k` loop (`k >= 1`, step 1) where BOTH `i` and `L` are provably NOT
    /// reassigned anywhere in the loop body. Then `get/set(L, i)` is in-range
    /// (`i <= len-k < len`), and `get/set(L, i+1)` too when `k >= 2` — so the
    /// per-access bounds check is elided. Set for the duration of the body and
    /// removed after; because `i`/`L` are unmodified across the WHOLE body, the fact
    /// stays sound throughout (no mid-body clear needed). UNSOUND elision = silent
    /// OOB — gated by the whole-body-unmodified proof AND mandatory negative fixtures.
    pub(crate) provable_index_locals: HashMap<String, (String, i64)>,
}

impl<'a> CodeBuilder<'a> {
    /// A builder for a **synthesized** function — one the compiler emits that no
    /// NIR function corresponds to: a builtin `FunctionRef` wrapper, or one of
    /// plan-118's out-of-line `runtime.*` helpers.
    ///
    /// Everything not passed is the empty/default state a fresh function starts
    /// in. Factored out because three sites already spelled this ~60-field
    /// literal by hand, and plan-118 adds more: a new `CodeBuilder` field that a
    /// synthesized function forgets to initialize is a silent miscompile in
    /// exactly the code paths no NIR fixture covers.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn for_synthetic_function(
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
    ) -> Self {
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
            next_vreg: 0,
            next_fp_vreg: 0,
            float_residents: HashMap::new(),
            promoted_float_locals: HashMap::new(),
            address_taken_locals: HashSet::new(),
            value_used_locals: HashSet::new(),
            borrow_get_locals: HashSet::new(),
            borrow_get_result: false,
            current_returns_param_borrow: false,
            callback_referenced_functions: HashSet::new(),
            synthesized_constructors: HashSet::new(),
            next_label: 0,
            trap: None,
            loop_stack: Vec::new(),
            active_cleanups: Vec::new(),
            cleanup_scope_starts: Vec::new(),
            pending_result_slots: None,
            escaping_value_slot: None,
            raw_result_capture: None,
            trap_discard_error_results: HashSet::new(),
            raw_result_discard_error: false,
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
            for_each_iterable_locals: Vec::new(),
            for_each_iterable_state_fields: Vec::new(),
            for_each_iterable_record_fields: Vec::new(),
            string_capacity_slots: HashMap::new(),
            math_pool_base_vreg: None,
            vector_natives: HashMap::new(),
            next_vector_native: 0,
            promoted_vector_locals: HashMap::new(),
            promotable_vector_locals: HashSet::new(),
            integer_lower_bounds: HashMap::new(),
            integer_strict_upper: std::collections::HashSet::new(),
            for_bound_expr: HashMap::new(),
            len_of_local: HashMap::new(),
            provable_index_locals: HashMap::new(),
            enclosing_loop_reassigned: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct LocalValue {
    pub(crate) type_: ParameterType,
    pub(crate) stack_offset: usize,
    pub(crate) constant: Option<NirValue>,
    /// A *reference* local: the stack slot holds a pointer to another binding's
    /// slot rather than the value/block itself. Set for a non-escaping `MUT`
    /// by-ref capture, where reads and writes deref through the slot pointer so
    /// the live parent binding is observed and updated. False for every ordinary
    /// binding.
    pub(crate) by_ref: bool,
}

#[derive(Clone)]
pub(crate) struct GlobalValue {
    pub(crate) type_: ParameterType,
    pub(crate) offset: usize,
}

#[derive(Clone)]
pub(crate) struct ValueResult {
    pub(crate) type_: ParameterType,
    pub(crate) location: Operand,
    pub(crate) text: String,
    /// The `NirValue` this result was lowered from, preserved so a **pre-lowered**
    /// `abi_inline` body can still run any NIR-structural analysis the former
    /// self-lowering body did (bounds-check elision off `Local`/`Local + 1`,
    /// float-finiteness re-check off an arithmetic node, constant-string folding, …)
    /// — the value is pre-lowered uniformly, but no structural detail is lost. `None`
    /// only for values not produced from a single source node (a synthesized result).
    pub(crate) origin: Option<crate::target::shared::nir::NirValue>,
}

pub(crate) struct TrapState {
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) in_trap_body: bool,
    /// The function-level trap error local's fixed stack slot, captured when the
    /// local is allocated (`function_lowering`). Every error route stores the
    /// built `Error` here and the handler reads `e` from here — resolved from
    /// this pinned offset, NOT `self.locals[name]`, because an inline `TRAP(e)`
    /// in the body reuses the shared name `e` and rebinds `self.locals[name]` to
    /// a different slot, which would otherwise desync the route store from the
    /// handler read (bug-148).
    pub(crate) stack_offset: usize,
}

#[derive(Clone)]
pub(crate) struct LoopLabels {
    pub(crate) kind: crate::ast::LoopKind,
    pub(crate) continue_label: String,
    pub(crate) exit_label: String,
    pub(crate) cleanup_depth: usize,
}

#[derive(Clone)]
pub(crate) struct ThreadCleanup {
    pub(crate) name: String,
    pub(crate) symbol: String,
}

#[derive(Clone)]
pub(crate) struct ResourceCleanup {
    pub(crate) name: String,
    pub(crate) symbol: String,
    /// The binding's `STATE` type, when it declared one. Carried from the bind
    /// (the only place it is known) so the drop can size the payload block it
    /// frees — a `STATE` record inlines its `String` fields, so its size is not a
    /// constant (plan-52-B Phase 2).
    pub(crate) state_type: Option<ParameterType>,
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
    pub(crate) has_io_buffers: bool,
}

#[derive(Clone)]
pub(crate) struct ResourceUnionCleanup {
    pub(crate) name: String,
    /// `(tag, close_symbol)` per active variant; drop reads the union tag and
    /// calls the matching close op on the variant's resource pointer.
    pub(crate) variants: Vec<(usize, String)>,
    /// The union's uniform `STATE` type, when it declared one (plan-74). Carried
    /// from the bind so the tag-dispatched drop can free the active variant
    /// record's STATE block after the close — mirrors `ResourceCleanup.state_type`.
    pub(crate) state_type: Option<ParameterType>,
}

/// A per-scope runtime owned-list (§15.6): the close obligations for resources
/// whose ownership floated up to this scope from inner blocks. The list is a
/// nul-terminated singly linked list of `{record_ptr, next}` nodes, with its
/// head in `head_slot`; draining walks it head-first (most-recent first) and
/// closes each record once (the close is closed-flag idempotent).
/// How each node of an owned-list is closed at drain: a concrete resource
/// element has one registered close op; a resource-union element (bug-429) is
/// tag-dispatched to its active variant's close op, then its uniform STATE block
/// is freed — the same drop a lone `RES u AS Union STATE S` binding runs.
#[derive(Clone)]
pub(crate) enum OwnedListDrop {
    /// A single registered close op applied to each node's record pointer.
    Concrete(String),
    /// A resource-union element: `(tag, close_symbol)` per variant, plus the
    /// union's uniform `STATE` type (when it has one) to free after the close.
    Union {
        variants: Vec<(usize, String)>,
        state_type: Option<ParameterType>,
    },
}

#[derive(Clone)]
pub(crate) struct OwnedListCleanup {
    /// The owning collection binding's name (for transfer-on-return lookup).
    pub(crate) name: String,
    /// Stack offset of the list head pointer (0 when empty).
    pub(crate) head_slot: usize,
    /// How each node's resource is closed (concrete close op vs union dispatch).
    pub(crate) drop: OwnedListDrop,
}

/// An owned, non-escaping flat value freed at scope-drop (plan-01 Phase 5 /
/// plan-02 Phase 8). Because every non-resource value is a pointer-free flat
/// block and copy-insertion (`lower_value_owned`) makes each owner's block
/// unaliased, a single `arena_free(ptr, size)` reclaims the whole value; the
/// size is recomputed from the static type at drop via
/// `emit_inlined_block_size_from_ptr_slot`.
#[derive(Clone)]
pub(crate) struct OwnedValueCleanup {
    /// Static type of the bound value (drives the runtime size computation).
    pub(crate) type_: ParameterType,
    /// Stack offset of the binding's slot (holds the block pointer).
    pub(crate) stack_offset: usize,
    /// plan-77 M6: when `Some`, this is a non-escaping closure binding rather than
    /// a flat value; the slot holds the closure object pointer and the vec is the
    /// static types of its captures. The drop frees the object, its env block, and
    /// each freeable-flat capture (skipping by-value scalars/floats) instead of a
    /// single flat `arena_free`.
    pub(crate) closure_captures: Option<Vec<ParameterType>>,
}

/// A fresh, freeable-flat heap temporary awaiting a statement-scope free
/// (plan-25 temp-lifetime fix). `location` is the register the value occupied
/// when produced — used to recognise (and exempt) the temp an owning consumer
/// claims; `slot` is the spill holding the block pointer for the eventual
/// `arena_free`.
#[derive(Clone)]
pub(crate) struct PendingTemp {
    pub(crate) type_: ParameterType,
    pub(crate) slot: usize,
    pub(crate) location: Operand,
}

#[derive(Clone)]
pub(crate) enum ActiveCleanup {
    Thread(ThreadCleanup),
    Resource(ResourceCleanup),
    ResourceUnion(ResourceUnionCleanup),
    OwnedList(OwnedListCleanup),
    OwnedValue(OwnedValueCleanup),
}

#[derive(Clone, Copy)]
pub(crate) struct PendingResultSlots {
    pub(crate) value: usize,
    pub(crate) tag: usize,
    pub(crate) message: usize,
    // Stashed error-source pointer (`ErrorLoc`) for the pending error result.
    pub(crate) source: usize,
}

#[derive(Clone, Copy)]
pub(crate) enum ExitDestination {
    Return,
    Trap,
}

#[derive(Clone)]
pub(crate) struct PayloadSlot {
    pub(crate) slot: usize,
    pub(crate) type_: ParameterType,
}

#[derive(Clone)]
pub(crate) struct CollectionValueSlot {
    pub(crate) key: Option<PayloadSlot>,
    pub(crate) value: PayloadSlot,
}

pub(crate) struct CollectionTypeLayout {
    pub(crate) kind: usize,
    pub(crate) key_type_code: usize,
    pub(crate) value_type_code: usize,
}

#[derive(Clone)]
pub(crate) struct TypeModel {
    /// plan-111-C: keyed by `(enum TYPE, member name)`. The member name is a
    /// member, not a type, so it stays a `String`.
    pub(crate) enum_members: HashMap<(ParameterType, String), usize>,
    /// plan-111-C: keyed by the record TYPE. The `String` in the value is a
    /// FIELD name.
    pub(crate) record_fields: HashMap<ParameterType, Vec<(String, ParameterType)>>,
    pub(crate) union_names: HashSet<ParameterType>,
    /// A union VARIANT type -> the union type that declares it. Both halves
    /// are types (`builder_value_semantics.rs` looks a variant up and then reads
    /// the resulting union's tag).
    pub(crate) union_variants: HashMap<ParameterType, ParameterType>,
    pub(crate) union_variant_unions: HashMap<ParameterType, HashSet<ParameterType>>,
    pub(crate) union_variant_tags: HashMap<ParameterType, usize>,
    pub(crate) union_variant_fields: HashMap<ParameterType, Vec<(String, ParameterType)>>,
    /// Names of the module's user-declared `RESOURCE` types. Built-in resources
    /// are recognized by `builtins::is_resource_type`; a `RESOURCE Db CLOSE BY …`
    /// is not, so without this set codegen could not tell `Db` from an unknown
    /// type — and `RES x AS Db = <fallible> TRAP` failed to build for want of a
    /// default value on the error path (bug-372).
    pub(crate) resource_names: HashSet<ParameterType>,
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
    /// plan-111-C: keyed by the resource TYPE. The VALUE stays a `String` — it
    /// is a routing name resolved through `resolve_closer_symbol`, which is
    /// where bug-374 and bug-377 live, not a type.
    pub(crate) resource_closers: HashMap<ParameterType, String>,
}

/// plan-67-B: the single predicate gating all runtime perf-tracking injection.
/// It is `true` exactly when the **compiler** is built with `--cfg perf`
/// (`RUSTFLAGS="--cfg perf" cargo build`, in debug or release); every ordinary
/// build returns `false`, so the entire golden/acceptance path (plan-67-A drives
/// it from a perf-free build) is byte-identical to pre-plan-67 HEAD. The
/// macOS-only and has-entry conditions are applied at each *use* site (the force
/// in `plan::symbols::runtime_symbols` and the entry/exit injection), so this
/// predicate stays a pure build-flag question.
pub(crate) fn perf_injection_enabled() -> bool {
    cfg!(perf)
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
    let string_symbols = crate::trace::timed("string symbols", || string_symbols(module));
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
    }
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
        data_objects.push(crate::codegen::runtime::canvas::graphics_state_data_object());
        data_objects.push(CodeDataObject {
            symbol: CANVAS_SCENE_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.canvas_scene.v1 { u64 revision, count, items, hashes, \
                     layers, layerCount }"
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
            layout: "mfb.runtime.canvas_fonts.v1 { u64 handle, block }[16]".to_string(),
            align: 8,
            size: CANVAS_FONT_TABLE_BYTES,
            value: "00".repeat(CANVAS_FONT_TABLE_BYTES),
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
    // plan-67-B: perf-tracking region base (an 8-byte writable global, mirroring
    // the arena global above) and the table-header string. Emitted only for a
    // `--cfg perf`-built macOS entry module — the exact gate under which the
    // entry/exit injection and the `_mfb_rt_perf_*` bodies are emitted — so
    // perf-free, non-macOS, and non-entry plans stay byte-identical to pre-plan-67 HEAD.
    if perf_injection_enabled() && module.entry.is_some() && module.target == "macos-aarch64" {
        data_objects.push(CodeDataObject {
            symbol: PERF_STATE_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.perf_state.v1 { u64 regionBase }".to_string(),
            align: 8,
            size: 8,
            value: "0000000000000000".to_string(),
        });
        data_objects.push(string_data_object(
            PERF_HEADER_SYMBOL,
            "name count avg median min max sum\n".to_string(),
        ));
        // plan-67-C: the whole-program span's name object (one object per unique
        // name; plan-67-F adds one per instrumented arena region).
        data_objects.push(string_data_object(
            PERF_NAME_PROGRAM_SYMBOL,
            "program".to_string(),
        ));
        // plan-67-D: pseudo-name objects for the diagnostic counter rows.
        data_objects.push(string_data_object(
            PERF_NAME_MISMATCH_SYMBOL,
            "mismatch".to_string(),
        ));
        data_objects.push(string_data_object(
            PERF_NAME_OVERFLOW_SYMBOL,
            "overflow".to_string(),
        ));
        // plan-67-F: name objects for the instrumented arena regions.
        data_objects.push(string_data_object(
            PERF_NAME_MFB_ALLOC_SYMBOL,
            "mfb_alloc".to_string(),
        ));
        data_objects.push(string_data_object(
            PERF_NAME_MFB_FREE_SYMBOL,
            "mfb_free".to_string(),
        ));
    }
    // Writable `argc`/`argv` globals for `os::args()` (plan-31-B): filled by the
    // program entry from the values the OS passes in, read back when a later
    // `os::args()` builds its `List OF String`. Emitted only when the module uses
    // `os.args`, so existing programs' data layout is unchanged.
    if module_uses_call(module, "os.args") {
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
    let type_model = crate::trace::timed("type model", || {
        TypeModel::from_module_and_packages(module, packages)
    })?;
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
    // Linux defers arena teardown while worker threads may still be live; macOS
    // does not. 47-H must deliberately decide this for Windows (§3.2 flags the
    // silent `false` a raw `starts_with("linux")` would have handed it).
    let family_defers_arena_destroy = match platform.family() {
        PlatformFamily::Linux => true,
        // macOS destroys the arena at exit; Windows console does the same. This is
        // computed for EVERY build, so it must be a real answer, not unreachable!.
        // A threadless program has no live workers, so destroying is safe; 47-H
        // revisits this when Windows threads (CreateThread) land, exactly as it
        // will for the Linux `&& has_threads` guard below.
        PlatformFamily::MacOS | PlatformFamily::Windows => false,
    };
    let skip_entry_arena_destroy = family_defers_arena_destroy
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
    let arena_global_slots =
        globals_base + link_slot_count + term_state_slots + presentation_mode_slots;
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
                initial_mode,
                uses_canvas: module_uses_canvas(module)
                    || module_uses_call(module, "canvas.blitSurface"),
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
                    needs_winsock: uses_net,
                    // plan-62-B: the worker owns the per-arena presentation-mode
                    // slot the program's `app::` calls read/write, so it seeds the
                    // `None` static default here.
                    seed_presentation_mode_offset,
                },
                &platform_imports,
            )?);
            code_functions.extend(app_entry);
            data_objects.extend(platform.app_mode_data_objects(&module.project));
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
                    needs_winsock: uses_net,
                    // Console builds have no `app::` (the package is `--app`-gated),
                    // so this is always `None` here and the entry is unchanged.
                    seed_presentation_mode_offset,
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
                )
            },
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
            platform,
            module.build_mode,
            &globals,
            &string_symbols,
            type_model.clone(),
        )?);
    }
    // A per-type runtime deep-copy function for every recursive type, so a
    // recursive value (e.g. `dom::Node`) can be transferred out of a worker arena
    // — `copy_value_to_current_arena` routes such a value's copy to a call to
    // `thread_copy_symbol(type)` rather than recursing over the type inline
    // (bug-391). The functions reference one another for their sub-edges; the
    // set is closed under that reference.
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
    for type_ in recursive_copy_types {
        // `recursive_transfer_types` returns rendered names (the emission ORDER
        // is observable in the `.ncode`); parse each back once, here, for the
        // typed emitters below.
        let type_ = ParameterType::declared(&type_);
        let symbol = thread_copy_symbol(&type_);
        code_functions.push(lower_thread_copy_function(
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
    code_functions.push(lower_arena_alloc(platform)?);
    code_functions.push(lower_build_error_loc());
    code_functions.push(lower_make_error_result());
    code_functions.push(lower_simd_alloc_list());
    code_functions.push(lower_arena_insert_free());
    code_functions.push(lower_arena_flush_coalesce());
    code_functions.push(lower_arena_free(platform));
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
                        global_slots: arena_global_slots,
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
        || runtime_symbols.iter().any(|symbol| {
            matches!(
                symbol.as_str(),
                "_mfb_rt_fs_fs_readText"
                    | "_mfb_rt_fs_fs_readAll"
                    | "_mfb_rt_fs_fs_readLine"
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
    // plan-98-D Phase 2: the graphics thread's trampoline. Emitted for a program that
    // draws, alongside the shared-state block `canvas::startGraphics` spawns it from.
    if module_uses_call(module, "canvas.startGraphics") {
        code_functions.push(crate::codegen::runtime::canvas::emit_graphics_trampoline(
            &platform_imports,
            platform,
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_runtime_helper(
    symbol: &str,
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
    arena_layout: ArenaLayout,
    uses_rng: bool,
    type_model: &TypeModel,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    string_symbols: &HashMap<String, String>,
) -> Result<CodeFunction, String> {
    let Some(spec) = runtime::spec_for_symbol(symbol) else {
        return Err(format!(
            "native code plan does not emit runtime helper '{symbol}'"
        ));
    };
    // Every runtime helper lowers to the same CodeFunction shape — an empty
    // `params` list and the spec's return type — differing only in the
    // (frame, instructions, relocations, stack_slots) tuple each arm computes.
    // Build that tuple here, then construct the single CodeFunction after the
    // match (the shape the net.*/tls.* inner-match arms already used).
    let (frame, mut instructions, mut relocations, stack_slots) =
        if crate::codegen::registry::is_abi_function_call(spec.call) {
            // Any package's `AbiFunction` member (the unified successor to OsLower) is
            // wrapped once into its shared `_mfb_rt_*` helper by
            // `lower_abi_function_helper`, regardless of package prefix — so this is
            // checked before the per-package prefix arms below.
            lower_abi_function_helper(
                spec.call,
                symbol,
                build_mode,
                module_name,
                type_model,
                platform_imports,
                platform,
                string_symbols,
                arena_layout.term_state_offset,
                arena_layout.presentation_mode_offset,
                arena_layout.global_slots,
                uses_rng,
            )?
        // (`term.*` members are `Body::abi_function` since the clean-room migration —
        // each member's own per-member body (`func_*.rs::lower_<name>`) calls the shared
        // `gen_shared::lower_term_helper`, which branches app-vs-console off the `AbiCtx`
        // (`build_mode`/`term_state_offset`/`presentation_mode_offset`) and, in app
        // mode, appends the platform's `emit_app_term_helper` sequence — so they route
        // through the `is_abi_function_call` branch above; no `term.` arm here. The
        // app-mode `ErrWrongMode` gate stays in the shared code layer, spliced by that
        // shared helper.)
        } else {
            match spec.call {
                // (`app.*` presentation-mode members are `Body::abi_function` since the
                // clean-room migration — each member's own per-member body reads the
                // arena `presentation_mode_offset` off the `AbiCtx` — so they route
                // through the `is_abi_function_call` branch above; no `app.` arm here.)
                // plan-67-B: internal perf-tracking helpers (injected, never NIR-level).
                "perf.init" | "perf.start" | "perf.end" | "perf.done" => {
                    perf::lower_perf_helper(spec.call, symbol, platform_imports, platform)?
                }
                // (`crypto.*` and `os.*` members are `Body::abi_function` since the
                // clean-room migration — each member owns its per-member body in its
                // `func_*.rs` (`lower_<name>`), so they route through the
                // `is_abi_function_call` branch above; no `crypto.`/`os.` arm here.)
                // (`io.*` members are `Body::abi_function` since plan-101, so they are
                // handled by the `is_abi_function_call` branch above — no `io.` arm here.)
                // (`thread.*` members are `Body::abi_function`/`abi_function_aliased` since
                // the clean-room migration — each member owns its per-member body in its
                // `func_*.rs` (`lower_<name>`), which calls the shared un-finalized
                // `gen_shared`/runtime-thread emitters and branches worker/parent + the
                // resource plane off `AbiCtx::call`; the internal `emit`/`read`/
                // the `*Resource`/`drop` code forms are `os_aliases`. So they
                // route through the `is_abi_function_call` branch above; no `thread.` arm
                // here. `thread.start` reads `AbiCtx::arena_global_slots`/`uses_rng`.)
                // (`net.*` members are `Body::abi_function_aliased` since the clean-room
                // migration — each member's own per-member body (`func_*.rs::lower_<name>`)
                // plus its `connectTcpAddr`/`pollList` code-form aliases — so they route
                // through the `is_abi_function_call` branch above; no `net.` arm here.)
                // (`audio.*` device-I/O members are `Body::abi_function_aliased` since the
                // clean-room migration — each member's own per-member body
                // (`func_*.rs::lower_<name>`) plus its IR-level overload-split code forms
                // (openInputDevice/openOutputDevice/readTimeout/pollTimeout/closeInput/
                // closeOutput) — so they route through the `is_abi_function_call` branch
                // above; no `audio.` arm here.)
                // Every `process.*` member is `Body::abi_function` since the
                // clean-room migration (the shared `per-member body plus
                // its `spawnEnv`/`sendTimeout`/… code-form aliases), so it routes
                // through the `is_abi_function_call` branch above. `process.__drop` is
                // the sole exception — it is not a descriptor member (the scope-drop op
                // synthesized during IR lowering), so it keeps its own family-branching
                // self-finalizing helper and this dedicated arm.
                "process.__drop" => crate::codegen::builtins::process::lower_process_drop_helper(
                    symbol,
                    platform_imports,
                    platform,
                )?,
                // (`tls.*` members are `Body::abi_function`/`abi_function_aliased` since
                // the clean-room migration — each member's own per-member body plus its
                // `pollList`/`closeListener` code-form aliases — so they route through the
                // `is_abi_function_call` branch above; no `tls.` arm here.)
                other => {
                    return Err(format!(
                        "native code plan does not emit runtime call '{other}'"
                    ));
                }
            }
        };
    // plan-62-E / plan-98-A: the console-reading `io::` helpers
    // (`input`/`readLine`/`readChar`) are gated in an app build that uses `app::` —
    // in a mode whose window input pipe has no producer, an ungated read would block
    // forever, so they raise the trappable `ErrWrongMode` instead. The requirement is
    // `WindowedMode`, not `Console`: `Canvas` has a window whose key events feed the
    // very same fd-0 pipe, so only `None` — which has no window at all — has nowhere
    // for input to come from. The writing side (`io::print`/`io::write`) is never
    // gated: it degrades to the fd sink in every mode. `io::readByte` is not in this
    // list and never has been, so it is ungated in all modes (plan-98-A §2).
    // No-op when the program cannot leave `Console` (`presentation_mode_offset` is
    // `None`). `term::` is gated separately at its own dispatch above, and keeps the
    // `Console`-only requirement — it needs the character grid, not just input.
    if matches!(spec.call, "io.input" | "io.readLine" | "io.readChar") {
        app::prepend_wrong_mode_gate(
            &mut instructions,
            &mut relocations,
            symbol,
            arena_layout.presentation_mode_offset,
            app::ModeRequirement::WindowedMode,
        );
    }
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

/// Whether a runtime helper symbol belongs to the TLS **server** side
/// (`tls::listen`/`tls::accept`/the listener close). Gates the server-only
/// data objects and block trampolines so client-only programs keep their
/// exact pre-server native output (plan-06-tls-server.md §1 non-goals).
pub(crate) fn is_tls_server_symbol(symbol: &str) -> bool {
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
/// in `tls/mod.rs` as a `pub(crate)` item that the whole of `code/` already
/// resolved through its glob imports, while four sibling modules each defined a
/// byte-identical private copy that shadowed it (bug-322). Those are deleted;
/// this is the one definition.
pub(crate) fn emit_alloc(
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

/// `arena_free(x0 = ptr, x1 = size)` — return a compiler-sized allocation to the
/// arena. The caller preloads `x0`/`x1`; one internal-call relocation per (from,to)
/// covers all sites in `symbol`. Companion of [`emit_alloc`].
pub(crate) fn emit_arena_free(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::branch_link(ARENA_FREE_SYMBOL));
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
}

pub(crate) fn internal_branch(from: &str, to: &str) -> CodeRelocation {
    CodeRelocation {
        from: from.to_string(),
        to: to.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }
}

pub(crate) fn external_branch(
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
pub(crate) fn lower_map_build_buckets_helper() -> CodeFunction {
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
        abi::move_register("%v18", abi::c_arg(0)),
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
pub(crate) fn lower_map_bucket_put_helper() -> CodeFunction {
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
        abi::move_register("%v20", abi::c_arg(0)),
        abi::move_register("%v21", abi::c_arg(1)),
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
/// `_mfb_rt_string_concat(left, right) -> block | 0` — the shared two-operand
/// `String` concatenation (plan-118-C).
///
/// `binop:Concat` was the single largest expansion category in the module:
/// 2,907,604 builder-emitted instructions over 17,221 sites, 169 each, because
/// every `a & b` inlined the length loads, the arena allocation, the header
/// store, and TWO byte-at-a-time copy loops. All of that is here once; a call
/// site becomes two argument moves, a `bl`, and a null check.
///
/// The copies are word-at-a-time where the alignment permits it. An arena block
/// is 8-aligned and both payloads start at `+8`, so copying the LEFT operand
/// keeps both cursors aligned for as long as 8 bytes remain. The RIGHT operand's
/// destination is only aligned when the left length is a multiple of 8, so that
/// case is tested at run time and falls back to bytes otherwise — rather than
/// issuing an unaligned word access, which a riscv64 target may service by a
/// kernel trap.
///
/// The allocation error stays at the call site: an `ErrOutOfMemory` carries the
/// `ErrorLoc` of the concatenation that failed, and a shared helper has no line
/// or column. So this returns a null pointer and the site raises.
pub(crate) fn lower_string_concat_helper() -> CodeFunction {
    let symbol = STRING_CONCAT_SYMBOL;
    let fail = format!("{symbol}_fail");
    let left_word = format!("{symbol}_left_word");
    let left_byte = format!("{symbol}_left_byte");
    let left_done = format!("{symbol}_left_done");
    let right_word = format!("{symbol}_right_word");
    let right_byte = format!("{symbol}_right_byte");
    let right_done = format!("{symbol}_right_done");
    let mut instructions = vec![
        abi::label("entry"),
        // Park both operands before the allocation call: `_mfb_arena_alloc`
        // clobbers the argument registers (and on x86-64 rdx via its own
        // arithmetic), exactly as `lower_map_probe_helper` documents.
        abi::move_register("%v20", abi::c_arg(0)),
        abi::move_register("%v21", abi::c_arg(1)),
        abi::load_u64("%v22", "%v20", 0), // left byteLength
        abi::load_u64("%v23", "%v21", 0), // right byteLength
        abi::add_registers("%v24", "%v22", "%v23"),
        // `mfb.string.v1` is `u64 byteLength; bytes; NUL`, so 8 + len + 1.
        abi::add_immediate(abi::c_arg(0), "%v24", 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&fail),
        abi::move_register("%v25", abi::mfb_return(1)), // new block
        abi::store_u64("%v24", "%v25", 0),
        abi::add_immediate("%v26", "%v25", 8), // write cursor
        abi::add_immediate("%v27", "%v20", 8), // left payload
        abi::add_immediate("%v28", "%v21", 8), // right payload
        // --- left operand: aligned words, then the tail ---
        abi::move_register("%v29", "%v22"),
        abi::label(&left_word),
        abi::compare_immediate("%v29", "8"),
        abi::branch_lo(&left_byte),
        abi::load_u64("%v30", "%v27", 0),
        abi::store_u64("%v30", "%v26", 0),
        abi::add_immediate("%v27", "%v27", 8),
        abi::add_immediate("%v26", "%v26", 8),
        abi::subtract_immediate("%v29", "%v29", 8),
        abi::branch(&left_word),
        abi::label(&left_byte),
        abi::compare_immediate("%v29", "0"),
        abi::branch_eq(&left_done),
        abi::load_u8("%v30", "%v27", 0),
        abi::store_u8("%v30", "%v26", 0),
        abi::add_immediate("%v27", "%v27", 1),
        abi::add_immediate("%v26", "%v26", 1),
        abi::subtract_immediate("%v29", "%v29", 1),
        abi::branch(&left_byte),
        abi::label(&left_done),
        // --- right operand: words only if the write cursor is still aligned ---
        abi::move_register("%v29", "%v23"),
        abi::move_immediate("%v31", "Integer", "7"),
        abi::and_registers("%v30", "%v22", "%v31"),
        abi::compare_immediate("%v30", "0"),
        abi::branch_ne(&right_byte),
        abi::label(&right_word),
        abi::compare_immediate("%v29", "8"),
        abi::branch_lo(&right_byte),
        abi::load_u64("%v30", "%v28", 0),
        abi::store_u64("%v30", "%v26", 0),
        abi::add_immediate("%v28", "%v28", 8),
        abi::add_immediate("%v26", "%v26", 8),
        abi::subtract_immediate("%v29", "%v29", 8),
        abi::branch(&right_word),
        abi::label(&right_byte),
        abi::compare_immediate("%v29", "0"),
        abi::branch_eq(&right_done),
        abi::load_u8("%v30", "%v28", 0),
        abi::store_u8("%v30", "%v26", 0),
        abi::add_immediate("%v28", "%v28", 1),
        abi::add_immediate("%v26", "%v26", 1),
        abi::subtract_immediate("%v29", "%v29", 1),
        abi::branch(&right_byte),
        abi::label(&right_done),
        abi::move_immediate("%v30", "Integer", "0"),
        abi::store_u8("%v30", "%v26", 0),
        abi::move_register(abi::mfb_return(0), "%v25"),
        abi::return_(),
        abi::label(&fail),
        abi::move_immediate(abi::mfb_return(0), "Integer", "0"),
        abi::return_(),
    ];
    let relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.stringConcat".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "String".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations,
    }
}

pub(crate) fn lower_map_probe_helper() -> CodeFunction {
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
        abi::move_register("%v20", abi::c_arg(0)),
        abi::move_register("%v21", abi::c_arg(1)),
        abi::move_register("%v22", abi::c_arg(2)),
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
        abi::move_register(abi::mfb_return(0), "%v13"),
        abi::branch(&done),
        abi::label(&pnext),
        abi::add_immediate("%v4", "%v4", 1),
        abi::compare_registers("%v4", "%v10"),
        abi::branch_lo(&nowrap),
        abi::move_immediate("%v4", "Integer", "0"),
        abi::label(&nowrap),
        abi::branch(&ploop),
        abi::label(&notfound),
        abi::move_immediate(abi::mfb_return(0), "Integer", "0"),
        abi::subtract_immediate(abi::mfb_return(0), abi::mfb_return(0), 1), // -1
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
pub(crate) fn resolve_closer_symbol(
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

/// Resolve every logical `LINK` library this module names to the concrete
/// `source` the declaring binding declared for this build's `(os, arch, libc)`
/// (plan-46-C §4.2).
///
/// Locators come from two places: an imported binding's `.mfp` section 10 (read
/// here, since resolution is per-target and so cannot happen upstream of this
/// per-flavor pass), and the project's own `libraries` section for a project
/// declaring its own `LINK` block. A no-match or ambiguity is a hard build error.
pub(crate) fn resolve_link_libraries(
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
pub(crate) fn bind_deferred_relocation_libraries(
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

pub(crate) fn native_link_error_messages() -> Vec<(&'static str, &'static str, &'static str)> {
    // plan-88-D: the `(code, message, symbol)` triples are sourced from
    // `ERRORCODE_CONSTANTS` by name (order preserved: it drives data-object layout).
    // Boundary validations (plan-linker.md §12.3/§12.4) are the last four.
    [
        "ErrNativeBindingUnavailable",
        "ErrNativeBindingCallFailed",
        "ErrNativeBufferOverrun",
        "ErrOutOfMemory",
        "ErrOverflow",
        "ErrEncoding",
        "ErrFloatNaN",
        "ErrFloatInf",
    ]
    .into_iter()
    .map(|name| {
        crate::codegen::registry::runtime_error_triple(name)
            .expect("native-link error name is an errorCode constant")
    })
    .collect()
}

pub(crate) fn standard_error_messages() -> Vec<(&'static str, &'static str, &'static str)> {
    // plan-88-D: `(code, message, symbol)` triples sourced from `ERRORCODE_CONSTANTS`
    // by name; order preserved (it drives the emitted data-object layout).
    [
        "ErrInvalidArgument",
        "ErrInvalidFormat",
        "ErrOverflow",
        "ErrUnderflow",
        "ErrFloatDomain",
        "ErrFloatNaN",
        "ErrFloatInf",
        "ErrFloatOverflow",
        "ErrOutOfMemory",
        "ErrIndexOutOfRange",
        "ErrNotFound",
        "ErrTimeout",
        "ErrInterrupted",
        "ErrUnknown",
        "ErrReadFailed",
        "ErrAlreadyExists",
        "ErrAccessDenied",
        "ErrResourceBusy",
        "ErrCloseFailed",
        "ErrPathNotFound",
        "ErrInvalidPath",
        "ErrResourceClosed",
        "ErrResourceMoved",
        "ErrUnsupported",
        "ErrWriteFailed",
        "ErrEndOfFile",
        "ErrEncoding",
        "ErrInputFailed",
        "ErrInvalidContext",
        "ErrWrongMode",
        "ErrAddressInvalid",
        "ErrAddressNotFound",
        "ErrNetworkFailed",
        "ErrConnectionClosed",
        "ErrMessageTooLarge",
        "ErrTlsFailed",
        "ErrAudioUnavailable",
        "ErrAudioDevice",
        "ErrSpawnFailed",
    ]
    .into_iter()
    .map(|name| {
        crate::codegen::registry::runtime_error_triple(name)
            .expect("standard error name is an errorCode constant")
    })
    .collect()
}

pub(crate) fn standard_error_message_symbol(message: &str) -> Option<&'static str> {
    standard_error_messages()
        .iter()
        .find_map(|(_, candidate, symbol)| (*candidate == message).then_some(*symbol))
}
