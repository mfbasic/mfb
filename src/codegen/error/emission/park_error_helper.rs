//! The shared error-block park (plan-118-E phase 2).
//!
//! Every fallible operation that fails assembles an error the same way: stage
//! the code, message and source, call `_mfb_make_error_result`, then **build an
//! owned `Error` block and park it** in the arena's current-error slot so
//! whoever catches the error adopts it instead of rebuilding it. Only the first
//! part varies per site (the code, the message symbol, and the line/column of
//! the failure). The park does not vary at all: it reads three fixed registers,
//! allocates, copies, writes one arena slot, and restores the registers.
//!
//! It was inlined at every one of those sites anyway — **~174 machine
//! instructions each**, measured by dumping `FUNC cat2(a, b) RETURN a & b`,
//! where 194 of the function's 218 instructions were the error path and all but
//! ~20 of those were the park. That is the single shape plan-118-B/-C/-D kept
//! bottoming out on: after out-of-lining the concat, the renderers, the print
//! marshalling and the record constructors, what was left at each of those sites
//! was this.
//!
//! # Why one function per module, not a block per function
//!
//! plan-118-E §3 designs this as per-function blocks that sites jump to. That is
//! necessary for the *cleanup epilogue* (phase 3), whose content depends on the
//! function's live scope — but not here. The park closes over nothing: its
//! inputs are the error registers, its scratch is its own frame, and its one
//! side effect is a store through `ARENA_STATE_REGISTER`, which is per-thread
//! and callee-saved. So it is a `bl` to a single synthesized function, which is
//! both a smaller blast radius and a smaller module than 1,600 copies of the
//! same block would be.
//!
//! The OOM path inside is unchanged: building the block is itself an allocation,
//! and its failure sets the loose `RESULT_ERR_TAG` and returns rather than
//! recursing (the `building_error_block` guard). Inside the helper that reads as
//! "return to the site with a loose error", which is exactly what the inline
//! version left in the registers.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirFunction;
use crate::types::ParameterType;
use std::collections::HashMap;

/// Synthesize `_mfb_rt_park_error`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_park_error_helper<'a>(
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, ParameterType>,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &'a HashMap<String, GlobalValue>,
    string_symbols: &'a HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let mut builder = CodeBuilder::for_synthetic_function(
        PARK_ERROR_SYMBOL,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    );
    // The very sequence the sites used to inline, so the two cannot drift.
    builder.emit_park_error_block_from_registers()?;
    builder.emit(abi::return_());

    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
    let is_x86 = crate::codegen::engine::mir::active_backend()
        .register_model()
        .arena_base()
        == crate::arch::x86_64::regmodel::ARENA_BASE_REGISTER;
    crate::optimizer::opt2::peephole::forward_stores_to_loads(&mut instructions, is_x86);
    crate::optimizer::opt2::peephole::remove_fp_shuttles(
        &mut instructions,
        crate::codegen::engine::mir::active_backend().register_model(),
    );
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );
    Ok(CodeFunction {
        name: "runtime.parkError".to_string(),
        symbol: PARK_ERROR_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

/// bug-573: where the `ErrorLoc` in `RESULT_ERROR_SOURCE_REGISTER` came from at
/// a park site.
///
/// `_mfb_rt_park_error` inlines a COPY of that `ErrorLoc` into the owned `Error`
/// block it builds, and then releases the original — ~200 B per raised error for
/// `src/main.mfb`, ~682 B for a 117-character path, because the filename is
/// inlined into it. That release is only sound because every park site hands it a
/// block THIS FRAME just allocated, and the helper is shared, so it cannot ask.
///
/// An `ErrorLoc` pointer is NOT always fresh elsewhere in the compiler:
/// `emit_load_error_fields` produces an INTERIOR pointer (`errorBase + offset`)
/// into somebody else's `Error` block, and `emit_direct_error_return` puts exactly
/// that in `RESULT_ERROR_SOURCE_REGISTER` for a `FAIL <Error value>` — which takes
/// the legacy loose-`ERR` route and never reaches the park. Freeing one of those
/// would be a free of an interior pointer, which corrupts the arena's free list
/// rather than merely leaking.
///
/// So each site declares its provenance and [`CodeBuilder::emit_park_error_call`]
/// matches on it exhaustively, with no wildcard: a fourth park site is a build
/// error until somebody answers this question for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParkedErrorSource {
    /// `emit_error_register_return` — a domain error raised HERE. The source is
    /// whatever `_mfb_make_error_result` returned, which is a fresh
    /// `_mfb_build_error_loc` block (or null on OOM).
    MakeErrorResult,
    /// `emit_stamp_current_error_source` — a native runtime helper returned an
    /// error with no origin of its own, and the call site stamped one with a fresh
    /// `emit_build_error_loc`.
    StampedCallSite,
    /// `emit_finalize_worker_error_source` — a propagated `thread::waitFor` error.
    /// The WORKER's `ErrorLoc` has already been deep-copied into THIS arena by
    /// `copy_value_to_current_arena` (or, when `waitFor` raised its own error,
    /// stamped with `emit_build_error_loc`); either way the pointer the park sees
    /// is this frame's copy, never the worker's original. `x19` is per-thread, so
    /// the distinction is the difference between a free and cross-arena
    /// corruption.
    WorkerArenaCopy,
}

/// Whether the park may release the `ErrorLoc` it was handed.
///
/// One value today — the enum exists so the answer is written down per site
/// rather than inherited, and so a `Borrowed` provenance cannot be added without
/// the `match` below failing to compile.
enum ParkedSourceOwnership {
    /// A block this frame allocated, with no other owner once the park has
    /// inlined its copy.
    OwnedByThisFrame,
}

impl CodeBuilder<'_> {
    /// `bl _mfb_rt_park_error` — the call-site form of
    /// [`Self::emit_park_error_block_from_registers`].
    ///
    /// `source` declares where this site's `RESULT_ERROR_SOURCE_REGISTER` came
    /// from (bug-573). The park releases it, so the declaration is load-bearing.
    pub(crate) fn emit_park_error_call(&mut self, source: ParkedErrorSource) {
        // Exhaustive, no wildcard (bug-567's shape): a new `ParkedErrorSource`
        // is a build error here, not a silent free of a block somebody else owns.
        let ownership = match source {
            ParkedErrorSource::MakeErrorResult
            | ParkedErrorSource::StampedCallSite
            | ParkedErrorSource::WorkerArenaCopy => ParkedSourceOwnership::OwnedByThisFrame,
        };
        match ownership {
            ParkedSourceOwnership::OwnedByThisFrame => {}
        }
        self.emit(abi::branch_link(PARK_ERROR_SYMBOL));
        self.push_internal_call_relocation(PARK_ERROR_SYMBOL);
    }
}

/// Synthesize `_mfb_rt_drop_owned_string` — the scope-drop of one owned `String`
/// (plan-118-E phase 3).
///
/// `String` is by far the commonest owned-value cleanup, and its drop is eleven
/// instructions at every exit: load the slot, null-test, branch, read the block
/// header for the length, add the 9-byte `{u64 len; bytes; NUL}` overhead,
/// reload the pointer, load the size, `bl _mfb_arena_free`, null the slot. All
/// of it is a pure function of the slot address, so all of it moves here and the
/// site becomes `add x0, sp, #slot` + `bl` — **two** instructions.
///
/// Taking the slot ADDRESS rather than the pointer is what lets the helper own
/// the free-and-null: nulling the slot after the free is what stops a re-reached
/// drop (a loop body whose owned temp came from a conditionally-evaluated
/// initializer) from double-freeing a stale pointer — bug-440, and the reason
/// the guard cannot simply be dropped.
///
/// Why this and not §3's per-function chained cleanup epilogue: see the plan's
/// Corrections. In short, Phase 1 found that a function's live cleanup set is
/// NOT a function of scope depth — `plan_returned_move`,
/// `deactivate_thread_cleanup`, `deactivate_resource_cleanup` and
/// `deactivate_owned_list` each remove cleanups per RETURN depending on which
/// local escapes — so "one block per depth" cannot be shared soundly. Sharing
/// the individual drop instead needs no scope reasoning at all.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_drop_owned_string_helper<'a>(
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, ParameterType>,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &'a HashMap<String, GlobalValue>,
    string_symbols: &'a HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    lower_drop_owned_string_body(
        false,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    )
}

/// bug-560: synthesize `_mfb_rt_drop_owned_string_cap` — the same drop for a
/// `String` binding that is the target of an in-place self-append, so its block
/// carries geometric capacity headroom the `byteLength` header does not record.
///
/// `x1` is the binding's capacity shadow (spare bytes), so the freed size is
/// `byteLength + spare + 9`: exactly the size
/// `lower_string_self_append_one`'s regrow passed to `arena_alloc`. It is a
/// SEPARATE symbol rather than an extra argument to the plain helper so every
/// existing drop site stays byte-identical — only a function that actually
/// self-appends a `String` emits this one.
///
/// The size can only ever be LARGER than the tight one, never smaller, and only
/// by the shadow the regrow itself wrote — so this corrects an under-free and
/// can never over-free.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_drop_owned_string_cap_helper<'a>(
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, ParameterType>,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &'a HashMap<String, GlobalValue>,
    string_symbols: &'a HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    lower_drop_owned_string_body(
        true,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    )
}

/// The one body behind both owned-`String` drops. `with_capacity` adds the
/// shadow's spare bytes to the freed size and nothing else; the vreg allocation
/// order of the plain form is unchanged, so its codegen is byte-identical.
#[allow(clippy::too_many_arguments)]
fn lower_drop_owned_string_body<'a>(
    with_capacity: bool,
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, ParameterType>,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &'a HashMap<String, GlobalValue>,
    string_symbols: &'a HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let symbol = if with_capacity {
        DROP_OWNED_STRING_CAP_SYMBOL
    } else {
        DROP_OWNED_STRING_SYMBOL
    };
    let mut builder = CodeBuilder::for_synthetic_function(
        symbol,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    );
    let done = builder.label("drop_owned_string_done");
    let slot = builder.allocate_register();
    let ptr = builder.allocate_register();
    let size = builder.allocate_register();
    // bug-560: the spare-byte count arrives in `x1`; capture it before the
    // header load and the free call reuse the argument bank. Allocated AFTER the
    // three registers above so the plain form's vreg numbering is untouched.
    let spare = if with_capacity {
        let spare = builder.allocate_register();
        builder.emit(abi::move_register(&spare, abi::c_arg(1)));
        Some(spare)
    } else {
        None
    };
    builder.emit(abi::move_register(&slot, abi::c_arg(0)));
    builder.emit(abi::load_u64(&ptr, &slot, 0));
    builder.emit(abi::compare_immediate(&ptr, "0"));
    builder.emit(abi::branch_eq(&done));
    // `mfb.string.v1` is `{u64 byteLength; bytes; NUL}`, so the block is
    // `byteLength + 9` — the same arithmetic
    // `emit_inlined_block_size_from_ptr_slot` performs for `ParameterType::String`.
    builder.emit(abi::load_u64(&size, &ptr, 0));
    builder.emit(abi::add_immediate(&size, &size, 9));
    // …plus the self-append headroom the header cannot record. `spare` is the
    // binding's capacity shadow, which `lower_string_self_append_one` keeps equal
    // to `alloc_payload - byteLength` for the block currently in the slot, so
    // `byteLength + spare + 9` is the allocated size exactly.
    if let Some(spare) = &spare {
        builder.emit(abi::add_registers(&size, &size, spare));
    }
    builder.emit(abi::move_register(abi::c_arg(0), &ptr));
    builder.emit(abi::move_register(abi::c_arg(1), &size));
    builder.emit_arena_free_call();
    // Free-and-null (bug-440): a drop re-reached without an intervening store
    // must see 0 and skip rather than free the stale pointer again.
    builder.emit(abi::store_u64(abi::ZERO, &slot, 0));
    builder.emit(abi::label(&done));
    builder.emit(abi::return_());

    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
    let is_x86 = crate::codegen::engine::mir::active_backend()
        .register_model()
        .arena_base()
        == crate::arch::x86_64::regmodel::ARENA_BASE_REGISTER;
    crate::optimizer::opt2::peephole::forward_stores_to_loads(&mut instructions, is_x86);
    crate::optimizer::opt2::peephole::remove_fp_shuttles(
        &mut instructions,
        crate::codegen::engine::mir::active_backend().register_model(),
    );
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );
    Ok(CodeFunction {
        name: if with_capacity {
            "runtime.dropOwnedStringCap".to_string()
        } else {
            "runtime.dropOwnedString".to_string()
        },
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

/// Synthesize `_mfb_rt_drop_owned_collection` — the scope-drop of one owned flat
/// `List`/`Map`/`Set` (plan-118-E phase 3), the other cleanup shape that recurs
/// at every exit.
///
/// The flat size formula
/// (`builder_collection_layout.rs::emit_flat_block_size`) varies on exactly two
/// things: the entry stride, and whether the block carries a hash-bucket region
/// (`Map`/`Set` do, `List` does not). Both are compile-time constants at the
/// site, so they become arguments and the twelve-instruction drop becomes four.
///
/// The formula is reproduced here deliberately, not approximated. Its own source
/// carries the warning that this is "the one edit whose mistake is heap
/// corruption rather than a wrong value" — a wrong stride frees past the end of
/// the block and corrupts the arena free list (bug-02) — so this uses the same
/// named layout constants that emitter uses, and a layout change must move both.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_drop_owned_collection_helper<'a>(
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, ParameterType>,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &'a HashMap<String, GlobalValue>,
    string_symbols: &'a HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let mut builder = CodeBuilder::for_synthetic_function(
        DROP_OWNED_COLLECTION_SYMBOL,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    );
    let done = builder.label("drop_owned_collection_done");
    let no_buckets = builder.label("drop_owned_collection_no_buckets");
    let slot = builder.allocate_register();
    let stride = builder.allocate_register();
    let buckets = builder.allocate_register();
    let ptr = builder.allocate_register();
    let size = builder.allocate_register();
    let scratch = builder.allocate_register();
    // Park the three arguments before the free call can clobber the bank.
    builder.emit(abi::move_register(&slot, abi::c_arg(0)));
    builder.emit(abi::move_register(&stride, abi::c_arg(1)));
    builder.emit(abi::move_register(&buckets, abi::c_arg(2)));
    builder.emit(abi::load_u64(&ptr, &slot, 0));
    builder.emit(abi::compare_immediate(&ptr, "0"));
    builder.emit(abi::branch_eq(&done));
    // header + capacity * entryStride + dataCapacity (+ the bucket region).
    builder.emit(abi::load_u64(&size, &ptr, COLLECTION_OFFSET_CAPACITY));
    builder.emit(abi::multiply_registers(&size, &size, &stride));
    builder.emit(abi::add_immediate(&size, &size, COLLECTION_HEADER_SIZE));
    builder.emit(abi::load_u64(
        &scratch,
        &ptr,
        COLLECTION_OFFSET_DATA_CAPACITY,
    ));
    builder.emit(abi::add_registers(&size, &size, &scratch));
    builder.emit(abi::compare_immediate(&buckets, "0"));
    builder.emit(abi::branch_eq(&no_buckets));
    builder.emit(abi::load_u64(&scratch, &ptr, COLLECTION_OFFSET_CAPACITY));
    builder.emit(abi::shift_left_immediate(&scratch, &scratch, 4));
    builder.emit(abi::add_registers(&size, &size, &scratch));
    builder.emit(abi::label(&no_buckets));
    builder.emit(abi::move_register(abi::c_arg(0), &ptr));
    builder.emit(abi::move_register(abi::c_arg(1), &size));
    builder.emit_arena_free_call();
    builder.emit(abi::store_u64(abi::ZERO, &slot, 0));
    builder.emit(abi::label(&done));
    builder.emit(abi::return_());

    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
    let is_x86 = crate::codegen::engine::mir::active_backend()
        .register_model()
        .arena_base()
        == crate::arch::x86_64::regmodel::ARENA_BASE_REGISTER;
    crate::optimizer::opt2::peephole::forward_stores_to_loads(&mut instructions, is_x86);
    crate::optimizer::opt2::peephole::remove_fp_shuttles(
        &mut instructions,
        crate::codegen::engine::mir::active_backend().register_model(),
    );
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );
    Ok(CodeFunction {
        name: "runtime.dropOwnedCollection".to_string(),
        symbol: DROP_OWNED_COLLECTION_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}
