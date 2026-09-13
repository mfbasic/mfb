//! The `arena` report section (plan-130-C): a process-global registry of every arena
//! state — the main thread's, each `thread::start` worker's, and the canvas graphics
//! thread's — with a set of allocator event counters per arena.
//!
//! The registry is a region mapped on first registration through the platform's
//! `emit_arena_map` seam (system memory, never an arena), holding a
//! `{count, overflow}` header and [`ARENA_DEBUG_SLOTS`] fixed-size slots
//! `{state_ptr, kind, counters[18]}`; its base lives in a writable global. Registration
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

use super::write::{
    emit_debug_key_value, emit_prepend_decimal, emit_prepend_object, emit_write_window, key_object,
    DEBUG_LINE_BUFFER_SIZE,
};
use super::{DebugEmitCtx, DebugFeature};
use crate::codegen::engine::builder::{internal_branch, EmitCtx};
use crate::codegen::engine::types::{
    CodeDataObject, CodeFunction, CodeInstruction, CodeRelocation, CodegenPlatform, PlatformFamily,
};
use crate::codegen::engine::util::{finalize_vreg_body_with_locals, finalize_vreg_helper, Vregs};
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

/// Registry capacity. A program that registers more arenas than this loses their
/// slots (and so their counts); `arena.registry_overflow` reports how many.
const ARENA_DEBUG_SLOTS: usize = 1024;
const REGION_COUNT_OFFSET: usize = 0;
const REGION_OVERFLOW_OFFSET: usize = 8;
const REGION_HEADER_SIZE: usize = 16;
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

/// Every counter, in slot and report order: `(report name, slot offset)`.
const ARENA_COUNTERS: [(&str, usize); 18] = [
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
];

/// One slot: `state_ptr`, `kind`, and the counters.
const SLOT_SIZE: usize = 16 + ARENA_COUNTERS.len() * 8;
const REGION_SIZE: usize = REGION_HEADER_SIZE + ARENA_DEBUG_SLOTS * SLOT_SIZE;

// The counter offsets are contiguous words ending exactly at the slot's end.
const _: () = assert!(COUNTER_DOUBLE_FREE_SKIPS + 8 == SLOT_SIZE && SLOT_SIZE == 160);

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
    Ok(finalize_vreg_helper(
        "runtime.debug_arena_register",
        symbol,
        "Nothing",
        instructions,
        relocations,
    ))
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
            lower_report(platform_imports, platform)?,
        ])
    }

    fn runtime_calls(&self) -> &'static [&'static str] {
        &[]
    }

    fn import_calls(&self) -> &'static [&'static str] {
        &[]
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
