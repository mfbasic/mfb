//! The `arena` report section (plan-130-C): a process-global registry of every arena
//! state — the main thread's, each `thread::start` worker's, and the canvas graphics
//! thread's — with a set of allocator event counters per arena.
//!
//! The registry is a region mapped on first registration through the platform's
//! `emit_arena_map` seam (system memory, never an arena), holding a
//! `{count, overflow, t0}` header and [`ARENA_DEBUG_SLOTS`] fixed-size slots
//! `{state_ptr, kind, counters[22], series}`; its base lives in a writable global.
//! `t0` is the monotonic clock when the main arena registered. The series (plan-133-C)
//! is `{count, stride, until, pending}` and 257 samples `{t_ns, mapped_bytes,
//! live_bytes, peak_rss_bytes}`: every grow writes the entry after the kept ones
//! through `_mfb_debug_arena_sample`, keeps it when the grow count reaches `until`,
//! and when 256 are kept halves them (the even entries plus the latest) and doubles the
//! stride, so the series stays bounded and spans the whole run. Registration
//! is the only locked operation (a statically initialized process-global mutex, so it
//! exists before the region does). Counters are written only by the thread that owns
//! the arena — the allocator helpers find their slot by the arena register — so they
//! need no lock. The report reads the header and slots unlocked: a still-running
//! worker's slot is a word-sized snapshot.
//!
//! Report lines: `arena.count <n>`, `arena.registry_overflow <n>`, then for each
//! registered arena in registration order (the main arena is `0`):
//! `arena.<n>.kind <main|worker|graphics>` and one `arena.<n>.<counter> <value>` line
//! per [`ARENA_COUNTERS`] entry.

use std::collections::HashMap;

use super::clock::{emit_debug_monotonic_nanos, CLOCK_BUFFER_SIZE};
use super::process::{emit_debug_peak_rss, PEAK_RSS_BUFFER_SIZE};
use super::write::{
    emit_debug_key_value, emit_prepend_decimal, emit_prepend_object, emit_write_window, key_object,
    DEBUG_LINE_BUFFER_SIZE,
};
use super::{DebugEmitCtx, DebugFeature};
use crate::codegen::engine::builder::{internal_branch, EmitCtx};
use crate::codegen::engine::types::{
    CodeDataObject, CodeFunction, CodeInstruction, CodeRelocation, CodegenPlatform, PlatformFamily,
};
use crate::codegen::engine::util::{finalize_vreg_body_with_locals, Vregs};
use crate::codegen::error::constants::ARENA_STATE_REGISTER;
use crate::codegen::memory::data::{push_symbol_address, string_data_object};
use crate::codegen::runtime::thread::emit_thread_external_call;
use crate::target::shared::abi;
use crate::target::shared::nir::NirModule;

mod report;
mod sample;
use report::lower_report;
use sample::lower_sample;

/// The section's name; `feature_active(module, ARENA_SECTION)` decides whether the
/// thread and canvas start paths register their arenas and the allocator counts.
pub(crate) const ARENA_SECTION: &str = "arena";

/// `_mfb_debug_arena_register(state, kind)`: record one arena state in the registry.
pub(crate) const DEBUG_ARENA_REGISTER_SYMBOL: &str = "_mfb_debug_arena_register";

/// `_mfb_debug_arena_sample(slot)`: record one memory-series sample for the arena whose
/// registry slot is `slot` (0: an unregistered arena, nothing recorded). Called by the
/// allocator's grow path in a `--debug` build (plan-133-C).
pub(crate) const DEBUG_ARENA_SAMPLE_SYMBOL: &str = "_mfb_debug_arena_sample";

/// The `kind` argument of the register helper (and the slot word it stores).
pub(crate) const ARENA_KIND_MAIN: &str = "0";
pub(crate) const ARENA_KIND_WORKER: &str = "1";
pub(crate) const ARENA_KIND_GRAPHICS: &str = "2";
/// plan-156-A: a thread control block's own arena — the block arena every value that
/// crosses between a parent and its worker is copied into.
pub(crate) const ARENA_KIND_THREAD_BLOCK: &str = "3";

const ARENA_REPORT_SYMBOL: &str = "_mfb_debug_report_arena";
pub(crate) const ARENA_BASE_SYMBOL: &str = "_mfb_rt_debug_arena_base";
/// plan-183-A: non-zero when the program was started with `MFB_ARENA_CHECK_ZERO` set to
/// a non-empty value. Read once, at the first arena registration (program entry,
/// before any thread exists); `arena_alloc` then checks every chunk it hands out.
pub(crate) const ARENA_CHECK_ZERO_SYMBOL: &str = "_mfb_rt_debug_arena_check_zero";
const ARENA_CHECK_ZERO_ENV_SYMBOL: &str = "_mfb_rt_debug_arena_check_zero_env";
/// The `MFB_ARENA_CHECK_ZERO` test switch's variable name.
pub(crate) const ARENA_CHECK_ZERO_ENV: &str = "MFB_ARENA_CHECK_ZERO";
const ARENA_LOCK_SYMBOL: &str = "_mfb_rt_debug_arena_lock";
const ARENA_COUNT_KEY_SYMBOL: &str = "_mfb_rt_debug_arena_key_count";
const ARENA_OVERFLOW_KEY_SYMBOL: &str = "_mfb_rt_debug_arena_key_overflow";
const ARENA_SLICES_KEY_SYMBOL: &str = "_mfb_rt_debug_arena_key_slices";
const ARENA_SLICE_SIZE_KEY_SYMBOL: &str = "_mfb_rt_debug_arena_key_slice_size";
const ARENA_PREFIX_SYMBOL: &str = "_mfb_rt_debug_arena_key_prefix";
const ARENA_KIND_SUFFIX_SYMBOL: &str = "_mfb_rt_debug_arena_key_kind";
const ARENA_TOKEN_MAIN_SYMBOL: &str = "_mfb_rt_debug_arena_token_main";
const ARENA_TOKEN_WORKER_SYMBOL: &str = "_mfb_rt_debug_arena_token_worker";
const ARENA_TOKEN_GRAPHICS_SYMBOL: &str = "_mfb_rt_debug_arena_token_graphics";
const ARENA_TOKEN_THREAD_BLOCK_SYMBOL: &str = "_mfb_rt_debug_arena_token_thread_block";
const ARENA_SERIES_INFIX_SYMBOL: &str = "_mfb_rt_debug_arena_series_infix";
const ARENA_SERIES_COUNT_KEY_SYMBOL: &str = "_mfb_rt_debug_arena_series_key_count";

/// Registry capacity. A program that registers more arenas than this loses their
/// slots (and so their counts); `arena.registry_overflow` reports how many.
pub(crate) const ARENA_DEBUG_SLOTS: usize = 1024;
pub(crate) const REGION_COUNT_OFFSET: usize = 0;
const REGION_OVERFLOW_OFFSET: usize = 8;
/// The monotonic clock (nanoseconds) when the main arena registered: every series
/// sample's `t_ns` is measured from it.
const REGION_T0_OFFSET: usize = 16;
pub(crate) const REGION_HEADER_SIZE: usize = 24;
pub(crate) const SLOT_KIND_OFFSET: usize = 8;

/// Counter words of a slot, after `state_ptr` (+0) and `kind` (+8). plan-183-C: the
/// region allocator's set — commits and decommits of the arena's slice, the path that
/// served each allocation, and the rows plan-183-D/E fill (in-place growth, the
/// slice's size).
pub(crate) const COUNTER_COMMITS: usize = 16;
pub(crate) const COUNTER_COMMITTED_BYTES: usize = 24;
pub(crate) const COUNTER_COMMITTED_PEAK_BYTES: usize = 32;
pub(crate) const COUNTER_DECOMMITS: usize = 40;
pub(crate) const COUNTER_DECOMMITTED_BYTES: usize = 48;
pub(crate) const COUNTER_ALLOC_CALLS: usize = 56;
pub(crate) const COUNTER_ALLOC_BYTES: usize = 64;
pub(crate) const COUNTER_FREE_CALLS: usize = 72;
pub(crate) const COUNTER_FREE_BYTES: usize = 80;
pub(crate) const COUNTER_LIVE_BYTES: usize = 88;
pub(crate) const COUNTER_PEAK_LIVE_BYTES: usize = 96;
pub(crate) const COUNTER_HIT_SMALL: usize = 104;
pub(crate) const COUNTER_SMALL_PAGES: usize = 112;
pub(crate) const COUNTER_HIT_MEDIUM_RUN: usize = 120;
pub(crate) const COUNTER_HIT_MEDIUM_FRONTIER: usize = 128;
pub(crate) const COUNTER_HIT_HUGE_RUN: usize = 136;
pub(crate) const COUNTER_HIT_HUGE_FRONTIER: usize = 144;
pub(crate) const COUNTER_DOUBLE_FREE_SKIPS: usize = 152;
/// plan-133-B: zero-fill calls and bytes, on the grow path (always 0 since
/// plan-183-A) and the free path (a freed chunk, whole since plan-183-C).
pub(crate) const COUNTER_FILL_GROW_CALLS: usize = 160;
pub(crate) const COUNTER_FILL_GROW_BYTES: usize = 168;
pub(crate) const COUNTER_FILL_FREE_CALLS: usize = 176;
pub(crate) const COUNTER_FILL_FREE_BYTES: usize = 184;
/// plan-183-A: with `MFB_ARENA_CHECK_ZERO` set, the chunks `arena_alloc` checked before
/// handing them out, and those with a non-zero byte in the checked range.
pub(crate) const COUNTER_ZERO_CHECKS: usize = 192;
pub(crate) const COUNTER_ZERO_VIOLATIONS: usize = 200;
pub(crate) const COUNTER_EXTEND_CALLS: usize = 208;
pub(crate) const COUNTER_EXTEND_HITS: usize = 216;
pub(crate) const COUNTER_RESERVED_BYTES: usize = 224;
/// plan-183-C: how often a region's frontier extended — a never-used small page set
/// up, a medium or huge chunk carved at its region's frontier. Each one samples the
/// memory series (`_mfb_debug_arena_sample` counts it).
pub(crate) const COUNTER_GROWS: usize = 232;

/// Every counter, in slot and report order: `(report name, slot offset)`.
const ARENA_COUNTERS: [(&str, usize); 28] = [
    ("reserved_bytes", COUNTER_RESERVED_BYTES),
    ("grows", COUNTER_GROWS),
    ("commits", COUNTER_COMMITS),
    ("committed_bytes", COUNTER_COMMITTED_BYTES),
    ("committed_peak_bytes", COUNTER_COMMITTED_PEAK_BYTES),
    ("decommits", COUNTER_DECOMMITS),
    ("decommitted_bytes", COUNTER_DECOMMITTED_BYTES),
    ("alloc_calls", COUNTER_ALLOC_CALLS),
    ("alloc_bytes", COUNTER_ALLOC_BYTES),
    ("free_calls", COUNTER_FREE_CALLS),
    ("free_bytes", COUNTER_FREE_BYTES),
    ("live_bytes", COUNTER_LIVE_BYTES),
    ("peak_live_bytes", COUNTER_PEAK_LIVE_BYTES),
    ("hit_small", COUNTER_HIT_SMALL),
    ("small_pages", COUNTER_SMALL_PAGES),
    ("hit_medium_run", COUNTER_HIT_MEDIUM_RUN),
    ("hit_medium_frontier", COUNTER_HIT_MEDIUM_FRONTIER),
    ("hit_huge_run", COUNTER_HIT_HUGE_RUN),
    ("hit_huge_frontier", COUNTER_HIT_HUGE_FRONTIER),
    ("double_free_skips", COUNTER_DOUBLE_FREE_SKIPS),
    ("fill_grow_calls", COUNTER_FILL_GROW_CALLS),
    ("fill_grow_bytes", COUNTER_FILL_GROW_BYTES),
    ("fill_free_calls", COUNTER_FILL_FREE_CALLS),
    ("fill_free_bytes", COUNTER_FILL_FREE_BYTES),
    ("zero_checks", COUNTER_ZERO_CHECKS),
    ("zero_violations", COUNTER_ZERO_VIOLATIONS),
    ("extend_calls", COUNTER_EXTEND_CALLS),
    ("extend_hits", COUNTER_EXTEND_HITS),
];

/// plan-133-C: the memory series after the counters — how many samples are kept, the
/// grow-count stride between kept samples, the grow count that keeps the next one,
/// whether the entry after the kept ones holds an unkept latest grow, then the entries.
const SERIES_COUNT_OFFSET: usize = 240;
const SERIES_STRIDE_OFFSET: usize = 248;
const SERIES_UNTIL_OFFSET: usize = 256;
const SERIES_PENDING_OFFSET: usize = 264;
const SERIES_SAMPLES_OFFSET: usize = 272;
/// Kept samples before the series halves; one more entry holds the latest unkept grow.
const SERIES_CAPACITY: usize = 256;
const SERIES_SAMPLE_SIZE: usize = 32;
/// One sample's words, in report order: `(report name, offset in the entry)`.
const SERIES_FIELDS: [(&str, usize); 4] = [
    ("t_ns", 0),
    ("committed_bytes", 8),
    ("live_bytes", 16),
    ("peak_rss_bytes", 24),
];

/// One slot: `state_ptr`, `kind`, the counters, and the series.
pub(crate) const SLOT_SIZE: usize =
    SERIES_SAMPLES_OFFSET + (SERIES_CAPACITY + 1) * SERIES_SAMPLE_SIZE;
const REGION_SIZE: usize = REGION_HEADER_SIZE + ARENA_DEBUG_SLOTS * SLOT_SIZE;

// The counter offsets are contiguous words ending where the series starts, and the
// series fills the rest of the slot.
const _: () = assert!(
    16 + ARENA_COUNTERS.len() * 8 == SERIES_COUNT_OFFSET
        && COUNTER_GROWS + 8 == SERIES_COUNT_OFFSET
        && SLOT_SIZE == 8496
);

/// The key-suffix data object of a series sample's field (`.t_ns ` …).
fn series_field_suffix_symbol(name: &str) -> String {
    format!("_mfb_rt_debug_arena_series_key_{name}")
}

/// The key-suffix data object of a counter's report line (`.maps ` …).
fn counter_suffix_symbol(name: &str) -> String {
    format!("_mfb_rt_debug_arena_key_{name}")
}

pub(super) struct ArenaFeature;

/// The platform family a module's target names, for the mutex's static initializer.
fn family_of(module: &NirModule) -> PlatformFamily {
    if module.target.starts_with("macos") {
        PlatformFamily::MacOS
    } else if module.target.starts_with("windows") {
        PlatformFamily::Windows
    } else {
        PlatformFamily::Linux
    }
}

/// `dst` = this thread's registry slot (the slot whose `state_ptr` is the arena
/// register), or 0 when the registry is unmapped or the arena is not registered.
///
/// Emitted once per allocator-helper call. Arena-free and call-free: it reads only
/// the registry and clobbers only fresh vregs, so the allocator's live values
/// survive it. `from` is the emitting helper's symbol (labels and relocation).
pub(crate) fn emit_debug_arena_slot(
    from: &str,
    dst: impl Into<abi::Reg>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let dst: abi::Reg = dst.into();
    let global = vregs.next();
    let base = vregs.next();
    let count = vregs.next();
    let index = vregs.next();
    let state = vregs.next();
    let scan = format!("{from}_dbg_slot_scan");
    let missing = format!("{from}_dbg_slot_missing");
    let done = format!("{from}_dbg_slot_done");
    push_symbol_address(from, ARENA_BASE_SYMBOL, &global, instructions, relocations);
    instructions.extend([
        abi::move_immediate(dst, "Integer", "0"),
        abi::load_u64(&base, &global, 0),
        abi::compare_immediate(&base, "0"),
        abi::branch_eq(&done),
        abi::load_u64(&count, &base, REGION_COUNT_OFFSET),
        abi::move_immediate(&index, "Integer", "0"),
        abi::add_immediate(dst, &base, REGION_HEADER_SIZE),
        abi::label(&scan),
        abi::compare_registers(&index, &count),
        abi::branch_ge(&missing),
        abi::load_u64(&state, dst, 0),
        abi::compare_registers(&state, ARENA_STATE_REGISTER),
        abi::branch_eq(&done),
        abi::add_immediate(dst, dst, SLOT_SIZE),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&scan),
        abi::label(&missing),
        abi::move_immediate(dst, "Integer", "0"),
        abi::label(&done),
    ]);
}

/// bug-622: retire the registry slot of the arena state at `state` — a joined worker whose
/// state block is about to be freed — by clearing its `state_ptr`. The slot keeps its kind
/// and counters, so the report still prints the worker; what changes is that the lookup in
/// [`emit_debug_arena_slot`] no longer matches it, so a later worker whose state block
/// reuses the freed address counts into its own slot instead of the dead one's.
///
/// Unlocked, like the lookup: the only thread that ever matched this slot has exited, and
/// registration only appends past `count`. Call-free; clobbers the four `scratch`
/// registers only. `from` is the emitting helper's symbol (labels and relocation).
pub(crate) fn emit_debug_arena_unregister(
    from: &str,
    state: impl Into<abi::Reg>,
    scratch: [impl Into<abi::Reg>; 4],
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let state: abi::Reg = state.into();
    let [base, count, slot, word]: [abi::Reg; 4] = scratch.map(Into::into);
    // A helper may retire arenas from more than one site; the emission point keeps the
    // labels unique.
    let site = instructions.len();
    let scan = format!("{from}_dbg_unregister_{site}_scan");
    let next = format!("{from}_dbg_unregister_{site}_next");
    let done = format!("{from}_dbg_unregister_{site}_done");
    push_symbol_address(from, ARENA_BASE_SYMBOL, base, instructions, relocations);
    instructions.extend([
        abi::load_u64(base, base, 0),
        abi::compare_immediate(base, "0"),
        abi::branch_eq(&done),
        abi::load_u64(count, base, REGION_COUNT_OFFSET),
        abi::add_immediate(slot, base, REGION_HEADER_SIZE),
        abi::label(&scan),
        abi::compare_immediate(count, "0"),
        abi::branch_eq(&done),
        abi::load_u64(word, slot, 0),
        abi::compare_registers(word, state),
        abi::branch_ne(&next),
        abi::store_u64(abi::ZERO, slot, 0),
        abi::branch(&done),
        abi::label(&next),
        abi::add_immediate(slot, slot, SLOT_SIZE),
        abi::subtract_immediate(count, count, 1),
        abi::branch(&scan),
        abi::label(&done),
    ]);
}

/// `[slot + counter] += 1` when `slot` is non-zero. `tag` keeps labels distinct.
pub(crate) fn emit_debug_arena_bump(
    from: &str,
    slot: impl Into<abi::Reg>,
    counter: usize,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let slot: abi::Reg = slot.into();
    let value = vregs.next();
    let skip = format!("{from}_dbg_{tag}_skip");
    instructions.extend([
        abi::compare_immediate(slot, "0"),
        abi::branch_eq(&skip),
        abi::load_u64(&value, slot, counter),
        abi::add_immediate(&value, &value, 1),
        abi::store_u64(&value, slot, counter),
        abi::label(&skip),
    ]);
}

/// `[slot + counter] += amount` when `slot` is non-zero; `amount` is read, not changed.
pub(crate) fn emit_debug_arena_add(
    from: &str,
    slot: impl Into<abi::Reg>,
    counter: usize,
    amount: impl Into<abi::Reg>,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let amount: abi::Reg = amount.into();
    let slot: abi::Reg = slot.into();
    let value = vregs.next();
    let skip = format!("{from}_dbg_{tag}_skip");
    instructions.extend([
        abi::compare_immediate(slot, "0"),
        abi::branch_eq(&skip),
        abi::load_u64(&value, slot, counter),
        abi::add_registers(&value, &value, amount),
        abi::store_u64(&value, slot, counter),
        abi::label(&skip),
    ]);
}

/// A successful allocation of `size` bytes: `live_bytes += size`, and
/// `peak_live_bytes` follows it up. No-op when `slot` is zero.
pub(crate) fn emit_debug_arena_live_add(
    from: &str,
    slot: impl Into<abi::Reg>,
    size: impl Into<abi::Reg>,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let size: abi::Reg = size.into();
    let slot: abi::Reg = slot.into();
    let live = vregs.next();
    let peak = vregs.next();
    let skip = format!("{from}_dbg_{tag}_skip");
    instructions.extend([
        abi::compare_immediate(slot, "0"),
        abi::branch_eq(&skip),
        abi::load_u64(&live, slot, COUNTER_LIVE_BYTES),
        abi::add_registers(&live, &live, size),
        abi::store_u64(&live, slot, COUNTER_LIVE_BYTES),
        abi::load_u64(&peak, slot, COUNTER_PEAK_LIVE_BYTES),
        abi::compare_registers(&live, &peak),
        abi::branch_lo(&skip),
        abi::store_u64(&live, slot, COUNTER_PEAK_LIVE_BYTES),
        abi::label(&skip),
    ]);
}

/// A freed chunk of `size` bytes: `live_bytes -= size`, clamped at zero — a chunk the
/// thread allocated before its arena was registered was never added. No-op when
/// `slot` is zero.
pub(crate) fn emit_debug_arena_live_sub(
    from: &str,
    slot: impl Into<abi::Reg>,
    size: impl Into<abi::Reg>,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let size: abi::Reg = size.into();
    let slot: abi::Reg = slot.into();
    let live = vregs.next();
    let skip = format!("{from}_dbg_{tag}_skip");
    let floor = format!("{from}_dbg_{tag}_floor");
    let store = format!("{from}_dbg_{tag}_store");
    instructions.extend([
        abi::compare_immediate(slot, "0"),
        abi::branch_eq(&skip),
        abi::load_u64(&live, slot, COUNTER_LIVE_BYTES),
        abi::compare_registers(&live, size),
        abi::branch_lo(&floor),
        abi::subtract_registers(&live, &live, size),
        abi::branch(&store),
        abi::label(&floor),
        abi::move_immediate(&live, "Integer", "0"),
        abi::label(&store),
        abi::store_u64(&live, slot, COUNTER_LIVE_BYTES),
        abi::label(&skip),
    ]);
}

/// `pthread_mutex_lock`/`unlock(&_mfb_rt_debug_arena_lock)` through the thread seam
/// (an SRWLOCK on Windows).
fn emit_registry_lock(
    call: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    push_symbol_address(
        DEBUG_ARENA_REGISTER_SYMBOL,
        ARENA_LOCK_SYMBOL,
        abi::c_arg(0),
        instructions,
        relocations,
    );
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: DEBUG_ARENA_REGISTER_SYMBOL,
            platform_imports,
            platform,
            instructions,
            relocations,
        },
        call,
    )
}

fn lower_register(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = DEBUG_ARENA_REGISTER_SYMBOL;
    let mut vregs = Vregs::new();
    let state = vregs.next();
    let kind = vregs.next();
    let global = vregs.next();
    let base = vregs.next();
    let size = vregs.next();
    let count = vregs.next();
    let overflow = vregs.next();
    let slot_size = vregs.next();
    let slot = vregs.next();
    let have_region = "debug_arena_have_region";
    let room = "debug_arena_room";
    let unlock = "debug_arena_unlock";
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&state, abi::c_arg(0)),
        abi::move_register(&kind, abi::c_arg(1)),
    ];
    let mut relocations = Vec::new();
    emit_registry_lock(
        "pthread_mutex_lock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    push_symbol_address(
        symbol,
        ARENA_BASE_SYMBOL,
        &global,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(&base, &global, 0),
        abi::compare_immediate(&base, "0"),
        abi::branch_ne(have_region),
        abi::move_immediate(&size, "Integer", &REGION_SIZE.to_string()),
    ]);
    // First registration maps the region. A failed map leaves the base at 0, so this
    // arena goes unrecorded and the next registration tries again.
    platform.emit_arena_map(size.into(), &mut instructions)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(unlock),
        abi::move_register(&base, abi::return_register()),
        abi::store_u64(&base, &global, 0),
    ]);
    // plan-133-C: the first registration (the main arena, at program entry) records the
    // series' time origin `t0`.
    let t0 = vregs.next();
    emit_debug_monotonic_nanos(
        symbol,
        &t0,
        0,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    )?;
    instructions.push(abi::store_u64(&t0, &base, REGION_T0_OFFSET));
    // plan-183-A: the `MFB_ARENA_CHECK_ZERO` test switch, read with the same one-time
    // work. The frame's clock buffer is free again and serves the Windows call.
    emit_read_check_zero(
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    )?;
    instructions.extend([
        abi::label(have_region),
        abi::load_u64(&count, &base, REGION_COUNT_OFFSET),
        abi::compare_immediate(&count, &ARENA_DEBUG_SLOTS.to_string()),
        abi::branch_lt(room),
        abi::load_u64(&overflow, &base, REGION_OVERFLOW_OFFSET),
        abi::add_immediate(&overflow, &overflow, 1),
        abi::store_u64(&overflow, &base, REGION_OVERFLOW_OFFSET),
        abi::branch(unlock),
        abi::label(room),
        // The region is fresh zeroed mmap and slots are never reused, so the
        // counters of a new slot are already zero.
        abi::move_immediate(&slot_size, "Integer", &SLOT_SIZE.to_string()),
        abi::multiply_registers(&slot, &count, &slot_size),
        abi::add_registers(&slot, &slot, &base),
        abi::add_immediate(&slot, &slot, REGION_HEADER_SIZE),
        abi::store_u64(&state, &slot, 0),
        abi::store_u64(&kind, &slot, SLOT_KIND_OFFSET),
        abi::add_immediate(&count, &count, 1),
        abi::store_u64(&count, &base, REGION_COUNT_OFFSET),
        abi::label(unlock),
    ]);
    emit_registry_lock(
        "pthread_mutex_unlock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::return_());
    // The clock's `timespec` (or the Windows counter words) for `t0` lives in the frame.
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], CLOCK_BUFFER_SIZE);
    Ok(CodeFunction {
        name: "runtime.debug_arena_register".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        instructions,
        relocations,
        stack_slots,
    })
}

/// `_mfb_rt_debug_arena_check_zero = (MFB_ARENA_CHECK_ZERO is set and non-empty)`.
/// POSIX: `getenv`, then the value's first byte. Windows: `GetEnvironmentVariableA`
/// into the frame's 16-byte clock buffer, which returns the value's length (0 when
/// unset or empty) or, for a longer value, the buffer size it needs — non-zero either
/// way.
fn emit_read_check_zero(
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = DEBUG_ARENA_REGISTER_SYMBOL;
    let flag = vregs.next();
    let addr = vregs.next();
    let off = "debug_arena_check_zero_off";
    let store = "debug_arena_check_zero_store";
    push_symbol_address(
        symbol,
        ARENA_CHECK_ZERO_ENV_SYMBOL,
        abi::c_arg(0),
        instructions,
        relocations,
    );
    if platform.family() == PlatformFamily::Windows {
        instructions.extend([
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), 0),
            abi::move_immediate(abi::c_arg(2), "Integer", &CLOCK_BUFFER_SIZE.to_string()),
        ]);
        platform.emit_external_call(
            "GetEnvironmentVariableA",
            symbol,
            platform_imports,
            instructions,
            relocations,
        )?;
        instructions.extend([
            abi::move_register(&flag, abi::c_return(0)),
            abi::compare_immediate(&flag, "0"),
            abi::branch_eq(off),
            abi::move_immediate(&flag, "Integer", "1"),
            abi::branch(store),
        ]);
    } else {
        platform.emit_external_call(
            "getenv",
            symbol,
            platform_imports,
            instructions,
            relocations,
        )?;
        instructions.extend([
            abi::move_register(&flag, abi::c_return(0)),
            abi::compare_immediate(&flag, "0"),
            abi::branch_eq(off),
            abi::load_u8(&flag, &flag, 0),
            abi::compare_immediate(&flag, "0"),
            abi::branch_eq(off),
            abi::move_immediate(&flag, "Integer", "1"),
            abi::branch(store),
        ]);
    }
    instructions.extend([
        abi::label(off),
        abi::move_immediate(&flag, "Integer", "0"),
        abi::label(store),
    ]);
    push_symbol_address(
        symbol,
        ARENA_CHECK_ZERO_SYMBOL,
        &addr,
        instructions,
        relocations,
    );
    instructions.push(abi::store_u64(&flag, &addr, 0));
    Ok(())
}

impl DebugFeature for ArenaFeature {
    fn name(&self) -> &'static str {
        ARENA_SECTION
    }

    fn applies(&self, _module: &NirModule) -> bool {
        true
    }

    fn data_objects(&self, module: &NirModule) -> Vec<CodeDataObject> {
        let text = |symbol: &str, value: &str| string_data_object(symbol, value.to_string());
        let mut objects = vec![
            CodeDataObject {
                symbol: ARENA_BASE_SYMBOL.to_string(),
                kind: "raw".to_string(),
                layout: "mfb.runtime.debug_arena_base.v1 { u64 regionBase }".to_string(),
                align: 8,
                size: 8,
                value: "0000000000000000".to_string(),
            },
            // The same statically initialized mutex shape as the `os::` env lock:
            // valid before any code runs, which the first registration relies on.
            CodeDataObject {
                symbol: ARENA_LOCK_SYMBOL.to_string(),
                kind: "raw".to_string(),
                layout: "mfb.runtime.debug_arena_lock.v1 { u8 mutex[64] }".to_string(),
                align: 8,
                size: crate::codegen::builtins::os::OS_ENV_LOCK_SIZE,
                value: crate::codegen::builtins::os::os_env_lock_init_hex(family_of(module)),
            },
            CodeDataObject {
                symbol: ARENA_CHECK_ZERO_SYMBOL.to_string(),
                kind: "raw".to_string(),
                layout: "mfb.runtime.debug_arena_check_zero.v1 { u64 enabled }".to_string(),
                align: 8,
                size: 8,
                value: "0000000000000000".to_string(),
            },
            // The variable's name, NUL-terminated for `getenv`/`GetEnvironmentVariableA`.
            CodeDataObject {
                symbol: ARENA_CHECK_ZERO_ENV_SYMBOL.to_string(),
                kind: "raw".to_string(),
                layout: "NUL-terminated C string (environment variable name)".to_string(),
                align: 1,
                size: ARENA_CHECK_ZERO_ENV.len() + 1,
                value: ARENA_CHECK_ZERO_ENV
                    .bytes()
                    .chain([0])
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
            },
            key_object(ARENA_COUNT_KEY_SYMBOL, "arena.count"),
            key_object(ARENA_OVERFLOW_KEY_SYMBOL, "arena.registry_overflow"),
            key_object(ARENA_SLICES_KEY_SYMBOL, "arena.slices"),
            key_object(ARENA_SLICE_SIZE_KEY_SYMBOL, "arena.slice_size"),
            text(ARENA_PREFIX_SYMBOL, "arena."),
            text(ARENA_KIND_SUFFIX_SYMBOL, ".kind "),
            text(ARENA_TOKEN_MAIN_SYMBOL, "main\n"),
            text(ARENA_TOKEN_WORKER_SYMBOL, "worker\n"),
            text(ARENA_TOKEN_GRAPHICS_SYMBOL, "graphics\n"),
            text(ARENA_TOKEN_THREAD_BLOCK_SYMBOL, "thread_block\n"),
        ];
        for (name, _) in ARENA_COUNTERS {
            objects.push(text(&counter_suffix_symbol(name), &format!(".{name} ")));
        }
        objects.push(text(ARENA_SERIES_COUNT_KEY_SYMBOL, ".series.count "));
        objects.push(text(ARENA_SERIES_INFIX_SYMBOL, ".series."));
        for (name, _) in SERIES_FIELDS {
            objects.push(text(
                &series_field_suffix_symbol(name),
                &format!(".{name} "),
            ));
        }
        objects
    }

    fn code_functions(
        &self,
        module: &NirModule,
        platform_imports: &HashMap<String, String>,
        platform: &dyn CodegenPlatform,
    ) -> Result<Vec<CodeFunction>, String> {
        Ok(vec![
            lower_register(platform_imports, platform)?,
            lower_sample(platform_imports, platform)?,
            lower_report(
                crate::codegen::runtime::arena_cap::arena_slice_count(module),
                platform_imports,
                platform,
            )?,
        ])
    }

    fn runtime_calls(&self) -> &'static [&'static str] {
        &[]
    }

    fn import_calls(&self) -> &'static [&'static str] {
        &[]
    }

    fn os_imports(
        &self,
        platform: &dyn crate::target::shared::plan::NativePlanPlatform,
        required_by: &str,
    ) -> Vec<crate::target::shared::plan::PlatformImport> {
        // plan-133-C: the series' clock (`t0` at registration, `t_ns` per sample). Its
        // peak-RSS read uses the imports `process` already requests. plan-183-A: the
        // `MFB_ARENA_CHECK_ZERO` read.
        let mut imports = platform.debug_clock_imports(required_by);
        imports.extend(platform.debug_env_imports(required_by));
        imports
    }

    fn lock_helpers(&self) -> &'static [&'static str] {
        &[DEBUG_ARENA_REGISTER_SYMBOL]
    }

    fn emit_entry_start(&self, ctx: &mut DebugEmitCtx<'_>) -> Result<(), String> {
        // Register the main thread's arena first, so it is arena 0.
        ctx.instructions.extend([
            abi::move_register(abi::c_arg(0), ARENA_STATE_REGISTER),
            abi::move_immediate(abi::c_arg(1), "Integer", ARENA_KIND_MAIN),
            abi::branch_link(DEBUG_ARENA_REGISTER_SYMBOL),
        ]);
        ctx.relocations.push(internal_branch(
            ctx.entry_symbol,
            DEBUG_ARENA_REGISTER_SYMBOL,
        ));
        Ok(())
    }

    fn report_symbol(&self) -> Option<&'static str> {
        Some(ARENA_REPORT_SYMBOL)
    }
}
