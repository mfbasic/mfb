//! Native code generation for the internal runtime performance-tracking helpers
//! (plan-67). These are NOT an MFB `perf::` package — there is no language
//! surface; the four helpers are invoked only by compiler-injected calls in a
//! **debug-built, macOS-entry** program (`perf_injection_enabled()` gates every
//! injection site). A `--release` compiler emits none of this, so release output
//! is byte-identical to pre-plan-67 HEAD.
//!
//! - `perf.init` — `emit_arena_map` a private-anon region and store its base in
//!   the writable global `_mfb_rt_perf_state`. Injected at program entry.
//! - `perf.start(namePtr)` — upsert table B (name → monotonic start-nanos) with
//!   the current time. Injected right after `perf.init` (whole-program span) and,
//!   from plan-67-F, around each arena region.
//! - `perf.done` — print the table (header + one `name  <startNanos>` row per B
//!   entry in plan-67-C; plan-67-D switches to table A counts, E to full stats) to
//!   stderr. Injected in the exit tail.
//! - `perf.end(namePtr)` — look up the name's open start in table B, record
//!   `now - start` into the flat sample log, and bump the name's count in table A
//!   (plan-67-D).
//!
//! **Name key = pointer identity.** Every injection of a given name loads the one
//! data-object symbol emitted for that name (`plan-67` emits one object per unique
//! name), so identical names arrive as the *identical* runtime pointer. Table B is
//! therefore keyed by an exact `namePtr` equality compare — no hash, no byte
//! compare, no key copy (see plan-67-C Corrections). perf_done prints each name
//! straight from its (process-lifetime-stable) data object.
//!
//! **Arena-free invariant (load-bearing):** none of these bodies may touch the
//! arena or call any arena helper. plan-67-F wraps the arena hot path with
//! `perf.start`/`perf.end`; if a perf helper reached the arena the wrapping would
//! recurse perf → arena → perf. The region is system memory via the
//! `emit_arena_map` platform seam, never `_mfb_arena_alloc`; the clock read rides
//! libc `clock_gettime` inline (never the arena-touching datetime helper).

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

// A generous fixed 16 MiB private-anon reservation for the perf tables. On
// exhaustion plan-67-D's perf_end drops the sample and bumps a visible `overflow`
// counter (never a silent cap). `MAP_ANON` pages arrive zeroed, so the
// header/table words start at 0 with no explicit clear.
const PERF_REGION_SIZE: &str = "16777216";

// `CLOCK_MONOTONIC` on Darwin (Linux uses 1). Matches `datetime.rs`.
const CLOCK_MONOTONIC_DARWIN: &str = "6";

// Region layout (byte offsets from the region base). A 64-byte header, then table
// B (name → open start-nanos), table A (name → sample count), then the flat sample
// log. Every table is keyed by `namePtr` identity.
const PERF_COUNT_B_OFFSET: usize = 0; // u64: occupied B entries
const PERF_COUNT_A_OFFSET: usize = 8; // u64: occupied A entries (plan-67-D)
const PERF_LOG_COUNT_OFFSET: usize = 16; // u64: recorded samples in the log (D)
const PERF_MISMATCH_OFFSET: usize = 24; // u64: perf_end with no open start (D)
const PERF_OVERFLOW_OFFSET: usize = 32; // u64: samples dropped, region full (D)
const PERF_HEADER_SIZE: usize = 64;
// Table B: a flat array of `{ u64 namePtr, i64 startNanos }`, packed [0..count-B),
// looked up by a linear `namePtr`-equality scan.
const PERF_B_TABLE_OFFSET: usize = PERF_HEADER_SIZE;
const PERF_B_ENTRY_SIZE: usize = 16;
const PERF_B_ENTRY_START_OFFSET: usize = 8;
const PERF_B_CAPACITY: usize = 512;
// Table A (plan-67-D): `{ u64 namePtr, u64 count }` per distinct name that has
// recorded at least one duration. `count` tracks the number of log samples for the
// name (so a full-log drop skips both the log append and the count bump, keeping
// them consistent for plan-67-E's stats).
const PERF_A_TABLE_OFFSET: usize = PERF_B_TABLE_OFFSET + PERF_B_CAPACITY * PERF_B_ENTRY_SIZE;
const PERF_A_ENTRY_SIZE: usize = 16;
const PERF_A_ENTRY_COUNT_OFFSET: usize = 8;
const PERF_A_CAPACITY: usize = 512;
// The flat sample log (plan-67-D): `{ u64 namePtr, i64 duration }` appended by
// perf_end, one entry per recorded span. Grows from `PERF_LOG_TABLE_OFFSET` to the
// end of the region; on exhaustion perf_end drops the sample and bumps the
// overflow counter (never a silent cap — plan-67-E's stats scan this log per name).
const PERF_LOG_TABLE_OFFSET: usize = PERF_A_TABLE_OFFSET + PERF_A_CAPACITY * PERF_A_ENTRY_SIZE;
const PERF_LOG_ENTRY_SIZE: usize = 16;
const PERF_LOG_ENTRY_DUR_OFFSET: usize = 8;
// Largest byte offset a log entry may start at and still fit before the 16 MiB
// region end (`16777216 - 16`).
const PERF_LOG_LIMIT: usize = 16_777_216 - PERF_LOG_ENTRY_SIZE;

// Stack locals. perf_start needs a 16-byte `timespec`; perf_done formats each
// numeric column into a scratch window. One frame size covers both (each lowering
// emits exactly one arm).
const PERF_LOCALS_SIZE: usize = 32;
const PERF_TIMESPEC_OFFSET: usize = 0;

pub(super) fn lower_perf_helper(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    // Platform scope (plan-67-B): real perf is macOS-only. The Linux/Windows arms
    // fall through to the shared return-only tail below, so the dispatch is total
    // if a perf symbol is ever referenced off-macOS. In practice the symbols are
    // force-added to the emitted set (and injected) only for macOS, so those arms
    // are never reached.
    if platform.family() == PlatformFamily::MacOS {
        let mut vregs = Vregs::new();
        match call {
            "perf.init" => {
                // mmap the region; on success store its base into the writable
                // global, on failure leave the global 0 so every other perf helper
                // treats perf as inert (a failed profiler must never crash the
                // program). `emit_arena_map` leaves the address in the return
                // register and normalizes failure to a negative sentinel.
                let size = vregs.next();
                let base_slot = vregs.next();
                let done = format!("{symbol}_done");
                instructions.push(abi::move_immediate(&size, "Integer", PERF_REGION_SIZE));
                platform.emit_arena_map(&size, &mut instructions)?;
                instructions.extend([
                    abi::compare_immediate(abi::return_register(), "0"),
                    abi::branch_lt(&done),
                ]);
                push_symbol_address(
                    symbol,
                    PERF_STATE_SYMBOL,
                    &base_slot,
                    &mut instructions,
                    &mut relocations,
                );
                instructions.extend([
                    abi::store_u64(abi::return_register(), &base_slot, 0),
                    abi::label(&done),
                ]);
            }
            "perf.start" => {
                // Upsert table B: linear-scan by `namePtr` equality; overwrite the
                // start on a hit, append a new entry on a miss (dropping the sample
                // if the table is full — plan-67-D adds a visible overflow counter).
                // Then stamp the entry with the current monotonic nanos.
                let name = vregs.next();
                let state = vregs.next();
                let base = vregs.next();
                let count = vregs.next();
                let index = vregs.next();
                let entry = vregs.next();
                let stored = vregs.next();
                let cap = vregs.next();
                let nanos = vregs.next();
                let scan = format!("{symbol}_scan");
                let found = format!("{symbol}_found");
                let not_found = format!("{symbol}_nf");
                let record = format!("{symbol}_rec");
                let done = format!("{symbol}_done");
                let cap_str = PERF_B_CAPACITY.to_string();
                instructions.push(abi::move_register(&name, abi::ARG[0]));
                push_symbol_address(
                    symbol,
                    PERF_STATE_SYMBOL,
                    &state,
                    &mut instructions,
                    &mut relocations,
                );
                instructions.extend([
                    abi::load_u64(&base, &state, 0),
                    abi::compare_immediate(&base, "0"),
                    abi::branch_eq(&done),
                    abi::load_u64(&count, &base, PERF_COUNT_B_OFFSET),
                    abi::add_immediate(&entry, &base, PERF_B_TABLE_OFFSET),
                    abi::move_immediate(&index, "Integer", "0"),
                    abi::label(&scan),
                    abi::compare_registers(&index, &count),
                    abi::branch_ge(&not_found),
                    abi::load_u64(&stored, &entry, 0),
                    abi::compare_registers(&stored, &name),
                    abi::branch_eq(&found),
                    abi::add_immediate(&entry, &entry, PERF_B_ENTRY_SIZE),
                    abi::add_immediate(&index, &index, 1),
                    abi::branch(&scan),
                    abi::label(&found),
                    abi::branch(&record),
                    abi::label(&not_found),
                    // `entry` now points at the first free slot (advanced `count`
                    // times). Append unless the table is full.
                    abi::move_immediate(&cap, "Integer", &cap_str),
                    abi::compare_registers(&count, &cap),
                    abi::branch_ge(&done),
                    abi::store_u64(&name, &entry, 0),
                    abi::add_immediate(&count, &count, 1),
                    abi::store_u64(&count, &base, PERF_COUNT_B_OFFSET),
                    abi::label(&record),
                ]);
                emit_read_monotonic_nanos(
                    &nanos,
                    symbol,
                    platform_imports,
                    platform,
                    &mut instructions,
                    &mut relocations,
                    &mut vregs,
                )?;
                instructions.extend([
                    abi::store_u64(&nanos, &entry, PERF_B_ENTRY_START_OFFSET),
                    abi::label(&done),
                ]);
            }
            "perf.end" => {
                // Look up the name's open start in B; a miss is an end-without-start
                // (bump the visible `mismatch` counter). Otherwise append
                // `now - start` to the flat sample log (bump `overflow` if the region
                // is full — never a silent cap) and bump the name's count in table A.
                let name = vregs.next();
                let state = vregs.next();
                let base = vregs.next();
                let count_b = vregs.next();
                let bindex = vregs.next();
                let bentry = vregs.next();
                let stored = vregs.next();
                let start = vregs.next();
                let now = vregs.next();
                let delta = vregs.next();
                let log_count = vregs.next();
                let off = vregs.next();
                let sixteen = vregs.next();
                let limit = vregs.next();
                let log_entry = vregs.next();
                let count_a = vregs.next();
                let aentry = vregs.next();
                let aindex = vregs.next();
                let astored = vregs.next();
                let acap = vregs.next();
                let scratch = vregs.next();
                let bscan = format!("{symbol}_bscan");
                let mismatch = format!("{symbol}_mm");
                let bfound = format!("{symbol}_bf");
                let ascan = format!("{symbol}_ascan");
                let anew = format!("{symbol}_anew");
                let ainc = format!("{symbol}_ainc");
                let overflow = format!("{symbol}_ov");
                let done = format!("{symbol}_done");
                let acap_str = PERF_A_CAPACITY.to_string();
                let limit_str = PERF_LOG_LIMIT.to_string();
                instructions.push(abi::move_register(&name, abi::ARG[0]));
                push_symbol_address(
                    symbol,
                    PERF_STATE_SYMBOL,
                    &state,
                    &mut instructions,
                    &mut relocations,
                );
                instructions.extend([
                    abi::load_u64(&base, &state, 0),
                    abi::compare_immediate(&base, "0"),
                    abi::branch_eq(&done),
                    // B scan for the open start.
                    abi::load_u64(&count_b, &base, PERF_COUNT_B_OFFSET),
                    abi::add_immediate(&bentry, &base, PERF_B_TABLE_OFFSET),
                    abi::move_immediate(&bindex, "Integer", "0"),
                    abi::label(&bscan),
                    abi::compare_registers(&bindex, &count_b),
                    abi::branch_ge(&mismatch),
                    abi::load_u64(&stored, &bentry, 0),
                    abi::compare_registers(&stored, &name),
                    abi::branch_eq(&bfound),
                    abi::add_immediate(&bentry, &bentry, PERF_B_ENTRY_SIZE),
                    abi::add_immediate(&bindex, &bindex, 1),
                    abi::branch(&bscan),
                    abi::label(&mismatch),
                    abi::load_u64(&scratch, &base, PERF_MISMATCH_OFFSET),
                    abi::add_immediate(&scratch, &scratch, 1),
                    abi::store_u64(&scratch, &base, PERF_MISMATCH_OFFSET),
                    abi::branch(&done),
                    abi::label(&bfound),
                    abi::load_u64(&start, &bentry, PERF_B_ENTRY_START_OFFSET),
                ]);
                emit_read_monotonic_nanos(
                    &now,
                    symbol,
                    platform_imports,
                    platform,
                    &mut instructions,
                    &mut relocations,
                    &mut vregs,
                )?;
                instructions.extend([
                    abi::subtract_registers(&delta, &now, &start),
                    // Append to the flat log unless the region is full.
                    abi::load_u64(&log_count, &base, PERF_LOG_COUNT_OFFSET),
                    abi::move_immediate(&sixteen, "Integer", "16"),
                    abi::multiply_registers(&off, &log_count, &sixteen),
                    abi::add_immediate(&off, &off, PERF_LOG_TABLE_OFFSET),
                    abi::move_immediate(&limit, "Integer", &limit_str),
                    abi::compare_registers(&off, &limit),
                    abi::branch_hi(&overflow),
                    abi::add_registers(&log_entry, &base, &off),
                    abi::store_u64(&name, &log_entry, 0),
                    abi::store_u64(&delta, &log_entry, PERF_LOG_ENTRY_DUR_OFFSET),
                    abi::add_immediate(&log_count, &log_count, 1),
                    abi::store_u64(&log_count, &base, PERF_LOG_COUNT_OFFSET),
                    // Upsert table A: bump the name's count, or append a new entry.
                    abi::load_u64(&count_a, &base, PERF_COUNT_A_OFFSET),
                    abi::add_immediate(&aentry, &base, PERF_A_TABLE_OFFSET),
                    abi::move_immediate(&aindex, "Integer", "0"),
                    abi::label(&ascan),
                    abi::compare_registers(&aindex, &count_a),
                    abi::branch_ge(&anew),
                    abi::load_u64(&astored, &aentry, 0),
                    abi::compare_registers(&astored, &name),
                    abi::branch_eq(&ainc),
                    abi::add_immediate(&aentry, &aentry, PERF_A_ENTRY_SIZE),
                    abi::add_immediate(&aindex, &aindex, 1),
                    abi::branch(&ascan),
                    abi::label(&anew),
                    abi::move_immediate(&acap, "Integer", &acap_str),
                    abi::compare_registers(&count_a, &acap),
                    abi::branch_ge(&done),
                    abi::store_u64(&name, &aentry, 0),
                    abi::move_immediate(&scratch, "Integer", "1"),
                    abi::store_u64(&scratch, &aentry, PERF_A_ENTRY_COUNT_OFFSET),
                    abi::add_immediate(&count_a, &count_a, 1),
                    abi::store_u64(&count_a, &base, PERF_COUNT_A_OFFSET),
                    abi::branch(&done),
                    abi::label(&ainc),
                    abi::load_u64(&scratch, &aentry, PERF_A_ENTRY_COUNT_OFFSET),
                    abi::add_immediate(&scratch, &scratch, 1),
                    abi::store_u64(&scratch, &aentry, PERF_A_ENTRY_COUNT_OFFSET),
                    abi::branch(&done),
                    abi::label(&overflow),
                    abi::load_u64(&scratch, &base, PERF_OVERFLOW_OFFSET),
                    abi::add_immediate(&scratch, &scratch, 1),
                    abi::store_u64(&scratch, &base, PERF_OVERFLOW_OFFSET),
                    abi::label(&done),
                ]);
            }
            "perf.done" => {
                // Load the region base; a 0 base (never mapped / release) is inert.
                // Otherwise write the header, then one `name  <count>` row per table
                // A entry, then the `mismatch`/`overflow` diagnostic rows (only when
                // non-zero). plan-67-E enriches each row with avg/median/min/max/sum
                // over the flat sample log. The region is left mapped at exit
                // (plan-67-B decision — the process is ending).
                let state = vregs.next();
                let base = vregs.next();
                let header = vregs.next();
                let count = vregs.next();
                let index = vregs.next();
                let entry = vregs.next();
                let name = vregs.next();
                let value = vregs.next();
                let arow = format!("{symbol}_arow");
                let extras = format!("{symbol}_extras");
                let done = format!("{symbol}_done");
                push_symbol_address(
                    symbol,
                    PERF_STATE_SYMBOL,
                    &state,
                    &mut instructions,
                    &mut relocations,
                );
                instructions.extend([
                    abi::load_u64(&base, &state, 0),
                    abi::compare_immediate(&base, "0"),
                    abi::branch_eq(&done),
                ]);
                // Header (mirrors `emit_write_string_object`: len at [hdr+0], bytes
                // at hdr+8, fd 2).
                push_symbol_address(
                    symbol,
                    PERF_HEADER_SYMBOL,
                    &header,
                    &mut instructions,
                    &mut relocations,
                );
                instructions.extend([
                    abi::load_u64(abi::string_length_register(), &header, 0),
                    abi::add_immediate(abi::string_data_register(), &header, 8),
                    abi::move_immediate(abi::return_register(), "Integer", "2"),
                ]);
                platform.emit_write(
                    symbol,
                    platform_imports,
                    &mut instructions,
                    &mut relocations,
                )?;
                // Table A rows: `name  <count>`.
                instructions.extend([
                    abi::load_u64(&count, &base, PERF_COUNT_A_OFFSET),
                    abi::add_immediate(&entry, &base, PERF_A_TABLE_OFFSET),
                    abi::move_immediate(&index, "Integer", "0"),
                    abi::label(&arow),
                    abi::compare_registers(&index, &count),
                    abi::branch_ge(&extras),
                    abi::load_u64(&name, &entry, 0),
                ]);
                emit_write_name(
                    &name,
                    symbol,
                    platform_imports,
                    platform,
                    &mut instructions,
                    &mut relocations,
                )?;
                instructions.push(abi::load_u64(&value, &entry, PERF_A_ENTRY_COUNT_OFFSET));
                emit_write_i64_line(
                    &value,
                    "a",
                    symbol,
                    platform_imports,
                    platform,
                    &mut instructions,
                    &mut relocations,
                    &mut vregs,
                )?;
                instructions.extend([
                    abi::add_immediate(&entry, &entry, PERF_A_ENTRY_SIZE),
                    abi::add_immediate(&index, &index, 1),
                    abi::branch(&arow),
                    abi::label(&extras),
                ]);
                // Diagnostic counter rows (printed only when non-zero).
                emit_counter_row(
                    PERF_NAME_MISMATCH_SYMBOL,
                    PERF_MISMATCH_OFFSET,
                    "mm",
                    &base,
                    symbol,
                    platform_imports,
                    platform,
                    &mut instructions,
                    &mut relocations,
                    &mut vregs,
                )?;
                emit_counter_row(
                    PERF_NAME_OVERFLOW_SYMBOL,
                    PERF_OVERFLOW_OFFSET,
                    "ov",
                    &base,
                    symbol,
                    platform_imports,
                    platform,
                    &mut instructions,
                    &mut relocations,
                    &mut vregs,
                )?;
                instructions.push(abi::label(&done));
            }
            other => {
                return Err(format!(
                    "native perf lowering does not support runtime call '{other}'"
                ));
            }
        }
    }

    instructions.push(abi::return_());
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], PERF_LOCALS_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

/// Read `CLOCK_MONOTONIC` into `dst` (i64 nanoseconds), inline, arena-free —
/// replicating `datetime.rs`'s macOS monotonic sequence rather than `bl`-ing the
/// datetime helper (which may not be emitted, and whose call path is not
/// guaranteed arena-free). Uses the `PERF_TIMESPEC_OFFSET` stack buffer and
/// clobbers `ARG[0]`/`ARG[1]` + x0–x17 via the libc call; the caller keeps its
/// live state in vregs (spilled across the call). Reused by perf_end (plan-67-D).
fn emit_read_monotonic_nanos(
    dst: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let sec = vregs.next();
    let nsec = vregs.next();
    let billion = vregs.next();
    instructions.push(abi::move_immediate(
        abi::ARG[0],
        "Integer",
        CLOCK_MONOTONIC_DARWIN,
    ));
    instructions.push(abi::add_immediate(
        abi::ARG[1],
        abi::stack_pointer(),
        PERF_TIMESPEC_OFFSET,
    ));
    platform.emit_libc_call(
        "clock_gettime",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64(&sec, abi::stack_pointer(), PERF_TIMESPEC_OFFSET),
        abi::load_u64(&nsec, abi::stack_pointer(), PERF_TIMESPEC_OFFSET + 8),
        abi::move_immediate(&billion, "Integer", "1000000000"),
        abi::multiply_registers(&sec, &sec, &billion),
        abi::add_registers(dst, &sec, &nsec),
    ]);
    Ok(())
}

/// Append instructions that format ` <value>\n` (leading space, signed decimal,
/// trailing newline) into the stack scratch window and write it to stderr (fd 2).
/// Reused for every numeric column in plan-67-C/D/E. `value` is consumed via a
/// copy, so the caller's register is preserved. `tag` disambiguates the internal
/// labels when a caller formats more than one column.
fn emit_write_i64_line(
    value: &str,
    tag: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let val = vregs.next();
    let cursor = vregs.next();
    let ten = vregs.next();
    let quotient = vregs.next();
    let digit = vregs.next();
    let endp = vregs.next();
    let dloop = format!("{symbol}_{tag}_dloop");
    instructions.extend([
        abi::move_register(&val, value),
        // cursor starts one past the scratch window; digits fill right-to-left.
        abi::add_immediate(&cursor, abi::stack_pointer(), PERF_LOCALS_SIZE),
        // trailing newline
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::move_immediate(&digit, "Integer", "10"),
        abi::store_u8(&digit, &cursor, 0),
        abi::move_immediate(&ten, "Integer", "10"),
        abi::label(&dloop),
        abi::unsigned_divide_registers(&quotient, &val, &ten),
        abi::multiply_subtract_registers(&digit, &quotient, &ten, &val),
        abi::add_immediate(&digit, &digit, 48),
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::store_u8(&digit, &cursor, 0),
        abi::move_register(&val, &quotient),
        abi::compare_immediate(&val, "0"),
        abi::branch_ne(&dloop),
        // leading space
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::move_immediate(&digit, "Integer", "32"),
        abi::store_u8(&digit, &cursor, 0),
        // write [cursor, sp+PERF_LOCALS_SIZE) to stderr
        abi::add_immediate(&endp, abi::stack_pointer(), PERF_LOCALS_SIZE),
        abi::subtract_registers(abi::string_length_register(), &endp, &cursor),
        abi::move_register(abi::string_data_register(), &cursor),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(symbol, platform_imports, instructions, relocations)?;
    Ok(())
}

/// Write the `mfb.string.v1` name object pointed to by `name` (len at `[name+0]`,
/// bytes at `name+8`) to stderr (fd 2).
fn emit_write_name(
    name: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.extend([
        abi::load_u64(abi::string_length_register(), name, 0),
        abi::add_immediate(abi::string_data_register(), name, 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(symbol, platform_imports, instructions, relocations)
}

/// Emit a diagnostic `name  <counter>` row for the header counter at
/// `counter_offset`, but only when it is non-zero (so a clean run prints no
/// diagnostic rows). `base` holds the region base; `tag` disambiguates labels.
#[allow(clippy::too_many_arguments)]
fn emit_counter_row(
    name_symbol: &str,
    counter_offset: usize,
    tag: &str,
    base: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let value = vregs.next();
    let nameptr = vregs.next();
    let skip = format!("{symbol}_{tag}_skip");
    instructions.extend([
        abi::load_u64(&value, base, counter_offset),
        abi::compare_immediate(&value, "0"),
        abi::branch_eq(&skip),
    ]);
    push_symbol_address(symbol, name_symbol, &nameptr, instructions, relocations);
    emit_write_name(
        &nameptr,
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    emit_write_i64_line(
        &value,
        tag,
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
        vregs,
    )?;
    instructions.push(abi::label(&skip));
    Ok(())
}
