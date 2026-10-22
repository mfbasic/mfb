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
use crate::codegen::collection::map::map_hash::{home_slot_of_key_bytes, KeyHashRegs};
use crate::codegen::collection::sort::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::park_error_helper::{
    lower_drop_owned_collection_helper, lower_drop_owned_string_cap_helper,
    lower_drop_owned_string_helper, lower_park_error_helper,
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

mod error_messages;
mod link_resolution;
mod lower_module;
mod module_finish;
mod module_helpers;
mod module_layout;
mod runtime_helpers;
#[cfg(test)]
mod tests;

pub(crate) use error_messages::{
    native_link_error_messages, standard_error_message_symbol, standard_error_messages,
};
pub(crate) use link_resolution::{
    bind_deferred_relocation_libraries, resolve_closer_symbol, resolve_link_libraries,
};
pub(crate) use lower_module::{lower_module_for_platform, lower_module_mir_for_platform};
pub(crate) use runtime_helpers::{
    emit_alloc, emit_arena_free, external_branch, internal_branch, is_tls_server_symbol,
    lower_map_bucket_put_helper, lower_map_build_buckets_helper, lower_map_probe_helper,
    lower_runtime_helper, lower_string_concat_helper,
};

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
    /// Float values whose bits in a GPR `ValueResult.location` are *also* resident
    /// in an FP virtual register (the `d`-register a float op left its result in).
    /// Lets a chained float operand be read straight from the `d`-register instead
    /// of round-tripping back through a GPR (plan-03 Stage C). Sound because both
    /// names hold the same value and neither vreg is ever redefined; the allocator
    /// keeps the FP vreg live (or spills/reloads it) wherever it is used.
    pub(crate) float_residents: HashMap<String, abi::Reg>,
    /// Float locals currently promoted to an FP virtual register for the duration
    /// of an enclosing loop (plan-03 Stage D part 2): name -> the `%fN` holding
    /// the live value. While promoted, reads of the local come from this register
    /// and writes update it, instead of round-tripping its stack slot every
    /// iteration; it is loaded once before the loop and stored back once after.
    pub(crate) promoted_float_locals: HashMap<String, abi::Reg>,
    /// Locals whose address is taken anywhere in the function (a `LocalRef`).
    /// Such a local may be observed or mutated through the slot reference, so it is never
    /// loop-promoted (its slot must stay authoritative).
    pub(crate) address_taken_locals: HashSet<String>,
    /// plan-86 E, bug-689: bindings `e = collections::get(L, i)` (or `getOr`) that
    /// hold an ALIAS into `L`'s element instead of an owned copy
    /// (`analysis::borrow_get`). Such a `get` returns a borrow instead of a fresh
    /// `copy_flat_block`, and the binding registers no scope-drop free (the container
    /// owns the element). The copy-skip AND the free-skip are BOTH gated on this set
    /// (a freed borrow is a double-free into the container).
    pub(crate) borrow_get_locals: HashSet<String>,
    /// bug-689: the element-bound `get → WITH → set` updates of this function.
    pub(crate) element_updates: ElementUpdates,
    /// bug-766 B: this function's record-temp scratch and the fixed-size record
    /// temporaries placed in it (`install_record_temps`).
    pub(crate) record_temps: RecordTemps,
    /// plan-86 E: true inside the `lower_value` frame of the ONE borrowed `get`
    /// node (a `borrow_get_locals` binding's initializer), so
    /// `materialize_owned_element` returns the aliasing borrow instead of copying
    /// it and `register_pending_temp` does not free it.
    ///
    /// bug-592: it is scoped to that node's own frame, NOT the whole initializer.
    /// `lower_value` takes it from [`Self::borrow_get_armed`] on entry and clears it
    /// for every operand frame, so an operand of the borrowed call (a `String` map
    /// key built by `getOr` or `&`) takes the ordinary copy + statement-scope free.
    /// Covering the whole initializer suppressed those frees and leaked them.
    /// Both readers see the same narrowed value, so a nested `get` operand is
    /// copied exactly when it is freed — never an alias that gets freed.
    pub(crate) borrow_get_result: bool,
    /// bug-592: set by the `Bind` arm immediately before it lowers a borrowed
    /// initializer; consumed by the very next `lower_value` frame, which is that
    /// initializer's own node.
    pub(crate) borrow_get_armed: bool,
    /// bug-770 C: the `MUT x = get/getOr(…)` binds (by `op_key`) whose next
    /// statement is `x = append(x, item)`, with that item
    /// (`analysis::append_headroom`). Their owned copy is made with growth
    /// headroom, so the append writes into it instead of reallocating.
    pub(crate) append_headroom_binds:
        crate::codegen::engine::analysis::append_headroom::AppendHeadroomBinds,
    /// bug-770 C: true inside the `lower_value` frame of the ONE `get`/`getOr`
    /// node an approved `append_headroom_binds` bind lowers, so
    /// `materialize_owned_element` copies a `List` with headroom instead of tight.
    /// Scoped exactly like [`Self::borrow_get_result`]: taken from
    /// [`Self::headroom_copy_armed`] on entry and cleared for every operand frame.
    pub(crate) headroom_copy_result: bool,
    /// bug-770 C: set by the `Bind` arm immediately before it lowers an approved
    /// initializer; consumed by the very next `lower_value` frame.
    pub(crate) headroom_copy_armed: bool,
    /// bug-760 B1: set by a tombstone-aware consumer of a `Map`/`Set` (`len`, a
    /// hash lookup) immediately before it lowers that operand. `lower_value` takes
    /// it on entry — so it covers exactly the next frame — and a `Local`/`Global`
    /// read in that frame then skips the settle every other read of a map binding
    /// runs (`CodeBuilder::emit_map_settle_value`).
    pub(crate) map_settle_exempt_armed: bool,
    /// bug-768: set by `lower_field_access_of` when the field it just read is a
    /// record field or a `STATE` payload — the owners whose inlined `Map`/`Set`
    /// sub-blocks an in-place field removal can leave tombstones in. Taken by the
    /// `lower_value` frame the read returns to, which then settles the result
    /// (`CodeBuilder::emit_tombstone_holder_settle`); every frame takes it on exit,
    /// so an inner frame's read never reaches an outer one.
    pub(crate) record_member_read: bool,
    /// bug-744: set by a position that discards its value — an `Eval` statement, or
    /// a `RETURN` from a `Nothing` function, whose result register is never read —
    /// immediately before it lowers that value; consumed by the very next
    /// `lower_value` frame (the value's own root node). Every other frame is a
    /// value position, where a `Nothing`-typed call's `void` result is
    /// materialized as the unit value `NOTHING` (0).
    pub(crate) value_discarded_armed: bool,
    /// bug-842: the root `NirValue` (by address) of the value a `LET`/`MUT` binding,
    /// an assignment or a module-level store is lowering. That statement takes the
    /// resources its own value carries — a binding claims them, a container adopts
    /// them, a global raises them — so only a producer call NESTED inside it is an
    /// unclaimed temporary (`register_resource_temp`). An `Eval` and a `RETURN`
    /// exempt their root through `value_discarded_armed` instead.
    pub(crate) resource_temp_exempt: Option<usize>,
    /// bug-842: locals bound to a resource-adopting `callResult` — the inline-`TRAP`
    /// desugar's `$trap_resN`. A `$trap_valN = resultValue($trap_resN)` assignment
    /// into a container adopts the resources that call handed out, exactly as a
    /// direct `xs = f()` does (`adopts_from_trapped_call`).
    pub(crate) trap_call_results: HashSet<String>,
    /// bug-842: set by a `RETURN` that closed its statement's resource temporaries
    /// ahead of the release (`close_resource_temps_before_return`); the interior drop
    /// that path emits next frees their blocks without closing them twice, then
    /// clears it.
    pub(crate) resource_temps_closed_on_path: bool,
    /// bug-999: the aggregate resource temporaries their statement owns outright.
    pub(crate) statement_temps: StatementTemps,
    /// plan-86 K1: true while lowering a function whose every value-return is a bare
    /// parameter (`function_returns_param_borrow`). Its `RETURN <param>` returns the
    /// argument pointer uncopied (a borrow); callers deep-copy the result only at an
    /// owning store, so the copy moves to the caller's ownership boundary with
    /// identical value semantics. Consistent with the caller side because both key
    /// off the same predicate.
    pub(crate) current_returns_param_borrow: bool,
    /// bug-536 shape B-2: true while lowering a function whose bare-`String` result
    /// callers are allowed to free (`function_returns_fresh_string`). It obliges
    /// this function to hand back a solely-owned block on EVERY return path, so
    /// `lower_returned_value` copies any returned `String` whose freshness lowering
    /// could not otherwise establish. Consistent with the caller side because both
    /// key off the same predicate — a disagreement is a leak in one direction and a
    /// double free of the caller's live `String` in the other.
    pub(crate) current_returns_fresh_string: bool,
    /// plan-86 K1: names of every function used as a `FunctionRef` (callback) in the
    /// module. Such a function is invoked through an owning ABI, so it is excluded
    /// from the parameter-passthrough borrow elision (`function_returns_param_borrow`)
    /// — both here (callee) and at every call site (caller) — keeping the two sides
    /// consistent. Shared verbatim across all functions in the build.
    pub(crate) callback_referenced_functions: HashSet<String>,
    /// bug-623 B: locals whose single store is a bare `Local(src)` — followed by a
    /// `RETURN` to find the binding that owns a returned union's box.
    pub(crate) resource_alias_sources: HashMap<String, String>,
    /// bug-1053: every owned-list of this function whose drain takes records for a
    /// later reclaim, for the error-return exit that frees them (the last
    /// error-return pad, `CodeBuilder::append_error_return_pads`).
    pub(crate) reclaiming_owned_lists: Vec<OwnedListCleanup>,
    /// The `RES` bindings of this function whose drop takes its record for the
    /// reclaim, for the shared last error-return pad.
    pub(crate) reclaiming_resources: Vec<ActiveCleanup>,
    /// bug-1045: the stores that raise an inner scope's value into a container
    /// declared further out, and the containers whose drain may only close what
    /// such a store handed them (`record_ownership::StoreFloats`).
    pub(crate) store_floats: crate::codegen::resource::cleanup::record_ownership::StoreFloats,
    /// bug-648: the inline-`TRAP` temps that receive a borrowed store, and the rule
    /// that classifies each store (`resource::cleanup::trap_ownership`).
    pub(crate) trap_ownership: crate::codegen::resource::cleanup::trap_ownership::TrapOwnership,
    /// bug-648: the flag slot of each bound `Mixed` resource temp, written at every
    /// store with whether that store handed the temp an owned value. The temp's drop
    /// closes only when it is set.
    pub(crate) trap_owner_flags: HashMap<String, usize>,
    /// bug-1051: per inline-`TRAP` temp, the slot holding the closed default record
    /// its bind materialized (`ResourceCleanup::default_record_slot`).
    pub(crate) trap_default_records: HashMap<String, usize>,
    /// Resource-union alias class: bind-time `name -> src` for a resource bind that is a
    /// bare-local alias (`RES v = u`, `RES d AS Union = c`), in lowering order. Unlike
    /// `resource_alias_sources` it follows the binding actually live at a `RETURN`.
    pub(crate) live_resource_aliases: HashMap<String, String>,
    /// Resource-union alias class: bind-time `name -> src` for a union wrapping a local
    /// (`RES c AS Union = u`) — the alias whose `RETURN` retires `src`'s close.
    pub(crate) live_union_wraps: HashMap<String, String>,
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
    /// bug-755: this function's scope-token stack object, allocated on the first
    /// claim, float, owner-checked drop or release (`resource::owner`).
    pub(crate) scope_token_slot: Option<usize>,
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
    /// bug-695: the shared error-return landing pads of the function being lowered,
    /// keyed by the cleanup-stack prefix each one drops, mapped to the pad's label.
    /// An inline raise that leaves the function branches to the pad for the whole
    /// active stack; the pad drops the innermost cleanup and continues into the pad
    /// for the next-shorter prefix, and the empty prefix's pad reloads the parked
    /// `Result` and returns. One pad per distinct prefix instead of the whole walk
    /// at every raise site ([`Self::emit_inline_error_exit`]).
    pub(crate) error_return_pads: HashMap<Vec<ErrorReturnPadEntry>, String>,
    /// bug-695: the pads' code, emitted when first requested (so each drop reads the
    /// builder state of the point that needed it) but held back and appended after
    /// the body by [`Self::append_error_return_pads`], deepest prefix first. That
    /// placement makes every branch into and between pads a FORWARD branch: a
    /// backward one reads as a loop to the live-range analyses
    /// (`v128_live_ranges` stretched every vector value across it).
    pub(crate) error_return_pad_code: Vec<ErrorReturnPad>,
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
    /// bug-778: `Result`-locals whose every use is `ResultIsOk` / `ResultValue` /
    /// `ResultError` of the local itself, each value read the direct value of a
    /// `Bind`/`Assign` — the inline-`TRAP` desugar's `$trap_resN` temporaries.
    /// Such a local never needs the heap `{tag, size, payload}` block: it lives
    /// in frame slots ([`UnboxedTrapResult`]). Populated once per function before
    /// lowering the body (`unboxed_trap_result_candidates`); a bind still checks
    /// the payload type before taking the unboxed form.
    pub(crate) unboxed_trap_candidates: HashMap<String, UnboxedTrapUses>,
    /// bug-778: the inline-`TRAP` delivery slots (`$trap_valN`, `$trap_errN`)
    /// read exactly once, by the whole value of a `Bind`/`Assign` — the
    /// desugar's `Bind y = $trap_valN`. That read is the slot's last use, so the
    /// owning store MOVES the block out instead of copying it
    /// (`trap_delivery_moves`, consulted by `lower_value_owned`).
    pub(crate) trap_delivery_moves: HashSet<String>,
    /// bug-778: the unboxed trap results bound so far, by local name — what the
    /// three readers consult instead of dereferencing a `Result` block.
    pub(crate) unboxed_trap_locals: HashMap<String, UnboxedTrapResult>,
    /// bug-778: ARMED by the unboxed `Bind` for exactly its own initializer.
    /// `lower_value` moves it into [`Self::unboxed_trap_active`] for the one frame
    /// it enters next — the initializer's own node — the way `borrow_get_armed`
    /// is scoped, so an operand frame below can never see it.
    pub(crate) unboxed_trap_pending: Option<UnboxedTrapResult>,
    /// bug-778: the unboxed trap result the CURRENT `lower_value` frame's trapped
    /// producer (`CallResult`/`Checked`) must fill instead of allocating a
    /// `Result` block. Taken by `finish_unboxed_trap_result`.
    pub(crate) unboxed_trap_active: Option<UnboxedTrapResult>,
    /// bug-778: set by `finish_unboxed_trap_result`, so the unboxed `Bind` can
    /// prove its producer filled the slots rather than building a heap `Result`
    /// the readers would then misread.
    pub(crate) unboxed_trap_filled: bool,
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
    /// Re-entrancy guard for an inline error's exit route (bug-03, bug-695).
    /// Set while `emit_error_register_return` is routing an inline failure out —
    /// to the enclosing trap or, through the scope-drop walk, to the caller. The
    /// trap route itself builds an `Error` inline, whose OOM fallback re-enters
    /// `emit_error_register_return` — which must then return to the caller (a
    /// plain `return_()`) rather than start a second walk over cleanups the first
    /// is already dropping.
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
    /// Live owned-lists: `(collection binding name, resource leaf type)` ->
    /// head-pointer stack slot. A container whose values reach several resource
    /// kinds (bug-805) keeps one list per kind, so each list has one drop.
    pub(crate) owned_list_heads: HashMap<(String, ParameterType), usize>,
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
    /// bug-781: the environment layout of every capturing `LAMBDA` this function
    /// creates (`record_closure_env_layout`), handed to the module so it emits each
    /// `LAMBDA`'s environment routine once (`lower_closure_env_ops`).
    pub(crate) closure_env_layouts: Vec<ClosureEnvLayout>,
    /// bug-536 shape B: the location of a `String` block this builder has just
    /// **provably freshly allocated** — set by the shared String producers
    /// (`emit_materialize_string_from_bytes`, `_mfb_rt_string_concat`, the
    /// `toString` formatter helpers) immediately after the `arena_alloc` whose
    /// block they return, and consumed by the very next `lower_value` that
    /// finishes lowering a node.
    ///
    /// This is fail-closed provenance, and the asymmetry is the whole safety
    /// argument. `register_pending_temp` frees a bare `String` temp only when
    /// this mark names the same operand the node's `ValueResult` carries; a
    /// producer that does not set it (a `RETURN "literal"` rodata pointer, the
    /// `toString(String)` identity arm that hands back its *argument*, a
    /// `strings::*` view into a parameter) keeps the pre-existing leak, which is
    /// bounded and safe. Freeing one of those instead would be a wild
    /// `arena_free` — SIGBUS on rodata, free-list corruption on a borrow.
    ///
    /// It lives on the builder rather than on `ValueResult` deliberately: the
    /// producers return a bare `VirtualRegister`, and the 333 `ValueResult`
    /// literals between them and `lower_value` would each have to re-thread a
    /// struct field, so the flag would be dropped (and the fix inert) at almost
    /// every site. Staleness is impossible because `lower_value` clears the mark
    /// before lowering a node and takes it after, and honours it only when it
    /// names that node's own result operand — the vregs the producers return are
    /// freshly allocated, so no unrelated value can wear one.
    pub(crate) fresh_string_block: Option<Operand>,
    /// bug-496: addresses (`&NirValue as usize`) of the operand nodes of every
    /// multi-operand value currently being lowered that must be **snapshotted**
    /// before a later sibling operand runs. Pushed by `lower_value` on entry to a
    /// `Call`/`Binary`/literal/constructor node for each operand that lowers to a
    /// pointer INTO storage a later operand's call can reassign (a global, a
    /// by-ref / address-taken local, a resource's STATE — see
    /// `operand_reachable_by_later_call`), truncated back on exit. When one of
    /// these operands is lowered, `snapshot_aliased_operand` deep-copies it into a
    /// statement-scope temporary so the reassignment's old-block free cannot
    /// dangle it: `g = append(g, f())` / `GS & f()` where `f` reassigns `g`/`GS`
    /// read the FREED block otherwise (audit-3 MEM-12). Node identity (address)
    /// rather than structural equality: the operands are lowered from the same
    /// borrowed tree the frame was computed over, so it matches exactly the
    /// occurrence that precedes the call and never a later re-read of the same
    /// name. A rewritten/cloned operand simply does not match — fail-safe to the
    /// pre-fix behavior, never to a wrong copy.
    pub(crate) operand_snapshot_wanted: Vec<usize>,
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
    /// bug-1036: the live `FOR EACH` loops that walk a binding's OWN block (a plain
    /// local the loop did not copy at entry, `for_each_iterable_locals`) and whose
    /// body reassigns it. A reassignment cannot free the block such a loop still
    /// reads, so it hands that block to the loop instead: it stores the pointer in
    /// the loop's `deferred_slot`, which the loop frees on every exit (an
    /// `OwnedValue` cleanup registered outside the body's scope). Outermost first,
    /// so a block two nested loops walk goes to the one that ends last.
    pub(crate) for_each_entry_blocks: Vec<ForEachEntryBlock>,
    /// `String` local name → stack slot tracking the spare payload capacity (bytes
    /// available past the current length) of the local's grown in-place self-append
    /// buffer (plan-02 §4.1 / D9). Zero means "tight" (no spare). The slot is a
    /// frame-local shadow that never escapes: any copy/return/transfer reads only
    /// `len` bytes, freezing the value to the canonical tight `[len][bytes][NUL]`
    /// form, so the spare is invisible outside the buffer. Reset to 0 on any
    /// non-self-append bind/assign of the local and zeroed at function entry.
    pub(crate) string_capacity_slots: HashMap<String, usize>,
    /// plan-142-B: the frame slot of this function's self-update scratch — a
    /// `List OF Integer` block the in-place arms use as raw bytes (per-element
    /// marks, results) so that no arm allocates per statement. `Some` only when
    /// the body holds a self-update that may need it
    /// (`prescan_self_update_scratch`); zeroed at entry, grown on demand, freed at
    /// every exit by a function-level cleanup.
    pub(crate) self_update_scratch: Option<usize>,
    /// plan-142-G: in a lambda that borrows its creator's scratch
    /// (`scratch_closure_captures`), the closure-env word holding the address of
    /// the creator's scratch slot. `self_update_scratch` is then a working copy
    /// that `emit_reserve_self_update_scratch` loads from and publishes back
    /// through that address, and the lambda frees nothing: the creator owns it.
    pub(crate) self_update_scratch_env: Option<usize>,
    /// plan-145-C: the scalar updates of a mixed `WITH` (one collection field served
    /// by a seam arm, the rest scalars) — `(field index, new value)` in source
    /// order. The arm's field route evaluates and spills them in
    /// `open_inplace_dest`, its first emission, after every gate: so an arm that
    /// declines emits none of them, and the values are computed before the arm
    /// mutates anything. `try_inplace_mixed_with` then stores them.
    pub(crate) field_pre_emit: Option<Vec<(usize, NirValue)>>,
    /// plan-145-C: `(field index, slot)` for each `field_pre_emit` value, once the
    /// field route has spilled them.
    pub(crate) field_pre_emitted: Option<Vec<(usize, usize)>>,
    /// plan-142-C: set by the in-place `math` arm around its call to the member's
    /// lowering; `emit_alloc_result_list` takes it and returns the self-update
    /// scratch instead of allocating the result list.
    pub(crate) simd_result_into: Option<()>,
    /// For backends whose `RegisterModel::math_pool_base` is `None` (no free
    /// physical to pin, e.g. x86), the per-kernel virtual register holding the
    /// SIMD math constant-pool base, keyed by the function symbol it was minted
    /// in (vreg indices are per-function). Lets `emit_load_math_pool_base` and
    /// the `broadcast_*` coefficient loads share one allocator-placed base within
    /// a kernel while re-minting it for the next kernel. Unused (stays `None`) on
    /// backends that pin a physical base.
    pub(crate) math_pool_base_vreg:
        Option<(String, crate::codegen::engine::operand::VirtualRegister)>,
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
    /// plan-116-J: for each local, the resource locals it structurally **holds** —
    /// `LET tag = canvas::Text[…, fonts := [face], …]` gives `tag -> {face}`.
    ///
    /// Computed once per function as a pre-pass, in the shape
    /// `promotable_vector_locals` established, and read only by
    /// `deactivate_consumed_cleanups`: when a consuming parameter is handed `[tag]`, the
    /// close obligation that must be dropped belongs to `face`, whose name appears
    /// nowhere in the argument.
    ///
    /// **This is the NIR twin of `ir::verify`'s containment relation, and they are two
    /// lists.** `IrValue` and `NirValue` are distinct types so the walk cannot literally
    /// be shared; an arm added to one and not the other does not fail to compile. The
    /// asymmetry of the failure is worth knowing: a missing arm in the verifier's walk
    /// silently allows a use-after-close, and a missing arm HERE silently leaks — the
    /// scope closes a resource the callee has taken over, and the callee's later close
    /// is a defined no-op, so nothing crashes either way.
    pub(crate) resource_containment: HashMap<String, Vec<String>>,
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
    /// bug-774: cold raise blocks set aside by [`CodeBuilder::emit_cold_block`],
    /// appended after the function body by `run_register_allocation`, so the hot
    /// path falls through its checks instead of jumping over their raise code.
    pub(crate) cold_instructions: Vec<CodeInstruction>,
    /// Whether [`CodeBuilder::emit_cold_block`] may set code aside. Only a builder
    /// whose stream is finished by `run_register_allocation` (which appends the
    /// cold code) turns it on — `lower_function`'s; a helper builder finalized
    /// some other way keeps every raise inline.
    pub(crate) cold_blocks_enabled: bool,
    /// plan-134-B: set only while `_mfb_rt_graph_copy`'s body is emitted. The copy's
    /// edge sites read it to push a cycle-typed edge onto the walker's work stack
    /// instead of calling a per-type copy function. `None` in every other builder.
    pub(crate) graph_copy_walker:
        Option<crate::codegen::memory::arena::graph_copy::GraphCopyWalker>,
    /// plan-134-D: which owning stores of the NIR function being lowered read their
    /// source for the last time (plan-134-C). `None` in a synthesized builder, where
    /// every store copies.
    pub(crate) move_sites: Option<crate::codegen::engine::analysis::last_use::MoveSites>,
    /// plan-134-D: the `op_key` of the op `lower_ops_inner` is lowering — what a store
    /// asks `move_sites` about.
    pub(crate) current_op_key: Option<usize>,
    /// plan-147-D: which call arguments of the NIR function being lowered may be
    /// HANDED to the callee instead of lent (plan-147-C). `None` in a synthesized
    /// builder, where every argument is lent.
    pub(crate) handover: Option<crate::codegen::engine::analysis::handover::HandOverArgs>,
    /// plan-147-D: the `call_key` of the `Call`/`CallResult` node being lowered —
    /// what `emit_call` asks `handover` about, alongside `current_op_key`.
    pub(crate) current_call_key: Option<usize>,
    /// plan-147-E: the argument positions of the call just lowered whose in-place
    /// self-update fired, so the hand-over nulls those locals' slots instead of
    /// claiming a temporary that was never built. Written by
    /// `emit_prepared_call_args_with_site` and taken immediately after.
    pub(crate) args_updated_in_place: Vec<usize>,
    /// plan-146-D: the frame slot caching `os::appResourcePath`'s base block for the
    /// in-place arm (`prescan_string_app_resource_base`), or `None` in a function with
    /// no `s = os::appResourcePath(s)`.
    pub(crate) string_app_resource_base: Option<usize>,
    /// plan-146-G: in a lambda that self-updates with `os::appResourcePath`, the
    /// closure-environment word holding the address of the CREATOR's base-path
    /// cache slot.
    pub(crate) string_app_resource_base_env: Option<usize>,
    /// plan-146-G: in a lambda that self-updates a by-ref-captured `String`, the
    /// closure-environment word holding the address of the OWNER's capacity shadow,
    /// per captured local (`string_shadow_captures`). Empty everywhere else.
    pub(crate) string_shadow_env: std::collections::HashMap<String, usize>,
    /// plan-146-D: the module (project) name, which `os::appResourcePath`'s app-mode
    /// resource suffix embeds (`share/<module>`). Empty in a synthesized function,
    /// which lowers no NIR statement and so no self-update.
    pub(crate) module_name: String,
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
            float_residents: HashMap::new(),
            promoted_float_locals: HashMap::new(),
            address_taken_locals: HashSet::new(),
            borrow_get_locals: HashSet::new(),
            element_updates: ElementUpdates::default(),
            record_temps: RecordTemps::default(),
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
            synthesized_constructors: HashSet::new(),
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
            current_file: String::new(),
            current_loc: NirSourceLoc::default(),
            resource_owners: HashMap::new(),
            owner_containers: HashSet::new(),
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
            cold_blocks_enabled: false,
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
            module_name: String::new(),
        }
    }
}

/// bug-689: the element-bound bindings of the function being lowered
/// (`analysis::borrow_get::ElementBinding`), keyed for the lowering.
#[derive(Default)]
pub(crate) struct ElementUpdates {
    /// By binding name. A single-expression update adds its hidden binding here.
    pub(crate) bindings:
        HashMap<String, crate::codegen::engine::analysis::borrow_get::ElementBinding>,
    /// The `p = WITH p { … }` statements (`op_key`) → `p`.
    pub(crate) updates: HashMap<usize, String>,
    /// The write-backs `xs = set(xs, i, p)` (`op_key`) — each lowered as nothing.
    pub(crate) write_backs: HashSet<usize>,
    /// While an element update tries its in-place routes: the owner whose block is
    /// an element inside a list's data region. No route may grow it
    /// (`field_is_last_inlined`).
    pub(crate) field_owner: Option<String>,
    /// bug-784: beside `field_owner`, the update statement's `op_key` (`None` for
    /// a single-expression update) — the site whose write-back an in-place route
    /// that cannot finish in place runs instead.
    pub(crate) field_site: Option<usize>,
    /// Counter for the hidden bindings of single-expression updates (and of
    /// bug-784's spilled element-update values).
    pub(crate) hidden: usize,
    /// bug-784: this function's destination-passing call sites
    /// (`analysis::into_dest`), by `call_key`.
    pub(crate) into_sites: crate::codegen::engine::analysis::into_dest::IntoSites,
    /// bug-784: the `call_key` of the one call `emit_call` lowers as a call of
    /// its callee's destination variant; armed only around that call's
    /// `lower_value`.
    pub(crate) into_call_armed: Option<usize>,
    /// bug-784: in a destination variant (`<base>$into<j>`), parameter `j`'s
    /// name — the caller's destination.
    pub(crate) into_param: Option<String>,
    /// bug-784: while a destination variant lowers `RETURN WITH <param> { … }`
    /// through the field store, the slot that receives the returned address (the
    /// destination itself, or a fresh block on a size change). `None` otherwise.
    pub(crate) into_result_slot: Option<usize>,
    /// bug-784: the per-binding roles of re-bound names, by `Bind` `op_key`
    /// (`analysis::borrow_get::BorrowGets::rebound`), applied at each bind.
    pub(crate) rebound: HashMap<usize, crate::codegen::engine::analysis::borrow_get::Rebound>,
}

/// bug-766 B: the fixed-size record temporaries of the function being lowered
/// that live in its record-temp scratch rather than in an arena block of their
/// own, and that scratch. `mfb spec memory collections`, "Self-updates": "State
/// an operation needs per element lives in a per-function scratch block".
///
/// The scratch is a `List OF Integer` block like the self-update scratch
/// (`emit_reserve_scratch_block`), reserved at the first placement and freed at
/// every exit. It is a block of its own, not the self-update scratch: that one is
/// freed and reallocated whenever an arm needs more room than it has (and a
/// copying `sort` — an expression — can do that while a temporary is live), and a
/// temporary's binding holds an address into its block. This one is only ever
/// asked for `size` bytes, so after the first reservation it never moves.
///
/// Each temporary has a region of its own at a fixed offset, so two live at once
/// never overlap; a region is rewritten only by the next evaluation of its own
/// site, after which nothing reads the previous value (see `install_record_temps`).
/// Every activation — a recursive call, a lambda call — has its own frame slot
/// and so its own block: no scratch is shared.
#[derive(Default)]
pub(crate) struct RecordTemps {
    /// The frame slot holding the block (0 until the first reservation).
    pub(crate) slot: Option<usize>,
    /// The bytes every reservation asks for: the sum of the regions.
    pub(crate) size: usize,
    /// The scratch-copy `get` bindings (`analysis::borrow_get::BorrowGets::scratch`
    /// of a fixed-size record type) → `(offset, size)` of their region.
    pub(crate) bindings: HashMap<String, (usize, usize)>,
    /// The region of the `WITH` item a `set`/`append` self-update copies, shared
    /// by every such statement: `(offset, size)`.
    pub(crate) with_item: Option<(usize, usize)>,
    /// The address of the `WITH` node a self-update arm is about to lower as its
    /// item (`lower_self_update_item`); only that node's own lowering takes it.
    pub(crate) with_item_armed: Option<usize>,
    /// The address of a `WITH` node just built in the scratch; its `lower_value`
    /// frame takes it and registers no statement temp for the region.
    pub(crate) placed: Option<usize>,
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
    /// The IR loop name of a `FOR`/`FOR EACH` (`None` for `DO`/`WHILE`). A named
    /// EXIT/CONTINUE carries this name as its target and finds the loop by it.
    pub(crate) name: Option<String>,
    pub(crate) continue_label: String,
    pub(crate) exit_label: String,
    pub(crate) cleanup_depth: usize,
    /// `pending_temp_frees.len()` when the body starts. An `EXIT`/`CONTINUE` jumps past the
    /// statement-end drop of every statement it leaves inside the body — an `IF` whose
    /// condition made a temp — so it frees the pending temps above this depth (bug-620).
    pub(crate) temp_depth: usize,
}

impl LoopLabels {
    /// Whether an EXIT/CONTINUE with this `kind` and `target` acts on this loop: a
    /// named one matches the loop with that IR name, a bare one the loop of its kind.
    pub(crate) fn resolves(&self, kind: crate::ast::LoopKind, target: &Option<String>) -> bool {
        match target {
            Some(name) => self.name.as_deref() == Some(name.as_str()),
            None => self.kind == kind,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
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
    /// Every resource kind shares the resource record, but ONLY `File`'s open
    /// helpers zero the buffer words after the (PRNG-poisoned) arena alloc.
    /// `net`'s `emit_make_handle` initializes offsets 0/8/16/24 and leaves 32–80 as
    /// poison; the layout comment calls those words "inert" for non-`File`
    /// resources, which was true only because nothing read them. The drop-path
    /// reclaim made them live, so freeing them unconditionally handed
    /// `arena_free` a poison value and segfaulted every `net::` program during
    /// cleanup (plan-52-B Phase 2).
    pub(crate) has_io_buffers: bool,
    /// bug-623: whether the drop also frees the resource record itself, after the
    /// close and the block reclaim, and nulls the binding's slot. True only for the
    /// kinds `CodeBuilder::resource_record_freed_at_drop` names (the `tcp`, `udp`
    /// and `tls` handles); every other kind keeps its record as the tombstone.
    pub(crate) frees_record: bool,
    /// bug-623 D: a successful `thread::transfer` moved this binding's resource to
    /// another thread. The drop then closes nothing, since the handle is the
    /// receiver's. It frees only the sender's tombstone record, and only
    /// when that record carries the moved bit, so a transfer that failed frees
    /// nothing here. The tombstone stays readable until then: every use of the
    /// binding after the transfer still sees `moved`. Set by
    /// `retire_moved_resource_cleanup` in place of removing the cleanup, and only
    /// for a cleanup that `frees_record`.
    pub(crate) moved_record_only: bool,
    /// bug-648: for an inline-`TRAP` temp whose stores are sometimes borrowed, the
    /// stack slot recording whether the value it holds now is owned. The drop closes
    /// and reclaims nothing while it reads 0. `None` for every binding that owns
    /// unconditionally.
    pub(crate) owner_flag_slot: Option<usize>,
    /// bug-1051: for an inline-`TRAP` temp that is not proved to own every record
    /// stored into it, the stack slot holding the closed default record its bind
    /// materialized (0 once that record is freed). The default is the temp's own
    /// whatever the stores are, so a drop that finds it still in the binding's
    /// slot frees it (`CodeBuilder::emit_free_trap_default_record`). `None` for
    /// every other binding.
    pub(crate) default_record_slot: Option<usize>,
    /// bug-755: the nesting depth of the scope the binding belongs to. The drop
    /// closes the record only when its owner word holds that scope's token.
    pub(crate) scope_depth: usize,
    /// The stack slot that holds the record this binding's drop closed and took
    /// (`RESOURCE_OWNER_TAKEN`), until the reclaim that follows the last cleanup
    /// of the same exit frees it (`CodeBuilder::emit_resource_reclaim`). `Some`
    /// for every binding of a record-freeing kind: the scope that closes a
    /// resource frees its record, decided by the owner word alone.
    pub(crate) reclaim_slot: Option<usize>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct ResourceUnionCleanup {
    pub(crate) name: String,
    /// `(tag, close_symbol)` per active variant; drop reads the union tag and
    /// calls the matching close op on the variant's resource pointer.
    pub(crate) variants: Vec<(usize, String)>,
    /// The union's uniform `STATE` type, when it declared one (plan-74). Carried
    /// from the bind so the tag-dispatched drop can free the active variant
    /// record's STATE block after the close — mirrors `ResourceCleanup.state_type`.
    pub(crate) state_type: Option<ParameterType>,
    /// bug-1001: the variant tags whose record carries the two `File` I/O buffers,
    /// which the drop frees after the close as the concrete drop does.
    pub(crate) io_buffer_tags: Vec<usize>,
    /// bug-623 B: the variant tags whose record this binding's drop frees after the
    /// close — the `resource_record_freed_at_drop` variants, and only when the binding
    /// owns the record (`record_owning_locals`). Empty for a union wrapping a live
    /// binding, whose record that binding frees.
    pub(crate) record_free_tags: Vec<usize>,
    /// Resource-union alias class: whether the drop closes the active variant (and frees
    /// its STATE). False for a union wrapping an aliasing source (`RES c AS Union = u`),
    /// whose record another binding owns: that drop frees only the union's own box.
    pub(crate) closes_variant: bool,
    /// bug-648: the same run-time ownership flag as `ResourceCleanup.owner_flag_slot`.
    pub(crate) owner_flag_slot: Option<usize>,
    /// bug-1051: as `ResourceCleanup.default_record_slot`, for the closed default
    /// record the temp's default box names.
    pub(crate) default_record_slot: Option<usize>,
    /// bug-755: as `ResourceCleanup.scope_depth`, for the active variant's record.
    pub(crate) scope_depth: usize,
    /// As `ResourceCleanup::reclaim_slot`: the slot that holds the box of the
    /// record this binding's drop closed and took, until the reclaim after the
    /// exit's last cleanup frees the record's blocks and the box
    /// (`CodeBuilder::emit_resource_union_reclaim`).
    pub(crate) reclaim_slot: Option<usize>,
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
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum OwnedListDrop {
    /// bug-843: a list whose nodes may hold any kind a closure can carry
    /// (`closure_resource_kinds`). Each node records its own kind (and a union
    /// leaf's active-variant tag) beside the record, because a `FUNC` holder's type
    /// does not name what its closures captured. See `closure_carry`.
    Dynamic,
    /// A single registered close op applied to each node's record pointer, plus what
    /// the drain may reclaim behind it (bug-645): the element's uniform `STATE` block,
    /// its two per-`File` I/O buffers, and — for a `resource_record_freed_at_drop`
    /// kind — the resource record. Exactly the descriptor `ActiveCleanup::Resource`
    /// carries for a lone binding, because the reclaim is the same one.
    Concrete {
        close: String,
        state_type: Option<ParameterType>,
        has_io_buffers: bool,
        frees_record: bool,
    },
    /// A resource-union element: `(tag, close_symbol)` per variant, plus the
    /// union's uniform `STATE` type (when it has one) to free after the close, and the
    /// variant tags whose record the drain may free (bug-645).
    Union {
        variants: Vec<(usize, String)>,
        state_type: Option<ParameterType>,
        /// bug-1001: the variant tags whose record carries `File` I/O buffers.
        io_buffer_tags: Vec<usize>,
        record_free_tags: Vec<usize>,
    },
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct OwnedListCleanup {
    /// The owning collection binding's name (for transfer-on-return lookup).
    pub(crate) name: String,
    /// Stack offset of the list head pointer (0 when empty).
    pub(crate) head_slot: usize,
    /// How each node's resource is closed (concrete close op vs union dispatch).
    pub(crate) drop: OwnedListDrop,
    /// bug-645: whether this collection provably owns each element's blocks, so the
    /// drain may free them after the close. False keeps the close-only drain — a
    /// bounded leak rather than a double free (`record_ownership::owning_collections`).
    pub(crate) owns_elements: bool,
    /// bug-755: the nesting depth of the owner collection's scope. A floated or
    /// seeded record takes this scope's token, and the drain closes only the
    /// records that still hold it.
    pub(crate) scope_depth: usize,
    /// bug-805: the resource leaf type (`RES` stripped, `STATE` kept) this list
    /// holds; `drop` is that leaf's descriptor.
    pub(crate) leaf: ParameterType,
    /// bug-1053: for a list whose container is not proved the one holder of what
    /// it lists (`owns_elements` false), the stack slot heading the nodes whose
    /// records its drain has closed and taken, and that are freed once every
    /// cleanup of the exit has run (`CodeBuilder::emit_owned_list_reclaims`).
    /// `None` keeps the close-only drain.
    pub(crate) reclaim_slot: Option<usize>,
}

/// bug-778: the frame representation of an inline-`TRAP` desugar temporary
/// (`$trap_resN : Result OF T`) whose uses are only the three `Result` readers.
///
/// The heap form is a `{tag, size, payload}` block allocated on every evaluation,
/// success included. This form keeps the same three facts in frame slots:
///
/// * `tag_slot` — the binding's own slot; `RESULT_OK_TAG` or an error tag.
/// * `payload_slot` — on `Ok`, the success value: the scalar itself, or the
///   pointer to a block this temporary OWNS. Zero on the error path. A block is
///   MOVED out by `ResultValue` (the slot is zeroed), so it is never copied in
///   or out; the slot's own drop frees one that was never read.
/// * `error_slot` — on the error path, the flat `Error` block this temporary
///   owns (adopted from the raiser or rebuilt from the registers, exactly what
///   the heap form copied inline). Moved out by `ResultError`; zero otherwise.
/// bug-778: which `Result` readers a [`UnboxedTrapResult`] candidate has —
/// at most one value read and one error read, each in store position
/// (`unboxed_trap_result_candidates`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct UnboxedTrapUses {
    pub(crate) reads_value: bool,
    pub(crate) reads_error: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct UnboxedTrapResult {
    pub(crate) tag_slot: usize,
    pub(crate) payload_slot: usize,
    pub(crate) error_slot: usize,
    /// The success type `T`.
    pub(crate) payload: ParameterType,
    /// Whether `T` is a block (`result_payload_is_block`), i.e. `payload_slot`
    /// holds an owned pointer rather than a scalar.
    pub(crate) block: bool,
    /// Whether a `ResultError` reads the error. When none does, the error path
    /// frees the `Error` block at once (the heap form's guarded free) and
    /// `error_slot` is unused.
    pub(crate) error_read: bool,
}

/// An owned, non-escaping flat value freed at scope-drop (plan-01 Phase 5 /
/// plan-02 Phase 8). Because every non-resource value is a pointer-free flat
/// block and copy-insertion (`lower_value_owned`) makes each owner's block
/// unaliased, a single `arena_free(ptr, size)` reclaims the whole value; the
/// size is recomputed from the static type at drop via
/// `emit_inlined_block_size_from_ptr_slot`.
/// bug-1036: one entry of [`CodeBuilder::for_each_entry_blocks`].
#[derive(Clone, Debug)]
pub(crate) struct ForEachEntryBlock {
    /// The binding the loop walks.
    pub(crate) name: String,
    /// The loop's `for_each_collection` slot: the block it walks.
    pub(crate) collection_slot: usize,
    /// Null until a reassignment replaces that block; then the block, freed when the
    /// loop ends.
    pub(crate) deferred_slot: usize,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct OwnedValueCleanup {
    /// Static type of the bound value (drives the runtime size computation).
    pub(crate) type_: ParameterType,
    /// Stack offset of the binding's slot (holds the block pointer).
    pub(crate) stack_offset: usize,
    /// bug-560: for a `String` binding that an in-place self-append targets, the
    /// frame offset of its capacity shadow (`string_capacity_slots`). The block in
    /// `stack_offset` was then allocated at `byteLength + spare + 9`, not
    /// `byteLength + 9`, so the drop must add the shadow's spare bytes or it
    /// under-frees and orphans the headroom on every drop.
    ///
    /// FAIL-CLOSED: `None` keeps the historical tight size. It is set only where
    /// the binding's NAME is in scope and `prescan_string_self_appends` has
    /// already claimed a shadow for it, so an unrecognized shape keeps leaking
    /// rather than freeing bytes it cannot prove were allocated.
    pub(crate) capacity_slot: Option<usize>,
    /// bug-571: for a `FOR EACH` loop item, the frame offset holding the ALIAS
    /// pointer the payload load would have returned had it not materialised —
    /// `emit_load_collection_payload_with_alias_base`'s second register, spilled
    /// once per iteration.
    ///
    /// A `FOR EACH` element is immutable and, for every payload type but `String`,
    /// IS a pointer into the container's own block; freeing that corrupts the
    /// collection. The drop compares the two at runtime and frees only when they
    /// differ, so "the loop materialised this" is decided by the emitter that
    /// materialised it rather than by a second copy of its type enumeration —
    /// `collections::reduce`'s model (`gen_memory.rs`), applied to the one value
    /// the loop provably does not own.
    ///
    /// `None` everywhere else: an ordinary binding's block has no container.
    pub(crate) loop_alias_slot: Option<usize>,
    /// bug-593: `Some` when this cleanup releases ONLY the `{tag, size, payload}`
    /// wrapper an inline `TRAP` built for a `Result OF T` whose `T` is not a flat
    /// value (a resource, or a collection of a recursive type), so
    /// `is_freeable_flat_value` gave the bind no drop at all.
    /// The drop frees the one wrapper block by the size word at +8, walking into its
    /// payload only for a recursive one; see [`ResultWrapperDrop`] for which paths it covers.
    ///
    /// `None` everywhere else.
    pub(crate) result_wrapper: Option<ResultWrapperDrop>,
}

/// bug-593: which runtime paths an inline-`TRAP` `Result` wrapper drop releases.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum ResultWrapperDrop {
    /// The payload word at +16 is loaded BY VALUE (`result_payload_is_block` is
    /// false — a resource handle), so nothing can alias into the wrapper on
    /// either path: release it whatever its tag.
    Always,
    /// The Ok payload is a BLOCK inlined at +16 that `ResultValue` hands out as an
    /// alias (`wrapper + 16`), and a non-flat `T` is not deep-copied by
    /// `lower_value_owned` — so on the Ok path the binding still points into the
    /// wrapper and it must NOT be released. On the error path the wrapper holds
    /// only the trapped `Error`, which every reader copies out
    /// (`ResultError` is an aliasing source) exactly as it does for a flat `T`,
    /// so it is released when the tag is not Ok.
    ErrorOnly,
    /// plan-134: the Ok payload is a recursive value (`owns_graph`) inlined at +16 — its top
    /// block's bytes, pointing into a graph only the wrapper holds. Nothing aliases into it:
    /// `ResultValue` is an aliasing source, so every owning store graph-copies it
    /// (`lower_value_owned`, plan-134-D). On the Ok path the payload's edges are dropped
    /// (`_mfb_rt_graph_drop_edges`, its bytes stay with the wrapper), and the wrapper is
    /// released whatever its tag.
    OkGraphPayload,
}

/// Rule 5 (`mfb spec language resource-management`): what each call result that
/// handed its statement resources registered. When a call to a function body
/// returns, every record its result reaches that the callee released is taken by
/// the caller's statement (a scope one inside the current one) and listed; at
/// the end of the statement, on every exit from it, the lists are drained: a
/// record the statement still owns is closed and freed, one a longer-lived taker
/// raised is left to it.
#[derive(Default)]
pub(crate) struct StatementTemps {
    /// Per temporary (by its `PendingTemp` slot): the owned-lists it seeded, one
    /// per resource leaf kind.
    pub(crate) drops: HashMap<usize, Vec<OwnedListCleanup>>,
    /// Per temporary whose value IS a resource (or a resource-union box): the
    /// statement's scope depth, whose token the record took when the call
    /// returned.
    pub(crate) leaf_depths: HashMap<usize, usize>,
    /// The container the assignment being lowered stores into, for the in-place
    /// lowerings' item float (`CodeBuilder::emit_stored_item_float`).
    pub(crate) assign_target: Option<String>,
    /// Set when the store floated the one item it added, so the assignment's
    /// statement-end float of the whole container is not needed.
    pub(crate) item_floated: bool,
    /// Whether the assignment being lowered is one of `StoreFloats::stores`.
    pub(crate) assign_floats: bool,
    /// bug-1075: the locals of this function that a `LAMBDA` captures by
    /// reference and that reach a resource. Such a binding's stack object is a
    /// three-word cell, `{value, owned-list head, scope token}`, so a store
    /// through the reference (in the lambda's frame) can raise what it stores
    /// to the binding's scope and list it there
    /// (`CodeBuilder::emit_captured_store_float`).
    pub(crate) ref_cells: HashSet<String>,
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
    /// plan-134-G: free only this block, not the graph it points into — a recursive value whose
    /// top block a store byte-copied, so its children now belong to that store's owner.
    pub(crate) shallow: bool,
    /// bug-842: a resource temporary — a producer call's resource result that no
    /// binding took. Its drop closes the resource when it is still unowned (and not
    /// already closed) instead of freeing a block (`emit_resource_temp_close`); its
    /// identity token is a register no owner names, so no claim can take it.
    pub(crate) resource: bool,
    /// bug-1012: a resource-union `{tag, record-ptr}` box this statement made — a
    /// union wrap, or the box a called function handed back — that no binding,
    /// `RETURN` or module-level store took. Its drop frees the 16-byte box and
    /// nothing else: the record it names has its own owner, or is a resource
    /// temporary (`resource`) with an entry of its own.
    pub(crate) union_box: bool,
}

/// bug-781: one capturing `LAMBDA`'s environment, as its routine copies and drops
/// it: `[routine address][capture words…][auxiliary words…]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClosureEnvLayout {
    /// The routine's symbol, `closure_env_ops_symbol` of the lifted `LAMBDA`.
    pub(crate) symbol: String,
    /// One entry per capture word, in capture order.
    pub(crate) words: Vec<ClosureEnvWord>,
    /// The compiler-managed words after the captures — the address of a slot in
    /// the creating frame (a borrowed self-update scratch, a shared capacity
    /// shadow, the `os::appResourcePath` base cache). Never owned.
    pub(crate) auxiliary_words: usize,
    /// bug-843: the by-value capture words whose type reaches a resource, as
    /// `(capture index, type)` — what the routine's resource operations walk.
    /// Empty unless the module's closures carry resources.
    pub(crate) resource_words: Vec<(usize, ParameterType)>,
}

/// bug-781: what one capture word of an environment holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClosureEnvWord {
    /// A value of this type the environment owns: copied deep with the
    /// environment, dropped with it.
    Owned(ParameterType),
    /// A word copied verbatim and never freed: a scalar, a by-ref capture's slot
    /// address, a resource or thread handle.
    Plain,
    /// bug-843: a captured resource union. A union value is a 16-byte
    /// `{tag@0, record@8}` box owned by the binding or statement that made it and
    /// freed at that owner's end whoever owns the record (bug-1012), so the
    /// environment holds its own copy of the box: copied with the environment,
    /// freed with it, and never closed through (the record belongs to its own,
    /// possibly raised, scope).
    ResourceUnionBox,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum ActiveCleanup {
    Resource(ResourceCleanup),
    ResourceUnion(ResourceUnionCleanup),
    OwnedList(OwnedListCleanup),
    OwnedValue(OwnedValueCleanup),
}

/// bug-695: one element of an error-return landing pad's key. A drop is fully
/// determined by its cleanup plus, for a name-addressed cleanup (a resource, a
/// resource union), the stack slot that name resolves to when the pad is emitted,
/// so two raise sites may share a pad only when both agree.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct ErrorReturnPadEntry {
    pub(crate) cleanup: ActiveCleanup,
    pub(crate) local_slot: Option<usize>,
}

/// bug-695: one error-return landing pad awaiting placement after the body.
pub(crate) struct ErrorReturnPad {
    /// The length of the cleanup-stack prefix this pad drops down from.
    pub(crate) depth: usize,
    /// The pad's own label (the first instruction of `code`).
    pub(crate) label: String,
    /// The pad it continues into, `None` for the empty prefix's (which returns).
    pub(crate) successor: Option<String>,
    pub(crate) code: Vec<CodeInstruction>,
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
    /// plan-140-C: each enum's member names in declaration (ordinal) order,
    /// derived from `enum_members` once the model is finished. The ONE ordering
    /// both sides of `toString(<enum>)` read — the string pre-pass registers
    /// these names and `lower_enum_to_string` loads them.
    pub(crate) enum_names: HashMap<ParameterType, Vec<String>>,
    /// plan-111-C: keyed by the record TYPE. The `String` in the value is a
    /// FIELD name.
    pub(crate) record_fields: HashMap<ParameterType, Vec<(String, ParameterType)>>,
    pub(crate) union_names: HashSet<ParameterType>,
    /// A union VARIANT type -> the union type that declares it. Both halves
    /// are types (`builder_value_semantics/` looks a variant up and then reads
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
    /// The subset of [`Self::resource_names`] whose declaration opted into
    /// crossing a thread boundary — `RESOURCE Db CLOSE BY sql::close
    /// THREAD_SENDABLE` (17_native-libraries.md), or an imported package's
    /// `RESOURCE_TABLE` row with the sendable bit set.
    ///
    /// bug-546: codegen's two resource predicates
    /// (`builtins::is_resource_type`, `builtins::is_thread_sendable_resource_type`)
    /// answer for the BUILT-IN registry only, so every user-declared resource was
    /// invisible to both. The sendable half decides which arm of
    /// `emit_thread_copy_real` a handle takes — deep-copy into the receiver's
    /// arena, or carry the pointer move-only — and getting it from the
    /// declaration is what keeps codegen from sending a resource its author never
    /// marked, which is the frontend's own rule.
    pub(crate) sendable_resource_names: HashSet<ParameterType>,
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
    /// plan-134-H: the recursive-value drop walker's kinds, as rendered type names —
    /// `recursive_transfer_types` in its order, then the resource-free record, union and
    /// collection types that only reach a cycle (`graph_drop_kind_names`). Empty for a model
    /// built without a module.
    pub(crate) graph_drop_kinds: Vec<String>,
    /// bug-843: the type of every by-value `LAMBDA` capture in the module that can
    /// reach a resource (`closure_carry::closure_capture_types`), in first-seen
    /// order. A `FUNC` type does not say what its closure captured, so this
    /// module-wide set is what tells codegen a function value can carry resource
    /// references at all — and which kinds. Empty for nearly every program, which
    /// then lowers exactly as before.
    pub(crate) closure_capture_types: Vec<ParameterType>,
}
