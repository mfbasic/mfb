//! Native code generation for the internal runtime performance-tracking helpers
//! (plan-67). These are NOT an MFB `perf::` package — there is no language
//! surface; the four helpers are invoked only by compiler-injected calls in a
//! **debug-built, macOS-entry** program (`perf_injection_enabled()` gates every
//! injection site). A `--release` compiler emits none of this, so release output
//! is byte-identical to pre-plan-67 HEAD.
//!
//! - `perf.init` — `emit_arena_map` a private-anon region and store its base in
//!   the writable global `_mfb_rt_perf_state`. Injected at program entry.
//! - `perf.done` — load the base; if non-zero, print the table (just the header
//!   in plan-67-B; rows arrive in C–E) to stderr. Injected in the exit tail.
//! - `perf.start` / `perf.end` — record per-name start / duration (bodies land in
//!   plan-67-C / plan-67-D). Return-only here.
//!
//! **Arena-free invariant (load-bearing):** none of these bodies may touch the
//! arena or call any arena helper. plan-67-F wraps the arena hot path with
//! `perf.start`/`perf.end`; if a perf helper reached the arena the wrapping would
//! recurse perf → arena → perf. The region is system memory via the
//! `emit_arena_map` platform seam, never `_mfb_arena_alloc`.

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

// A generous fixed 16 MiB private-anon reservation for the perf tables. plan-67-D
// chains an additional region on exhaustion (never a silent cap); plan-67-B only
// reserves and bases it. `MAP_ANON` pages arrive zeroed, so the header words the
// C/D tables live in start at 0 with no explicit clear.
const PERF_REGION_SIZE: &str = "16777216";

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
            "perf.done" => {
                // Load the region base; a 0 base (never mapped / release) is inert.
                // Otherwise write the table header to stderr (fd 2). plan-67-C/D
                // extend the middle to iterate the recorded rows. The region is
                // left mapped at exit (plan-67-B decision — the process is ending).
                let state = vregs.next();
                let base = vregs.next();
                let header = vregs.next();
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
                // Write the `mfb.string.v1` header object: length at `[hdr+0]`,
                // bytes at `hdr+8`. Mirrors `emit_write_string_object`.
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
                instructions.push(abi::label(&done));
            }
            "perf.start" | "perf.end" => {
                // Bodies land in plan-67-C (start) / plan-67-D (end). These symbols
                // are not injected until then, so a return-only body is inert while
                // keeping the dispatch total.
            }
            other => {
                return Err(format!(
                    "native perf lowering does not support runtime call '{other}'"
                ));
            }
        }
    }

    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 0);
    Ok((frame, instructions, relocations, stack_slots))
}
