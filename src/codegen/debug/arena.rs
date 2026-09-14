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

const ARENA_REPORT_SYMBOL: &str = "_mfb_debug_report_arena";
const ARENA_BASE_SYMBOL: &str = "_mfb_rt_debug_arena_base";
const ARENA_LOCK_SYMBOL: &str = "_mfb_rt_debug_arena_lock";
const ARENA_COUNT_KEY_SYMBOL: &str = "_mfb_rt_debug_arena_key_count";
const ARENA_OVERFLOW_KEY_SYMBOL: &str = "_mfb_rt_debug_arena_key_overflow";
const ARENA_PREFIX_SYMBOL: &str = "_mfb_rt_debug_arena_key_prefix";
const ARENA_KIND_SUFFIX_SYMBOL: &str = "_mfb_rt_debug_arena_key_kind";
const ARENA_TOKEN_MAIN_SYMBOL: &str = "_mfb_rt_debug_arena_token_main";
const ARENA_TOKEN_WORKER_SYMBOL: &str = "_mfb_rt_debug_arena_token_worker";
const ARENA_TOKEN_GRAPHICS_SYMBOL: &str = "_mfb_rt_debug_arena_token_graphics";
const ARENA_SERIES_INFIX_SYMBOL: &str = "_mfb_rt_debug_arena_series_infix";
const ARENA_SERIES_COUNT_KEY_SYMBOL: &str = "_mfb_rt_debug_arena_series_key_count";

/// Registry capacity. A program that registers more arenas than this loses their
/// slots (and so their counts); `arena.registry_overflow` reports how many.
const ARENA_DEBUG_SLOTS: usize = 1024;
const REGION_COUNT_OFFSET: usize = 0;
const REGION_OVERFLOW_OFFSET: usize = 8;
/// The monotonic clock (nanoseconds) when the main arena registered: every series
/// sample's `t_ns` is measured from it.
const REGION_T0_OFFSET: usize = 16;
const REGION_HEADER_SIZE: usize = 24;
const SLOT_KIND_OFFSET: usize = 8;

/// Counter words of a slot, after `state_ptr` (+0) and `kind` (+8).
pub(crate) const COUNTER_MAPS: usize = 16;
pub(crate) const COUNTER_MAPPED_BYTES: usize = 24;
pub(crate) const COUNTER_UNMAPS: usize = 32;
pub(crate) const COUNTER_UNMAPPED_BYTES: usize = 40;
pub(crate) const COUNTER_ALLOC_CALLS: usize = 48;
pub(crate) const COUNTER_ALLOC_BYTES: usize = 56;
pub(crate) const COUNTER_FREE_CALLS: usize = 64;
pub(crate) const COUNTER_FREE_BYTES: usize = 72;
pub(crate) const COUNTER_LIVE_BYTES: usize = 80;
pub(crate) const COUNTER_PEAK_LIVE_BYTES: usize = 88;
pub(crate) const COUNTER_HIT_QUICK_BIN: usize = 96;
pub(crate) const COUNTER_HIT_CARVE: usize = 104;
pub(crate) const COUNTER_HIT_LARGE_BIN: usize = 112;
pub(crate) const COUNTER_HIT_WALK: usize = 120;
pub(crate) const COUNTER_GROW: usize = 128;
pub(crate) const COUNTER_FLUSHES: usize = 136;
pub(crate) const COUNTER_INSERT_FREE_CALLS: usize = 144;
pub(crate) const COUNTER_DOUBLE_FREE_SKIPS: usize = 152;
/// plan-133-B: entropy-fill calls and bytes, on the grow path (a fresh block's usable
/// region) and the free path (a freed chunk's payload past its 16-byte node).
pub(crate) const COUNTER_FILL_GROW_CALLS: usize = 160;
pub(crate) const COUNTER_FILL_GROW_BYTES: usize = 168;
pub(crate) const COUNTER_FILL_FREE_CALLS: usize = 176;
pub(crate) const COUNTER_FILL_FREE_BYTES: usize = 184;

/// Every counter, in slot and report order: `(report name, slot offset)`.
const ARENA_COUNTERS: [(&str, usize); 22] = [
    ("maps", COUNTER_MAPS),
    ("mapped_bytes", COUNTER_MAPPED_BYTES),
    ("unmaps", COUNTER_UNMAPS),
    ("unmapped_bytes", COUNTER_UNMAPPED_BYTES),
    ("alloc_calls", COUNTER_ALLOC_CALLS),
    ("alloc_bytes", COUNTER_ALLOC_BYTES),
    ("free_calls", COUNTER_FREE_CALLS),
    ("free_bytes", COUNTER_FREE_BYTES),
    ("live_bytes", COUNTER_LIVE_BYTES),
    ("peak_live_bytes", COUNTER_PEAK_LIVE_BYTES),
    ("hit_quick_bin", COUNTER_HIT_QUICK_BIN),
    ("hit_carve", COUNTER_HIT_CARVE),
    ("hit_large_bin", COUNTER_HIT_LARGE_BIN),
    ("hit_walk", COUNTER_HIT_WALK),
    ("grow", COUNTER_GROW),
    ("flushes", COUNTER_FLUSHES),
    ("insert_free_calls", COUNTER_INSERT_FREE_CALLS),
    ("double_free_skips", COUNTER_DOUBLE_FREE_SKIPS),
    ("fill_grow_calls", COUNTER_FILL_GROW_CALLS),
    ("fill_grow_bytes", COUNTER_FILL_GROW_BYTES),
    ("fill_free_calls", COUNTER_FILL_FREE_CALLS),
    ("fill_free_bytes", COUNTER_FILL_FREE_BYTES),
];

/// plan-133-C: the memory series after the counters — how many samples are kept, the
/// grow-count stride between kept samples, the grow count that keeps the next one,
/// whether the entry after the kept ones holds an unkept latest grow, then the entries.
const SERIES_COUNT_OFFSET: usize = 192;
const SERIES_STRIDE_OFFSET: usize = 200;
const SERIES_UNTIL_OFFSET: usize = 208;
const SERIES_PENDING_OFFSET: usize = 216;
const SERIES_SAMPLES_OFFSET: usize = 224;
/// Kept samples before the series halves; one more entry holds the latest unkept grow.
const SERIES_CAPACITY: usize = 256;
const SERIES_SAMPLE_SIZE: usize = 32;
/// One sample's words, in report order: `(report name, offset in the entry)`.
const SERIES_FIELDS: [(&str, usize); 4] = [
    ("t_ns", 0),
    ("mapped_bytes", 8),
    ("live_bytes", 16),
    ("peak_rss_bytes", 24),
];

/// One slot: `state_ptr`, `kind`, the counters, and the series.
const SLOT_SIZE: usize = SERIES_SAMPLES_OFFSET + (SERIES_CAPACITY + 1) * SERIES_SAMPLE_SIZE;
const REGION_SIZE: usize = REGION_HEADER_SIZE + ARENA_DEBUG_SLOTS * SLOT_SIZE;

// The counter offsets are contiguous words ending where the series starts, and the
// series fills the rest of the slot.
const _: () = assert!(
    16 + ARENA_COUNTERS.len() * 8 == SERIES_COUNT_OFFSET
        && COUNTER_FILL_FREE_BYTES + 8 == SERIES_COUNT_OFFSET
        && SLOT_SIZE == 8448
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
    dst: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
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

/// `[slot + counter] += 1` when `slot` is non-zero. `tag` keeps labels distinct.
pub(crate) fn emit_debug_arena_bump(
    from: &str,
    slot: &str,
    counter: usize,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
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
    slot: &str,
    counter: usize,
    amount: &str,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
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
    slot: &str,
    size: &str,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
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
    slot: &str,
    size: &str,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
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
    platform.emit_arena_map(&size, &mut instructions)?;
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
    instructions.extend([
        abi::store_u64(&t0, &base, REGION_T0_OFFSET),
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

/// `arena.<index><suffix><value>\n` in the helper's window: right to left, the value
/// digits, the suffix object (`.maps ` …), the index digits, `arena.`.
#[allow(clippy::too_many_arguments)]
fn emit_slot_line(
    symbol: &str,
    index: &str,
    suffix_symbol: &str,
    value: &str,
    tag: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let cursor = vregs.next();
    let newline = vregs.next();
    let suffix = vregs.next();
    let prefix = vregs.next();
    instructions.extend([
        abi::add_immediate(&cursor, abi::stack_pointer(), DEBUG_LINE_BUFFER_SIZE),
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::move_immediate(&newline, "Integer", "10"),
        abi::store_u8(&newline, &cursor, 0),
    ]);
    emit_prepend_decimal(
        value,
        &cursor,
        &format!("debug_arena_{tag}_val"),
        instructions,
        vregs,
    );
    push_symbol_address(symbol, suffix_symbol, &suffix, instructions, relocations);
    emit_prepend_object(
        &suffix,
        &cursor,
        &format!("debug_arena_{tag}_sfx"),
        instructions,
        vregs,
    );
    emit_prepend_decimal(
        index,
        &cursor,
        &format!("debug_arena_{tag}_idx"),
        instructions,
        vregs,
    );
    push_symbol_address(
        symbol,
        ARENA_PREFIX_SYMBOL,
        &prefix,
        instructions,
        relocations,
    );
    emit_prepend_object(
        &prefix,
        &cursor,
        &format!("debug_arena_{tag}_pfx"),
        instructions,
        vregs,
    );
    emit_write_window(
        symbol,
        &cursor,
        platform_imports,
        platform,
        instructions,
        relocations,
        vregs,
    )
}

/// `arena.<index>.series.<sample><suffix><value>\n` (plan-133-C), assembled right to left
/// like [`emit_slot_line`]: the value digits, the field suffix (`.t_ns ` …), the sample
/// digits, `.series.`, the arena index digits, `arena.`.
#[allow(clippy::too_many_arguments)]
fn emit_series_line(
    symbol: &str,
    index: &str,
    sample: &str,
    suffix_symbol: &str,
    value: &str,
    tag: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let cursor = vregs.next();
    let newline = vregs.next();
    let suffix = vregs.next();
    let infix = vregs.next();
    let prefix = vregs.next();
    instructions.extend([
        abi::add_immediate(&cursor, abi::stack_pointer(), DEBUG_LINE_BUFFER_SIZE),
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::move_immediate(&newline, "Integer", "10"),
        abi::store_u8(&newline, &cursor, 0),
    ]);
    emit_prepend_decimal(
        value,
        &cursor,
        &format!("debug_arena_series_{tag}_val"),
        instructions,
        vregs,
    );
    push_symbol_address(symbol, suffix_symbol, &suffix, instructions, relocations);
    emit_prepend_object(
        &suffix,
        &cursor,
        &format!("debug_arena_series_{tag}_sfx"),
        instructions,
        vregs,
    );
    emit_prepend_decimal(
        sample,
        &cursor,
        &format!("debug_arena_series_{tag}_smp"),
        instructions,
        vregs,
    );
    push_symbol_address(
        symbol,
        ARENA_SERIES_INFIX_SYMBOL,
        &infix,
        instructions,
        relocations,
    );
    emit_prepend_object(
        &infix,
        &cursor,
        &format!("debug_arena_series_{tag}_inf"),
        instructions,
        vregs,
    );
    emit_prepend_decimal(
        index,
        &cursor,
        &format!("debug_arena_series_{tag}_idx"),
        instructions,
        vregs,
    );
    push_symbol_address(
        symbol,
        ARENA_PREFIX_SYMBOL,
        &prefix,
        instructions,
        relocations,
    );
    emit_prepend_object(
        &prefix,
        &cursor,
        &format!("debug_arena_series_{tag}_pfx"),
        instructions,
        vregs,
    );
    emit_write_window(
        symbol,
        &cursor,
        platform_imports,
        platform,
        instructions,
        relocations,
        vregs,
    )
}

fn lower_report(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = ARENA_REPORT_SYMBOL;
    let mut vregs = Vregs::new();
    let global = vregs.next();
    let base = vregs.next();
    let count = vregs.next();
    let overflow = vregs.next();
    let index = vregs.next();
    let slot_size = vregs.next();
    let slot = vregs.next();
    let kind = vregs.next();
    let token = vregs.next();
    let suffix = vregs.next();
    let prefix = vregs.next();
    let cursor = vregs.next();
    let totals = "debug_arena_totals";
    let rows = "debug_arena_rows";
    let worker = "debug_arena_kind_worker";
    let graphics = "debug_arena_kind_graphics";
    let line = "debug_arena_line";
    let done = "debug_arena_done";
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    push_symbol_address(
        symbol,
        ARENA_BASE_SYMBOL,
        &global,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::move_immediate(&count, "Integer", "0"),
        abi::move_immediate(&overflow, "Integer", "0"),
        abi::load_u64(&base, &global, 0),
        abi::compare_immediate(&base, "0"),
        abi::branch_eq(totals),
        abi::load_u64(&count, &base, REGION_COUNT_OFFSET),
        abi::load_u64(&overflow, &base, REGION_OVERFLOW_OFFSET),
        abi::label(totals),
    ]);
    for (key, value, tag) in [
        (ARENA_COUNT_KEY_SYMBOL, &count, "count"),
        (ARENA_OVERFLOW_KEY_SYMBOL, &overflow, "overflow"),
    ] {
        emit_debug_key_value(
            symbol,
            key,
            value,
            tag,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        )?;
    }
    // Per registered arena: `arena.<n>.kind <token>`, then each counter.
    instructions.extend([
        abi::move_immediate(&index, "Integer", "0"),
        abi::move_immediate(&slot_size, "Integer", &SLOT_SIZE.to_string()),
        abi::label(rows),
        abi::compare_registers(&index, &count),
        abi::branch_ge(done),
        abi::multiply_registers(&slot, &index, &slot_size),
        abi::add_registers(&slot, &slot, &base),
        abi::add_immediate(&slot, &slot, REGION_HEADER_SIZE),
        abi::load_u64(&kind, &slot, SLOT_KIND_OFFSET),
        abi::compare_immediate(&kind, ARENA_KIND_WORKER),
        abi::branch_eq(worker),
        abi::compare_immediate(&kind, ARENA_KIND_GRAPHICS),
        abi::branch_eq(graphics),
    ]);
    push_symbol_address(
        symbol,
        ARENA_TOKEN_MAIN_SYMBOL,
        &token,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(line), abi::label(worker)]);
    push_symbol_address(
        symbol,
        ARENA_TOKEN_WORKER_SYMBOL,
        &token,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(line), abi::label(graphics)]);
    push_symbol_address(
        symbol,
        ARENA_TOKEN_GRAPHICS_SYMBOL,
        &token,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(line),
        abi::add_immediate(&cursor, abi::stack_pointer(), DEBUG_LINE_BUFFER_SIZE),
    ]);
    // Right to left: `<token>\n`, `.kind `, `<n>`, `arena.`.
    emit_prepend_object(
        &token,
        &cursor,
        "debug_arena_tok",
        &mut instructions,
        &mut vregs,
    );
    push_symbol_address(
        symbol,
        ARENA_KIND_SUFFIX_SYMBOL,
        &suffix,
        &mut instructions,
        &mut relocations,
    );
    emit_prepend_object(
        &suffix,
        &cursor,
        "debug_arena_sfx",
        &mut instructions,
        &mut vregs,
    );
    emit_prepend_decimal(
        &index,
        &cursor,
        "debug_arena_idx",
        &mut instructions,
        &mut vregs,
    );
    push_symbol_address(
        symbol,
        ARENA_PREFIX_SYMBOL,
        &prefix,
        &mut instructions,
        &mut relocations,
    );
    emit_prepend_object(
        &prefix,
        &cursor,
        "debug_arena_pfx",
        &mut instructions,
        &mut vregs,
    );
    emit_write_window(
        symbol,
        &cursor,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    )?;
    for (name, offset) in ARENA_COUNTERS {
        let value = vregs.next();
        instructions.push(abi::load_u64(&value, &slot, offset));
        emit_slot_line(
            symbol,
            &index,
            &counter_suffix_symbol(name),
            &value,
            name,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        )?;
    }
    // plan-133-C: the memory series — `arena.<n>.series.count <k>`, then each sample's
    // four values, oldest first. `k` counts the kept samples plus the latest unkept grow.
    let kept = vregs.next();
    let pending = vregs.next();
    let total = vregs.next();
    let sample = vregs.next();
    let width = vregs.next();
    let entry = vregs.next();
    let series_rows = "debug_arena_series_rows";
    let series_done = "debug_arena_series_done";
    instructions.extend([
        abi::load_u64(&kept, &slot, SERIES_COUNT_OFFSET),
        abi::load_u64(&pending, &slot, SERIES_PENDING_OFFSET),
        abi::add_registers(&total, &kept, &pending),
    ]);
    emit_slot_line(
        symbol,
        &index,
        ARENA_SERIES_COUNT_KEY_SYMBOL,
        &total,
        "series_count",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    )?;
    instructions.extend([
        abi::move_immediate(&sample, "Integer", "0"),
        abi::move_immediate(&width, "Integer", &SERIES_SAMPLE_SIZE.to_string()),
        abi::label(series_rows),
        abi::compare_registers(&sample, &total),
        abi::branch_ge(series_done),
        abi::multiply_registers(&entry, &sample, &width),
        abi::add_registers(&entry, &entry, &slot),
        abi::add_immediate(&entry, &entry, SERIES_SAMPLES_OFFSET),
    ]);
    for (name, offset) in SERIES_FIELDS {
        let value = vregs.next();
        instructions.push(abi::load_u64(&value, &entry, offset));
        emit_series_line(
            symbol,
            &index,
            &sample,
            &series_field_suffix_symbol(name),
            &value,
            name,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        )?;
    }
    instructions.extend([
        abi::add_immediate(&sample, &sample, 1),
        abi::branch(series_rows),
        abi::label(series_done),
    ]);
    instructions.extend([
        abi::add_immediate(&index, &index, 1),
        abi::branch(rows),
        abi::label(done),
        abi::return_(),
    ]);
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], DEBUG_LINE_BUFFER_SIZE);
    Ok(CodeFunction {
        name: "runtime.debug_report_arena".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        instructions,
        relocations,
        stack_slots,
    })
}

/// `_mfb_debug_arena_sample(slot)` (plan-133-C): write this grow's sample into the entry
/// after the kept ones, then keep it when the slot's grow count has reached `until`.
/// Keeping the 256th sample halves the series in place — the even entries, then the one
/// just kept — and doubles the stride. An unkept grow sets `pending`, so the report shows
/// the latest grow as the last sample either way.
fn lower_sample(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = DEBUG_ARENA_SAMPLE_SYMBOL;
    let mut vregs = Vregs::new();
    let slot = vregs.next();
    let now = vregs.next();
    let rss = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&slot, abi::c_arg(0)),
        abi::compare_immediate(&slot, "0"),
        abi::branch_eq("debug_sample_done"),
    ];
    let mut relocations = Vec::new();
    emit_debug_monotonic_nanos(
        symbol,
        &now,
        0,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    )?;
    emit_debug_peak_rss(
        symbol,
        &rss,
        CLOCK_BUFFER_SIZE,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    )?;
    let global = vregs.next();
    let base = vregs.next();
    let t0 = vregs.next();
    let count = vregs.next();
    let width = vregs.next();
    let entry = vregs.next();
    let mapped = vregs.next();
    let live = vregs.next();
    let grow = vregs.next();
    let until = vregs.next();
    let stride = vregs.next();
    let flag = vregs.next();
    push_symbol_address(
        symbol,
        ARENA_BASE_SYMBOL,
        &global,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(&base, &global, 0),
        abi::load_u64(&t0, &base, REGION_T0_OFFSET),
        abi::subtract_registers(&now, &now, &t0),
        // entry = slot + SERIES_SAMPLES_OFFSET + count * SERIES_SAMPLE_SIZE
        abi::load_u64(&count, &slot, SERIES_COUNT_OFFSET),
        abi::move_immediate(&width, "Integer", &SERIES_SAMPLE_SIZE.to_string()),
        abi::multiply_registers(&entry, &count, &width),
        abi::add_registers(&entry, &entry, &slot),
        abi::add_immediate(&entry, &entry, SERIES_SAMPLES_OFFSET),
        abi::load_u64(&mapped, &slot, COUNTER_MAPPED_BYTES),
        abi::load_u64(&live, &slot, COUNTER_LIVE_BYTES),
        abi::store_u64(&now, &entry, SERIES_FIELDS[0].1),
        abi::store_u64(&mapped, &entry, SERIES_FIELDS[1].1),
        abi::store_u64(&live, &entry, SERIES_FIELDS[2].1),
        abi::store_u64(&rss, &entry, SERIES_FIELDS[3].1),
        // Keep it once the grow count reaches `until` (0 before the first grow).
        abi::load_u64(&grow, &slot, COUNTER_GROW),
        abi::load_u64(&until, &slot, SERIES_UNTIL_OFFSET),
        abi::compare_registers(&grow, &until),
        abi::branch_lo("debug_sample_pending"),
        abi::load_u64(&stride, &slot, SERIES_STRIDE_OFFSET),
        abi::compare_immediate(&stride, "0"),
        abi::branch_ne("debug_sample_stride"),
        abi::move_immediate(&stride, "Integer", "1"),
        abi::label("debug_sample_stride"),
        abi::add_immediate(&count, &count, 1),
        abi::compare_immediate(&count, &SERIES_CAPACITY.to_string()),
        abi::branch_ne("debug_sample_keep"),
    ]);
    // Halve: entries[k] = entries[2k] for k < 128, then entries[128] = the entry just
    // kept (index 255), so the latest grow survives; 129 kept, stride doubled.
    let half = SERIES_CAPACITY / 2;
    let k = vregs.next();
    let first = vregs.next();
    let double_width = vregs.next();
    let from = vregs.next();
    let to = vregs.next();
    let word = vregs.next();
    instructions.extend([
        abi::add_immediate(&first, &slot, SERIES_SAMPLES_OFFSET),
        abi::move_immediate(
            &double_width,
            "Integer",
            &(2 * SERIES_SAMPLE_SIZE).to_string(),
        ),
        abi::move_immediate(&k, "Integer", "0"),
        abi::label("debug_sample_halve"),
        abi::compare_immediate(&k, &half.to_string()),
        abi::branch_ge("debug_sample_halved"),
        abi::multiply_registers(&from, &k, &double_width),
        abi::add_registers(&from, &from, &first),
        abi::multiply_registers(&to, &k, &width),
        abi::add_registers(&to, &to, &first),
    ]);
    for (_, offset) in SERIES_FIELDS {
        instructions.extend([
            abi::load_u64(&word, &from, offset),
            abi::store_u64(&word, &to, offset),
        ]);
    }
    instructions.extend([
        abi::add_immediate(&k, &k, 1),
        abi::branch("debug_sample_halve"),
        abi::label("debug_sample_halved"),
        abi::add_immediate(&to, &first, half * SERIES_SAMPLE_SIZE),
    ]);
    for (_, offset) in SERIES_FIELDS {
        instructions.extend([
            abi::load_u64(&word, &entry, offset),
            abi::store_u64(&word, &to, offset),
        ]);
    }
    instructions.extend([
        abi::move_immediate(&count, "Integer", &(half + 1).to_string()),
        abi::add_registers(&stride, &stride, &stride),
        abi::label("debug_sample_keep"),
        abi::add_registers(&until, &grow, &stride),
        abi::store_u64(&count, &slot, SERIES_COUNT_OFFSET),
        abi::store_u64(&stride, &slot, SERIES_STRIDE_OFFSET),
        abi::store_u64(&until, &slot, SERIES_UNTIL_OFFSET),
        abi::move_immediate(&flag, "Integer", "0"),
        abi::store_u64(&flag, &slot, SERIES_PENDING_OFFSET),
        abi::branch("debug_sample_done"),
        abi::label("debug_sample_pending"),
        abi::move_immediate(&flag, "Integer", "1"),
        abi::store_u64(&flag, &slot, SERIES_PENDING_OFFSET),
        abi::label("debug_sample_done"),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(
        &mut instructions,
        &[],
        CLOCK_BUFFER_SIZE + PEAK_RSS_BUFFER_SIZE,
    );
    Ok(CodeFunction {
        name: "runtime.debug_arena_sample".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        instructions,
        relocations,
        stack_slots,
    })
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
            key_object(ARENA_COUNT_KEY_SYMBOL, "arena.count"),
            key_object(ARENA_OVERFLOW_KEY_SYMBOL, "arena.registry_overflow"),
            text(ARENA_PREFIX_SYMBOL, "arena."),
            text(ARENA_KIND_SUFFIX_SYMBOL, ".kind "),
            text(ARENA_TOKEN_MAIN_SYMBOL, "main\n"),
            text(ARENA_TOKEN_WORKER_SYMBOL, "worker\n"),
            text(ARENA_TOKEN_GRAPHICS_SYMBOL, "graphics\n"),
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
        _module: &NirModule,
        platform_imports: &HashMap<String, String>,
        platform: &dyn CodegenPlatform,
    ) -> Result<Vec<CodeFunction>, String> {
        Ok(vec![
            lower_register(platform_imports, platform)?,
            lower_sample(platform_imports, platform)?,
            lower_report(platform_imports, platform)?,
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
        // peak-RSS read uses the imports `process` already requests.
        platform.debug_clock_imports(required_by)
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
