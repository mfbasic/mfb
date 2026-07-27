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
//! - `perf.end(namePtr)` — record a duration (plan-67-D). Return-only here.
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

// A generous fixed 16 MiB private-anon reservation for the perf tables. plan-67-D
// chains an additional region on exhaustion (never a silent cap). `MAP_ANON` pages
// arrive zeroed, so the header/table words start at 0 with no explicit clear.
const PERF_REGION_SIZE: &str = "16777216";

// `CLOCK_MONOTONIC` on Darwin (Linux uses 1). Matches `datetime.rs`.
const CLOCK_MONOTONIC_DARWIN: &str = "6";

// Region layout (byte offsets from the region base). The header reserves room for
// plan-67-D's fields (count-A, bump cursor/end, mismatch counter) so the tables
// below never shift when D lands.
const PERF_COUNT_B_OFFSET: usize = 0; // u64: number of occupied B entries
const PERF_HEADER_SIZE: usize = 64;
// Table B: a flat array of `{ u64 namePtr, i64 startNanos }`, packed [0..count-B),
// looked up by a linear `namePtr`-equality scan.
const PERF_B_TABLE_OFFSET: usize = PERF_HEADER_SIZE;
const PERF_B_ENTRY_SIZE: usize = 16;
const PERF_B_ENTRY_START_OFFSET: usize = 8;
const PERF_B_CAPACITY: usize = 512;

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
            "perf.done" => {
                // Load the region base; a 0 base (never mapped / release) is inert.
                // Otherwise write the header, then one `name  <startNanos>` row per
                // B entry. plan-67-D switches this to table A + counts. The region
                // is left mapped at exit (plan-67-B decision — the process is
                // ending).
                let state = vregs.next();
                let base = vregs.next();
                let header = vregs.next();
                let count = vregs.next();
                let index = vregs.next();
                let entry = vregs.next();
                let name = vregs.next();
                let start = vregs.next();
                let rowloop = format!("{symbol}_row");
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
                // Rows.
                instructions.extend([
                    abi::load_u64(&count, &base, PERF_COUNT_B_OFFSET),
                    abi::add_immediate(&entry, &base, PERF_B_TABLE_OFFSET),
                    abi::move_immediate(&index, "Integer", "0"),
                    abi::label(&rowloop),
                    abi::compare_registers(&index, &count),
                    abi::branch_ge(&done),
                    // Write the name straight from its data object.
                    abi::load_u64(&name, &entry, 0),
                    abi::load_u64(abi::string_length_register(), &name, 0),
                    abi::add_immediate(abi::string_data_register(), &name, 8),
                    abi::move_immediate(abi::return_register(), "Integer", "2"),
                ]);
                platform.emit_write(
                    symbol,
                    platform_imports,
                    &mut instructions,
                    &mut relocations,
                )?;
                instructions.push(abi::load_u64(&start, &entry, PERF_B_ENTRY_START_OFFSET));
                emit_write_i64_line(
                    &start,
                    "b",
                    symbol,
                    platform_imports,
                    platform,
                    &mut instructions,
                    &mut relocations,
                    &mut vregs,
                )?;
                instructions.extend([
                    abi::add_immediate(&entry, &entry, PERF_B_ENTRY_SIZE),
                    abi::add_immediate(&index, &index, 1),
                    abi::branch(&rowloop),
                    abi::label(&done),
                ]);
            }
            "perf.end" => {
                // Body lands in plan-67-D. Not injected until then, so a return-only
                // body is inert while keeping the dispatch total.
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
